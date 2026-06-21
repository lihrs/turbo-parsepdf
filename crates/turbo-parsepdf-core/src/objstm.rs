//! Object streams (`/Type /ObjStm`, ISO 32000-1 §7.5.7).
//!
//! PDF 1.5+ packs many non-stream indirect objects into one compressed stream to
//! shrink the file. After decompression the stream is a header of `N` integer
//! pairs `(object-number, relative-offset)` followed, at byte `/First`, by the
//! objects themselves. A cross-reference entry of type 2 names the object stream
//! and the index of the wanted object within it.

use crate::cos::parse_object;
use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::lex::Lexer;
use crate::object::{Dictionary, Object};

fn bad(msg: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::InvalidObject, msg)
}

/// A decoded object stream: the packed objects plus their offset header.
#[derive(Debug, Clone)]
pub struct ObjStm {
    numbers: Vec<u32>,
    offsets: Vec<usize>,
    first: usize,
    data: Vec<u8>,
}

impl ObjStm {
    /// Parse the header of a decoded object stream given its dictionary.
    pub fn parse(dict: &Dictionary, data: Vec<u8>) -> Result<ObjStm> {
        let n = uint(dict, "N")?;
        let first = uint(dict, "First")?;
        let (numbers, offsets) = read_header(&data, n)?;
        Ok(ObjStm {
            numbers,
            offsets,
            first,
            data,
        })
    }

    /// The number of packed objects.
    pub fn len(&self) -> usize {
        self.numbers.len()
    }

    /// True when the stream packs no objects.
    pub fn is_empty(&self) -> bool {
        self.numbers.is_empty()
    }

    /// The object number stored at `index`.
    pub fn object_number(&self, index: usize) -> Option<u32> {
        self.numbers.get(index).copied()
    }

    /// Parse and return the object at `index` within the stream.
    pub fn get(&self, index: usize) -> Result<Object> {
        let rel = *self
            .offsets
            .get(index)
            .ok_or_else(|| bad("objstm index out of range"))?;
        let start = self.first + rel;
        let mut lx = Lexer::at(&self.data, start);
        parse_object(&mut lx)
    }
}

/// Read the `N` `(number, offset)` integer pairs at the head of the stream.
fn read_header(data: &[u8], n: usize) -> Result<(Vec<u32>, Vec<usize>)> {
    let mut lx = Lexer::new(data);
    let mut numbers = Vec::with_capacity(n);
    let mut offsets = Vec::with_capacity(n);
    for _ in 0..n {
        numbers.push(read_uint(&mut lx)? as u32);
        offsets.push(read_uint(&mut lx)? as usize);
    }
    Ok((numbers, offsets))
}

/// Read a required non-negative integer entry from the stream dictionary.
fn uint(dict: &Dictionary, key: &str) -> Result<usize> {
    let n = dict
        .get(key)
        .and_then(|o| o.as_integer())
        .ok_or_else(|| bad("objstm missing key"))?;
    usize::try_from(n).map_err(|_| bad("objstm negative key"))
}

/// Read a whitespace-led unsigned integer token from the header.
fn read_uint(lx: &mut Lexer) -> Result<u64> {
    lx.skip_whitespace();
    let tok = lx
        .read_keyword()
        .ok_or_else(|| bad("objstm truncated header"))?;
    std::str::from_utf8(tok)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("objstm bad integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objstm(n: i64, first: i64, body: &str) -> ObjStm {
        let mut d = Dictionary::new();
        d.insert("N", Object::Integer(n));
        d.insert("First", Object::Integer(first));
        ObjStm::parse(&d, body.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn parses_two_objects() {
        // header "10 0 20 10 " (obj10@rel0, obj20@rel10), First=11, then objects.
        let body = "10 0 20 10 << /A 1 >>(hi)";
        let os = objstm(2, 11, body);
        assert_eq!(os.len(), 2);
        assert!(!os.is_empty());
        assert_eq!(os.object_number(0), Some(10));
        assert_eq!(os.object_number(1), Some(20));
        assert_eq!(
            os.get(0)
                .unwrap()
                .as_dict()
                .unwrap()
                .get("A")
                .unwrap()
                .as_integer(),
            Some(1)
        );
        assert_eq!(os.get(1).unwrap().as_string(), Some(&b"hi"[..]));
    }

    #[test]
    fn index_out_of_range_errors() {
        let os = objstm(1, 4, "10 0 42");
        assert!(os.get(5).is_err());
        assert_eq!(os.object_number(9), None);
    }

    #[test]
    fn missing_keys_error() {
        let mut d = Dictionary::new();
        d.insert("N", Object::Integer(1));
        assert!(ObjStm::parse(&d, b"0 0 1".to_vec()).is_err()); // no First
        let mut d2 = Dictionary::new();
        d2.insert("N", Object::Integer(-1));
        d2.insert("First", Object::Integer(0));
        assert!(ObjStm::parse(&d2, b"".to_vec()).is_err()); // negative N
    }

    #[test]
    fn truncated_header_errors() {
        let mut d = Dictionary::new();
        d.insert("N", Object::Integer(3));
        d.insert("First", Object::Integer(0));
        assert!(ObjStm::parse(&d, b"10 0".to_vec()).is_err());
    }
}
