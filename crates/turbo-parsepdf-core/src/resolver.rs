//! Lazy indirect-object loader.
//!
//! A [`Resolver`] turns an [`ObjRef`] into a concrete [`Object`] by seeking to
//! the byte offset the [`Xref`] records, parsing the `n g obj … endobj` envelope,
//! and assembling any trailing `stream`. Results are memoized per object number,
//! so a re-referenced object is parsed once. Stream `/Length` is itself resolved
//! (it is commonly an indirect reference); when it is absent or wrong, the body
//! is recovered by scanning for `endstream`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::cos::parse_object;
use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::filter::decode_stream;
use crate::lex::Lexer;
use crate::object::{Dictionary, ObjRef, Object, Stream};
use crate::objstm::ObjStm;
use crate::xref::{Xref, XrefEntry};

fn bad_object(msg: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::InvalidObject, msg)
}

/// A reusable resolver over a buffer and its cross-reference table.
#[derive(Debug)]
pub struct Resolver<'a> {
    data: &'a [u8],
    xref: Xref,
    cache: RefCell<HashMap<u32, Object>>,
    objstms: RefCell<HashMap<u32, Rc<ObjStm>>>,
    #[cfg(feature = "encrypt")]
    decryptor: Option<(crate::crypt::Decryptor, Option<u32>)>,
}

impl<'a> Resolver<'a> {
    /// Build a resolver over `data` using the given cross-reference table.
    pub fn new(data: &'a [u8], xref: Xref) -> Self {
        Resolver {
            data,
            xref,
            cache: RefCell::new(HashMap::new()),
            objstms: RefCell::new(HashMap::new()),
            #[cfg(feature = "encrypt")]
            decryptor: None,
        }
    }

    /// The underlying cross-reference table.
    pub fn xref(&self) -> &Xref {
        &self.xref
    }

    /// Fetch and cache the object for a reference.
    pub fn get(&self, r: ObjRef) -> Result<Object> {
        if let Some(obj) = self.cache.borrow().get(&r.num) {
            return Ok(obj.clone());
        }
        let obj = self.load(r)?;
        self.cache.borrow_mut().insert(r.num, obj.clone());
        Ok(obj)
    }

    /// Resolve a value: follow a reference, or clone a direct object.
    pub fn resolve(&self, obj: &Object) -> Result<Object> {
        match obj.as_reference() {
            Some(r) => self.get(r),
            None => Ok(obj.clone()),
        }
    }

    /// Resolve a value and require it to be a dictionary (or stream dict).
    pub fn resolve_dict(&self, obj: &Object) -> Result<Dictionary> {
        match self.resolve(obj)? {
            Object::Dictionary(d) => Ok(d),
            Object::Stream(s) => Ok(s.dict),
            _ => Err(bad_object("expected dictionary")),
        }
    }

    /// Load an object from the file (uncached).
    fn load(&self, r: ObjRef) -> Result<Object> {
        match self.xref.get(r.num) {
            Some(XrefEntry::InUse { offset, .. }) => self.parse_indirect(offset, r.num),
            Some(XrefEntry::Compressed { stream, index }) => self.load_compressed(stream, index),
            _ => Err(TurboParsePdfError::new(
                ErrorCode::ObjectNotFound,
                "no such object",
            )),
        }
    }

    /// Parse `n g obj <value> [stream …] endobj` at `offset`.
    fn parse_indirect(&self, offset: usize, expected: u32) -> Result<Object> {
        let mut lx = Lexer::at(self.data, offset);
        let (num, _generation) = read_header(&mut lx)?;
        if num != expected {
            return Err(bad_object("object number mismatch"));
        }
        let value = parse_object(&mut lx)?;
        let length = value.as_dict().and_then(|d| self.declared_length(d));
        let obj = assemble_stream(self.data, &mut lx, value, length)?;
        #[cfg(feature = "encrypt")]
        let obj = self.finish_object(num, _generation, obj);
        Ok(obj)
    }

    /// Decrypt a freshly loaded object when the document is encrypted (the
    /// `/Encrypt` object itself is exempt).
    #[cfg(feature = "encrypt")]
    fn finish_object(&self, num: u32, generation: u16, obj: Object) -> Object {
        match &self.decryptor {
            Some((dec, exempt)) if Some(num) != *exempt => dec.decrypt_object(num, generation, obj),
            _ => obj,
        }
    }

    /// Install a decryptor (and the exempt `/Encrypt` object number).
    #[cfg(feature = "encrypt")]
    pub fn set_decryptor(&mut self, decryptor: crate::crypt::Decryptor, exempt: Option<u32>) {
        self.decryptor = Some((decryptor, exempt));
    }

    /// Load a type-2 (compressed) object from its object stream.
    fn load_compressed(&self, stream: u32, index: u32) -> Result<Object> {
        let objstm = self.get_objstm(stream)?;
        objstm.get(index as usize)
    }

    /// Fetch, decode, and cache an object stream by its object number.
    fn get_objstm(&self, stream: u32) -> Result<Rc<ObjStm>> {
        if let Some(os) = self.objstms.borrow().get(&stream) {
            return Ok(Rc::clone(os));
        }
        let os = Rc::new(self.parse_objstm(stream)?);
        self.objstms.borrow_mut().insert(stream, Rc::clone(&os));
        Ok(os)
    }

    /// Resolve the object-stream object, decode its body, and parse its header.
    fn parse_objstm(&self, stream: u32) -> Result<ObjStm> {
        let obj = self.get(ObjRef::new(stream, 0))?;
        let s = obj
            .as_stream()
            .ok_or_else(|| bad_object("ObjStm is not a stream"))?;
        let decoded = decode_stream(&s.dict, &s.data)?;
        ObjStm::parse(&s.dict, decoded)
    }

    /// The resolved integer `/Length`, if present and non-negative.
    fn declared_length(&self, dict: &Dictionary) -> Option<usize> {
        let obj = dict.get("Length")?;
        let resolved = self.resolve(obj).ok()?;
        usize::try_from(resolved.as_integer()?).ok()
    }
}

/// Parse an indirect object at `offset` using only a directly-stated `/Length`
/// (no reference resolution). Used to bootstrap cross-reference and object
/// streams, whose `/Length` is always a direct integer.
pub(crate) fn parse_indirect_direct(data: &[u8], offset: usize) -> Result<(u32, Object)> {
    let mut lx = Lexer::at(data, offset);
    let (num, _generation) = read_header(&mut lx)?;
    let value = parse_object(&mut lx)?;
    let length = value.as_dict().and_then(direct_length);
    let obj = assemble_stream(data, &mut lx, value, length)?;
    Ok((num, obj))
}

/// A directly-stated non-negative integer `/Length`, if present.
fn direct_length(dict: &Dictionary) -> Option<usize> {
    usize::try_from(dict.get("Length")?.as_integer()?).ok()
}

/// Assemble a stream object when a dictionary is followed by `stream`, using
/// `length` (when known and in range) or an `endstream` scan otherwise.
fn assemble_stream(
    data: &[u8],
    lx: &mut Lexer,
    value: Object,
    length: Option<usize>,
) -> Result<Object> {
    let dict = match value {
        Object::Dictionary(d) => d,
        other => return Ok(other),
    };
    lx.skip_whitespace();
    if !lx.eat_keyword(b"stream") {
        return Ok(Object::Dictionary(dict));
    }
    consume_eol(lx);
    let start = lx.pos();
    let body = match length.and_then(|n| take_exact(data, start, n)) {
        Some(b) => b,
        None => scan_to_endstream(data, start),
    };
    Ok(Object::Stream(Stream { dict, data: body }))
}

/// Read `n g obj` and return the object number and generation.
fn read_header(lx: &mut Lexer) -> Result<(u32, u16)> {
    let num = read_uint(lx)?;
    let generation = read_uint(lx)? as u16;
    lx.skip_whitespace();
    if !lx.eat_keyword(b"obj") {
        return Err(bad_object("missing obj keyword"));
    }
    Ok((num, generation))
}

/// Read a whitespace-led unsigned integer.
fn read_uint(lx: &mut Lexer) -> Result<u32> {
    lx.skip_whitespace();
    let tok = lx
        .read_keyword()
        .ok_or_else(|| bad_object("expected integer"))?;
    std::str::from_utf8(tok)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad_object("bad integer"))
}

/// Consume the end-of-line after the `stream` keyword (CRLF or LF, lenient CR).
fn consume_eol(lx: &mut Lexer) {
    if lx.peek() == Some(b'\r') {
        lx.bump();
    }
    if lx.peek() == Some(b'\n') {
        lx.bump();
    }
}

/// Slice exactly `n` bytes at `start`, if they fit and `endstream` follows.
fn take_exact(data: &[u8], start: usize, n: usize) -> Option<Vec<u8>> {
    let end = start.checked_add(n)?;
    let body = data.get(start..end)?;
    if endstream_follows(data, end) {
        Some(body.to_vec())
    } else {
        None
    }
}

/// True when `endstream` appears within a few bytes of `pos` (allowing EOL).
fn endstream_follows(data: &[u8], pos: usize) -> bool {
    let window = data.get(pos..(pos + 24).min(data.len())).unwrap_or(&[]);
    window.windows(9).any(|w| w == b"endstream")
}

/// Recover a stream body by scanning forward to `endstream`.
fn scan_to_endstream(data: &[u8], start: usize) -> Vec<u8> {
    let rest = data.get(start..).unwrap_or(&[]);
    let idx = rest
        .windows(9)
        .position(|w| w == b"endstream")
        .unwrap_or(rest.len());
    rest[..trim_trailing_eol(&rest[..idx])].to_vec()
}

/// Length of `body` with one trailing EOL (CRLF/LF/CR) removed.
fn trim_trailing_eol(body: &[u8]) -> usize {
    match body.last() {
        Some(b'\n') => drop_lf(body),
        Some(b'\r') => body.len() - 1,
        _ => body.len(),
    }
}

fn drop_lf(body: &[u8]) -> usize {
    let without_lf = body.len() - 1;
    if body.get(without_lf.wrapping_sub(1)) == Some(&b'\r') {
        without_lf - 1
    } else {
        without_lf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xref::read_xref;

    fn doc_resolver(data: &[u8]) -> Resolver<'_> {
        let xref = read_xref(data).unwrap();
        Resolver::new_from_owned(data, xref)
    }

    // Test-only constructor wrapper so the Resolver borrows the test buffer.
    impl<'a> Resolver<'a> {
        fn new_from_owned(data: &'a [u8], xref: Xref) -> Self {
            Resolver::new(data, xref)
        }
    }

    // Build a tiny but valid PDF: catalog (1), a stream (2) with literal Length.
    fn sample() -> Vec<u8> {
        let mut pdf = String::new();
        pdf.push_str("%PDF-1.4\n");
        let off1 = pdf.len();
        pdf.push_str("1 0 obj\n<< /Type /Catalog /S 2 0 R >>\nendobj\n");
        let off2 = pdf.len();
        pdf.push_str("2 0 obj\n<< /Length 5 >>\nstream\nHELLO\nendstream\nendobj\n");
        let xref_off = pdf.len();
        pdf.push_str("xref\n0 3\n");
        pdf.push_str("0000000000 65535 f \n");
        pdf.push_str(&format!("{off1:010} 00000 n \n"));
        pdf.push_str(&format!("{off2:010} 00000 n \n"));
        pdf.push_str("trailer\n<< /Size 3 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xref_off}\n%%EOF"));
        pdf.into_bytes()
    }

    #[test]
    fn resolves_catalog_and_reference() {
        let data = sample();
        let r = doc_resolver(&data);
        let cat = r.get(ObjRef::new(1, 0)).unwrap();
        assert_eq!(
            cat.as_dict().unwrap().get("Type").unwrap().as_name(),
            Some("Catalog")
        );
        // The /S entry is an indirect reference resolved to the stream object.
        let s = cat.as_dict().unwrap().get("S").unwrap().clone();
        let resolved = r.resolve(&s).unwrap();
        assert_eq!(resolved.as_stream().unwrap().data, b"HELLO");
    }

    #[test]
    fn caches_repeated_get() {
        let data = sample();
        let r = doc_resolver(&data);
        let a = r.get(ObjRef::new(1, 0)).unwrap();
        let b = r.get(ObjRef::new(1, 0)).unwrap(); // served from cache
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_direct_object_is_clone() {
        let data = sample();
        let r = doc_resolver(&data);
        assert_eq!(r.resolve(&Object::Integer(7)).unwrap(), Object::Integer(7));
    }

    #[test]
    fn resolve_dict_variants() {
        let data = sample();
        let r = doc_resolver(&data);
        let cat_ref = Object::Reference(ObjRef::new(1, 0));
        assert!(r.resolve_dict(&cat_ref).unwrap().get("Type").is_some());
        let stream_ref = Object::Reference(ObjRef::new(2, 0));
        assert!(r.resolve_dict(&stream_ref).unwrap().get("Length").is_some());
        assert!(r.resolve_dict(&Object::Integer(1)).is_err());
    }

    #[test]
    fn missing_object_errors() {
        let data = sample();
        let r = doc_resolver(&data);
        assert_eq!(
            r.get(ObjRef::new(99, 0)).unwrap_err().code,
            ErrorCode::ObjectNotFound
        );
    }

    #[test]
    fn stream_recovered_by_scan_when_length_wrong() {
        // /Length is absent → body recovered by scanning to endstream.
        let mut pdf = String::from("%PDF-1.4\n");
        let off1 = pdf.len();
        pdf.push_str("1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        let off2 = pdf.len();
        pdf.push_str("2 0 obj\n<< >>\nstream\nDATA\nendstream\nendobj\n");
        let xoff = pdf.len();
        pdf.push_str("xref\n0 3\n0000000000 65535 f \n");
        pdf.push_str(&format!("{off1:010} 00000 n \n{off2:010} 00000 n \n"));
        pdf.push_str("trailer\n<< /Size 3 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        let data = pdf.into_bytes();
        let r = doc_resolver(&data);
        assert_eq!(
            r.get(ObjRef::new(2, 0)).unwrap().as_stream().unwrap().data,
            b"DATA"
        );
    }

    #[test]
    fn header_helpers() {
        // read_uint rejects junk.
        assert!(read_uint(&mut Lexer::new(b"   ")).is_err());
        assert!(read_uint(&mut Lexer::new(b"/x")).is_err());
        // missing obj keyword.
        assert!(read_header(&mut Lexer::new(b"1 0 notobj")).is_err());
        // object-number mismatch is rejected (offset 9 holds object 1, not 5).
        let data = sample();
        let r = doc_resolver(&data);
        assert!(r.parse_indirect(9, 5).is_err());
    }

    #[test]
    fn eol_and_trim_helpers() {
        // CRLF after stream keyword consumed.
        let mut lx = Lexer::new(b"\r\nbody");
        consume_eol(&mut lx);
        assert_eq!(lx.peek(), Some(b'b'));
        // trailing EOL trimming for CRLF, LF, CR, none.
        assert_eq!(trim_trailing_eol(b"ab\r\n"), 2);
        assert_eq!(trim_trailing_eol(b"ab\n"), 2);
        assert_eq!(trim_trailing_eol(b"ab\r"), 2);
        assert_eq!(trim_trailing_eol(b"ab"), 2);
    }

    #[test]
    fn take_exact_bounds() {
        assert!(take_exact(b"HELLOendstream", 0, 99).is_none()); // out of range
        assert!(take_exact(b"HELLO....nope.", 0, 5).is_none()); // no endstream after
        assert_eq!(take_exact(b"HELLOendstream", 0, 5).unwrap(), b"HELLO");
    }

    #[test]
    fn non_dictionary_value_passes_through() {
        let mut lx = Lexer::new(b"");
        assert_eq!(
            assemble_stream(b"", &mut lx, Object::Integer(3), None).unwrap(),
            Object::Integer(3)
        );
    }

    #[test]
    fn dictionary_without_stream_keyword() {
        let mut d = Dictionary::new();
        d.insert("A", Object::Integer(1));
        let mut lx = Lexer::new(b"   endobj");
        let got = assemble_stream(b"   endobj", &mut lx, Object::Dictionary(d), None).unwrap();
        assert!(got.as_dict().is_some());
        assert!(got.as_stream().is_none());
    }

    #[test]
    fn parse_indirect_direct_reads_stream() {
        let data = b"3 0 obj\n<< /Length 5 >>\nstream\nHELLO\nendstream\nendobj";
        let (num, obj) = parse_indirect_direct(data, 0).unwrap();
        assert_eq!(num, 3);
        assert_eq!(obj.as_stream().unwrap().data, b"HELLO");
        assert_eq!(direct_length(obj.as_dict().unwrap()), Some(5));
    }
}
