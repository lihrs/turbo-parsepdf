//! Cross-reference resolution: where every indirect object lives in the file.
//!
//! A reader starts at the tail (`startxref` → byte offset → the most recent xref
//! section) and walks `/Prev` back through earlier incremental-update sections,
//! letting newer entries win. Both forms are read: the **classic** xref *table*
//! (`xref` keyword + 20-byte entry lines + `trailer` dictionary) and the
//! **cross-reference stream** (PDF 1.5+, `/Type /XRef` with packed binary
//! entries), including hybrid `/XRefStm` files that carry both.

use std::collections::{BTreeMap, HashSet};

use crate::cos::parse_object;
use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::filter::decode_stream;
use crate::lex::Lexer;
use crate::object::{Dictionary, Object};
use crate::resolver::parse_indirect_direct;

fn bad_xref(msg: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::InvalidXref, msg)
}

/// One cross-reference entry: a free slot, an in-use object at a byte offset, or
/// an object packed inside an object stream (xref-stream type 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrefEntry {
    /// A free (deleted) object slot.
    Free,
    /// An in-use object stored at `offset` with the given generation number.
    InUse { offset: usize, gen: u16 },
    /// An object packed at `index` inside the object stream numbered `stream`.
    Compressed { stream: u32, index: u32 },
}

/// The merged cross-reference table plus the document trailer.
#[derive(Debug, Clone, Default)]
pub struct Xref {
    entries: BTreeMap<u32, XrefEntry>,
    trailer: Dictionary,
}

impl Xref {
    /// The entry for an object number, if present.
    pub fn get(&self, num: u32) -> Option<XrefEntry> {
        self.entries.get(&num).copied()
    }

    /// The merged trailer dictionary (newest section's `/Root`, `/Size`, …).
    pub fn trailer(&self) -> &Dictionary {
        &self.trailer
    }

    /// The number of cross-reference entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries were collected.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Read and merge the full cross-reference chain for `data`.
pub fn read_xref(data: &[u8]) -> Result<Xref> {
    let start = find_startxref(data)?;
    follow_chain(data, start)
}

/// Locate the byte offset declared by the file's last `startxref`.
fn find_startxref(data: &[u8]) -> Result<usize> {
    let kw = b"startxref";
    let idx = data
        .windows(kw.len())
        .rposition(|w| w == kw)
        .ok_or_else(|| bad_xref("no startxref"))?;
    let mut lx = Lexer::at(data, idx + kw.len());
    read_uint(&mut lx).map(|n| n as usize)
}

/// Walk xref sections from `start` back through `/Prev`, merging any hybrid
/// `/XRefStm` cross-reference stream attached to a classic section. The first
/// (newest) section's trailer wins; classic entries take precedence over the
/// hybrid stream's entries for the same object.
fn follow_chain(data: &[u8], start: usize) -> Result<Xref> {
    let mut merged = Xref::default();
    let mut seen = HashSet::new();
    let mut offset = start;
    while seen.insert(offset) {
        let section = parse_section(data, offset)?;
        let prev = prev_offset(&section.trailer);
        let hybrid = xrefstm_offset(&section.trailer);
        merge_section(&mut merged, section);
        merge_hybrid(data, &mut merged, hybrid, &mut seen);
        match prev {
            Some(p) => offset = p,
            None => break,
        }
    }
    Ok(merged)
}

/// Merge a hybrid `/XRefStm` cross-reference stream's entries (the compressed
/// objects a classic section cannot record). Trailer keys are left untouched.
fn merge_hybrid(data: &[u8], merged: &mut Xref, hybrid: Option<usize>, seen: &mut HashSet<usize>) {
    if let Some(off) = hybrid {
        if seen.insert(off) {
            if let Ok(stream_section) = parse_xref_stream(data, off) {
                merge_entries(merged, stream_section.entries);
            }
        }
    }
}

/// The hybrid `/XRefStm` byte offset, if present.
fn xrefstm_offset(trailer: &Dictionary) -> Option<usize> {
    usize::try_from(trailer.get("XRefStm")?.as_integer()?).ok()
}

/// Fold a freshly parsed section into the running result: older entries never
/// overwrite newer ones, and only the newest (first-seen) trailer's keys are kept.
fn merge_section(merged: &mut Xref, section: Xref) {
    let take_trailer = merged.trailer.is_empty();
    merge_entries(merged, section.entries);
    if take_trailer {
        merged.trailer = section.trailer;
    }
}

/// Merge entries with first-seen-wins precedence.
fn merge_entries(merged: &mut Xref, entries: BTreeMap<u32, XrefEntry>) {
    for (num, entry) in entries {
        merged.entries.entry(num).or_insert(entry);
    }
}

/// The `/Prev` byte offset of an earlier section, if any.
fn prev_offset(trailer: &Dictionary) -> Option<usize> {
    let n = trailer.get("Prev")?.as_integer()?;
    usize::try_from(n).ok()
}

/// Parse one xref section at `offset`: a classic table, or a cross-reference
/// stream when the offset points at an indirect object instead of `xref`.
fn parse_section(data: &[u8], offset: usize) -> Result<Xref> {
    let mut lx = Lexer::at(data, offset);
    lx.skip_whitespace();
    if lx.eat_keyword(b"xref") {
        let entries = read_subsections(&mut lx)?;
        let trailer = read_trailer(&mut lx)?;
        Ok(Xref { entries, trailer })
    } else {
        parse_xref_stream(data, offset)
    }
}

/// Parse a cross-reference stream (`/Type /XRef`) at `offset`.
fn parse_xref_stream(data: &[u8], offset: usize) -> Result<Xref> {
    let (_num, obj) = parse_indirect_direct(data, offset)?;
    let stream = obj
        .as_stream()
        .ok_or_else(|| bad_xref("xref stream is not a stream"))?;
    let decoded = decode_stream(&stream.dict, &stream.data).map_err(|_| bad_xref("xref decode"))?;
    let entries = parse_xref_entries(&stream.dict, &decoded)?;
    Ok(Xref {
        entries,
        trailer: stream.dict.clone(),
    })
}

/// Decode the packed binary entries of a cross-reference stream.
fn parse_xref_entries(dict: &Dictionary, data: &[u8]) -> Result<BTreeMap<u32, XrefEntry>> {
    let widths = field_widths(dict)?;
    let record = widths.iter().sum::<usize>();
    let index = index_pairs(dict)?;
    decode_ranges(data, &widths, record, &index)
}

/// Decode every `/Index` range of binary records.
fn decode_ranges(
    data: &[u8],
    widths: &[usize; 3],
    record: usize,
    index: &[(u32, u32)],
) -> Result<BTreeMap<u32, XrefEntry>> {
    if record == 0 {
        return Err(bad_xref("xref stream zero-width records"));
    }
    let mut entries = BTreeMap::new();
    let mut cursor = 0usize;
    for &(start, count) in index {
        cursor = read_index_range(data, widths, record, cursor, (start, count), &mut entries)?;
    }
    Ok(entries)
}

/// The three `/W` field widths.
fn field_widths(dict: &Dictionary) -> Result<[usize; 3]> {
    let arr = dict
        .get("W")
        .and_then(|o| o.as_array())
        .ok_or_else(|| bad_xref("xref /W missing"))?;
    let w: Vec<usize> = arr
        .iter()
        .filter_map(|o| o.as_integer())
        .filter_map(|n| usize::try_from(n).ok())
        .collect();
    match w.as_slice() {
        [a, b, c] => Ok([*a, *b, *c]),
        _ => Err(bad_xref("xref /W must have three widths")),
    }
}

/// The `/Index` subsection pairs (default `[0 Size]`).
fn index_pairs(dict: &Dictionary) -> Result<Vec<(u32, u32)>> {
    match dict.get("Index").and_then(|o| o.as_array()) {
        Some(arr) => Ok(pairs_from_array(arr)),
        None => default_index(dict),
    }
}

fn pairs_from_array(arr: &[Object]) -> Vec<(u32, u32)> {
    arr.chunks_exact(2)
        .filter_map(|c| Some((to_u32(&c[0])?, to_u32(&c[1])?)))
        .collect()
}

fn default_index(dict: &Dictionary) -> Result<Vec<(u32, u32)>> {
    let size = dict
        .get("Size")
        .and_then(|o| o.as_integer())
        .ok_or_else(|| bad_xref("xref /Size missing"))?;
    Ok(vec![(
        0,
        u32::try_from(size).map_err(|_| bad_xref("bad /Size"))?,
    )])
}

fn to_u32(obj: &Object) -> Option<u32> {
    u32::try_from(obj.as_integer()?).ok()
}

/// Read `count` records of one `/Index` range; return the new byte cursor.
fn read_index_range(
    data: &[u8],
    widths: &[usize; 3],
    record: usize,
    mut cursor: usize,
    range: (u32, u32),
    entries: &mut BTreeMap<u32, XrefEntry>,
) -> Result<usize> {
    let (start, count) = range;
    for i in 0..count {
        let chunk = data
            .get(cursor..cursor + record)
            .ok_or_else(|| bad_xref("xref stream truncated"))?;
        if let Some(entry) = decode_record(widths, chunk) {
            entries.entry(start + i).or_insert(entry);
        }
        cursor += record;
    }
    Ok(cursor)
}

/// Decode one binary record into an entry (type 0 free / 1 in-use / 2 compressed).
fn decode_record(widths: &[usize; 3], chunk: &[u8]) -> Option<XrefEntry> {
    let (f1, rest) = chunk.split_at(widths[0]);
    let (f2, f3) = rest.split_at(widths[1]);
    let kind = if widths[0] == 0 { 1 } else { be(f1) };
    match kind {
        0 => Some(XrefEntry::Free),
        1 => Some(XrefEntry::InUse {
            offset: be(f2) as usize,
            gen: be(f3) as u16,
        }),
        2 => Some(XrefEntry::Compressed {
            stream: be(f2) as u32,
            index: be(f3) as u32,
        }),
        _ => None,
    }
}

/// Read a big-endian unsigned integer from a field of up to 8 bytes.
fn be(field: &[u8]) -> u64 {
    field.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

/// Read every `start count` subsection until the `trailer` keyword.
fn read_subsections(lx: &mut Lexer) -> Result<BTreeMap<u32, XrefEntry>> {
    let mut entries = BTreeMap::new();
    loop {
        lx.skip_whitespace();
        if lx.eat_keyword(b"trailer") {
            return Ok(entries);
        }
        read_one_subsection(lx, &mut entries)?;
    }
}

/// Read a single subsection header and its entry lines.
fn read_one_subsection(lx: &mut Lexer, entries: &mut BTreeMap<u32, XrefEntry>) -> Result<()> {
    let start = read_uint(lx)?;
    let count = read_uint(lx)?;
    for i in 0..count {
        let entry = read_entry(lx)?;
        entries.entry(start + i).or_insert(entry);
    }
    Ok(())
}

/// Read one 20-byte entry line (`offset gen n|f`), leniently tokenized.
fn read_entry(lx: &mut Lexer) -> Result<XrefEntry> {
    let offset = read_uint(lx)? as usize;
    let gen = read_uint(lx)? as u16;
    lx.skip_whitespace();
    match lx.read_keyword() {
        Some(b"n") => Ok(XrefEntry::InUse { offset, gen }),
        Some(b"f") => Ok(XrefEntry::Free),
        _ => Err(bad_xref("bad xref entry type")),
    }
}

/// Parse the `trailer` dictionary that follows the table.
fn read_trailer(lx: &mut Lexer) -> Result<Dictionary> {
    match parse_object(lx)? {
        Object::Dictionary(d) => Ok(d),
        _ => Err(bad_xref("trailer is not a dictionary")),
    }
}

/// Read a whitespace-led unsigned integer token.
fn read_uint(lx: &mut Lexer) -> Result<u32> {
    lx.skip_whitespace();
    let tok = lx
        .read_keyword()
        .ok_or_else(|| bad_xref("expected integer"))?;
    std::str::from_utf8(tok)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad_xref("bad integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal single-section file body: 3 objects + xref + trailer + startxref.
    const SIMPLE: &[u8] = b"%PDF-1.4\n\
1 0 obj<</Type/Catalog>>endobj\n\
xref\n\
0 2\n\
0000000000 65535 f \n\
0000000009 00000 n \n\
trailer\n\
<< /Size 2 /Root 1 0 R >>\n\
startxref\n\
9\n\
%%EOF";

    #[test]
    fn reads_simple_xref_and_trailer() {
        // Point startxref at the real xref keyword offset.
        let kw = b"xref\n";
        let off = SIMPLE.windows(kw.len()).position(|w| w == kw).unwrap();
        // Rewrite the startxref offset to the located xref position.
        let s = format!("startxref\n{off}\n%%EOF");
        let head = &SIMPLE[..SIMPLE.windows(9).position(|w| w == b"startxref").unwrap()];
        let doc = [head, s.as_bytes()].concat();

        let xref = read_xref(&doc).unwrap();
        assert_eq!(xref.len(), 2);
        assert_eq!(xref.get(0), Some(XrefEntry::Free));
        assert_eq!(xref.get(1), Some(XrefEntry::InUse { offset: 9, gen: 0 }));
        assert!(!xref.is_empty());
        assert_eq!(xref.trailer().get("Size").unwrap().as_integer(), Some(2));
    }

    fn xref_at(data: &[u8]) -> Result<Xref> {
        // Helper: build startxref pointing at the xref keyword, then read.
        let off = data.windows(4).position(|w| w == b"xref").unwrap();
        let doc = [data, format!("\nstartxref\n{off}\n%%EOF").as_bytes()].concat();
        read_xref(&doc)
    }

    #[test]
    fn two_subsections_merge() {
        let body = b"xref\n0 1\n0000000000 65535 f \n5 1\n0000000042 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R >>";
        let xref = xref_at(body).unwrap();
        assert_eq!(xref.get(0), Some(XrefEntry::Free));
        assert_eq!(xref.get(5), Some(XrefEntry::InUse { offset: 42, gen: 0 }));
    }

    #[test]
    fn missing_startxref_errors() {
        assert!(read_xref(b"%PDF-1.4 no pointer here").is_err());
    }

    #[test]
    fn wrong_keyword_errors() {
        let doc = b"notxref\nstartxref\n0\n%%EOF";
        assert!(read_xref(doc).is_err());
    }

    #[test]
    fn bad_entry_type_errors() {
        let body = b"xref\n0 1\n0000000000 00000 x \ntrailer\n<< /Size 1 >>";
        assert!(xref_at(body).is_err());
    }

    #[test]
    fn non_dict_trailer_errors() {
        let body = b"xref\n0 1\n0000000000 65535 f \ntrailer\n42";
        assert!(xref_at(body).is_err());
    }

    #[test]
    fn bad_integer_in_table_errors() {
        let body = b"xref\nZZ 1\n0000000000 65535 f \ntrailer\n<< >>";
        assert!(xref_at(body).is_err());
    }

    #[test]
    fn truncated_subsection_errors() {
        let body = b"xref\n0 1\n";
        assert!(xref_at(body).is_err());
    }

    #[test]
    fn prev_chain_is_followed_and_cycle_guarded() {
        // Section A at higher offset has /Prev → section B; B has /Prev → A (cycle).
        // Build a doc with two xref sections; the walk must terminate.
        let sec_b = "xref\n0 1\n0000000000 65535 f \n2 1\n0000000099 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\n";
        let prefix = "%PDF-1.4\n";
        let off_b = prefix.len();
        let sec_a = format!(
            "xref\n1 1\n0000000010 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R /Prev {off_b} >>\n"
        );
        let mut doc = String::new();
        doc.push_str(prefix);
        doc.push_str(sec_b);
        let off_a = doc.len();
        doc.push_str(&sec_a);
        doc.push_str(&format!("startxref\n{off_a}\n%%EOF"));

        let xref = read_xref(doc.as_bytes()).unwrap();
        // Object 1 from newest section A, object 2 from older section B.
        assert_eq!(xref.get(1), Some(XrefEntry::InUse { offset: 10, gen: 0 }));
        assert_eq!(xref.get(2), Some(XrefEntry::InUse { offset: 99, gen: 0 }));
    }

    // ---- cross-reference stream tests ----

    fn dict(pairs: &[(&str, Object)]) -> Dictionary {
        let mut d = Dictionary::new();
        for (k, v) in pairs {
            d.insert(*k, v.clone());
        }
        d
    }

    fn arr(ints: &[i64]) -> Object {
        Object::Array(ints.iter().map(|&n| Object::Integer(n)).collect())
    }

    #[test]
    fn decode_record_all_types() {
        let w = [1, 2, 1];
        assert_eq!(decode_record(&w, &[0, 0, 0, 0xff]), Some(XrefEntry::Free));
        assert_eq!(
            decode_record(&w, &[1, 0x01, 0x10, 0x00]),
            Some(XrefEntry::InUse {
                offset: 0x0110,
                gen: 0
            })
        );
        assert_eq!(
            decode_record(&w, &[2, 0x00, 0x05, 0x03]),
            Some(XrefEntry::Compressed {
                stream: 5,
                index: 3
            })
        );
        assert_eq!(decode_record(&w, &[9, 0, 0, 0]), None); // unknown type
    }

    #[test]
    fn first_width_zero_defaults_to_inuse() {
        // /W [0 2 1]: a zero type field defaults the entry type to 1 (in-use).
        let w = [0, 2, 1];
        assert_eq!(
            decode_record(&w, &[0x00, 0x2a, 0x00]),
            Some(XrefEntry::InUse {
                offset: 0x2a,
                gen: 0
            })
        );
    }

    #[test]
    fn field_widths_validation() {
        assert!(field_widths(&dict(&[("W", arr(&[1, 2, 1]))])).is_ok());
        assert!(field_widths(&dict(&[])).is_err());
        assert!(field_widths(&dict(&[("W", arr(&[1, 2]))])).is_err());
    }

    #[test]
    fn index_pairs_default_and_explicit() {
        let from_size = index_pairs(&dict(&[("Size", Object::Integer(4))])).unwrap();
        assert_eq!(from_size, vec![(0, 4)]);
        let explicit = index_pairs(&dict(&[("Index", arr(&[2, 3, 10, 1]))])).unwrap();
        assert_eq!(explicit, vec![(2, 3), (10, 1)]);
        assert!(index_pairs(&dict(&[])).is_err()); // no Size, no Index
    }

    #[test]
    fn zero_width_record_errors() {
        let d = dict(&[("W", arr(&[0, 0, 0])), ("Size", Object::Integer(1))]);
        assert!(parse_xref_entries(&d, &[]).is_err());
    }

    #[test]
    fn truncated_xref_stream_errors() {
        let d = dict(&[("W", arr(&[1, 2, 1])), ("Size", Object::Integer(3))]);
        assert!(parse_xref_entries(&d, &[1, 0, 0, 0]).is_err()); // only one record for Size 3
    }

    // Build a full xref-stream PDF: catalog (1), an ObjStm (2) packing object 4,
    // a Pages object (5), and a cross-reference stream (3) recording them all.
    fn xref_stream_pdf() -> Vec<u8> {
        let rec = |t: u8, f2: u16, f3: u8| [t, (f2 >> 8) as u8, f2 as u8, f3];
        let mut pdf: Vec<u8> = Vec::new();
        let put = |p: &mut Vec<u8>, s: &str| p.extend_from_slice(s.as_bytes());
        put(&mut pdf, "%PDF-1.5\n");
        let off1 = pdf.len();
        put(
            &mut pdf,
            "1 0 obj\n<< /Type /Catalog /Pages 5 0 R >>\nendobj\n",
        );
        let off2 = pdf.len();
        // ObjStm packs obj4 (at rel 0) and obj6 (at rel 23). Header "4 0 6 23 ",
        // First = 9 → object data begins after the header.
        let body = "4 0 6 23 << /Type /Test /V 99 >>(packed)";
        put(&mut pdf, &format!(
            "2 0 obj\n<< /Type /ObjStm /N 2 /First 9 /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            body.len(),
            body
        ));
        let off5 = pdf.len();
        put(&mut pdf, "5 0 obj\n<< /Type /Pages /Count 0 >>\nendobj\n");
        let off3 = pdf.len();
        let mut bin = Vec::new();
        for r in [
            rec(0, 0, 0xff),
            rec(1, off1 as u16, 0),
            rec(1, off2 as u16, 0),
            rec(1, off3 as u16, 0),
            rec(2, 2, 0),
            rec(1, off5 as u16, 0),
            rec(2, 2, 1),
        ] {
            bin.extend_from_slice(&r);
        }
        put(&mut pdf, &format!(
            "3 0 obj\n<< /Type /XRef /Size 7 /Root 1 0 R /W [1 2 1] /Index [0 7] /Length {} >>\nstream\n",
            bin.len()
        ));
        pdf.extend_from_slice(&bin);
        put(&mut pdf, "\nendstream\nendobj\n");
        put(&mut pdf, &format!("startxref\n{off3}\n%%EOF"));
        pdf
    }

    #[test]
    fn reads_cross_reference_stream() {
        let data = xref_stream_pdf();
        let xref = read_xref(&data).unwrap();
        assert_eq!(
            xref.trailer()
                .get("Root")
                .unwrap()
                .as_reference()
                .unwrap()
                .num,
            1
        );
        assert_eq!(xref.get(0), Some(XrefEntry::Free));
        assert!(matches!(xref.get(1), Some(XrefEntry::InUse { .. })));
        assert_eq!(
            xref.get(4),
            Some(XrefEntry::Compressed {
                stream: 2,
                index: 0
            })
        );
    }

    #[test]
    fn compressed_object_resolves_end_to_end() {
        use crate::doc::Document;
        use crate::object::ObjRef;
        let data = xref_stream_pdf();
        let doc = Document::parse(&data).unwrap();
        let obj4 = doc.get(ObjRef::new(4, 0)).unwrap();
        assert_eq!(
            obj4.as_dict().unwrap().get("V").unwrap().as_integer(),
            Some(99)
        );
        // A second compressed object from the same ObjStm hits the cached stream.
        let obj6 = doc.get(ObjRef::new(6, 0)).unwrap();
        assert_eq!(obj6.as_string(), Some(&b"packed"[..]));
    }

    // Build a hybrid file: a classic xref table whose trailer carries `/XRefStm`
    // pointing at a cross-reference stream that records a compressed object.
    fn hybrid_pdf() -> Vec<u8> {
        let rec = |t: u8, f2: u16, f3: u8| [t, (f2 >> 8) as u8, f2 as u8, f3];
        let mut pdf: Vec<u8> = Vec::new();
        let put = |p: &mut Vec<u8>, s: &str| p.extend_from_slice(s.as_bytes());
        put(&mut pdf, "%PDF-1.5\n");
        let off1 = pdf.len();
        put(
            &mut pdf,
            "1 0 obj\n<< /Type /Catalog /Pages 5 0 R >>\nendobj\n",
        );
        let off2 = pdf.len();
        let body = "4 0 << /Type /Test /V 7 >>";
        put(&mut pdf, &format!(
            "2 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            body.len(),
            body
        ));
        let off5 = pdf.len();
        put(&mut pdf, "5 0 obj\n<< /Type /Pages /Count 0 >>\nendobj\n");
        // Cross-reference stream (obj 3) records the compressed object 4.
        let off3 = pdf.len();
        let bin = rec(2, 2, 0).to_vec();
        put(&mut pdf, &format!(
            "3 0 obj\n<< /Type /XRef /Size 7 /Root 1 0 R /W [1 2 1] /Index [4 1] /Length {} >>\nstream\n",
            bin.len()
        ));
        pdf.extend_from_slice(&bin);
        put(&mut pdf, "\nendstream\nendobj\n");
        // Classic table for the uncompressed objects, trailer with /XRefStm.
        let xref_off = pdf.len();
        put(&mut pdf, "xref\n0 1\n0000000000 65535 f \n");
        put(&mut pdf, &format!("1 1\n{off1:010} 00000 n \n"));
        put(&mut pdf, &format!("2 1\n{off2:010} 00000 n \n"));
        put(&mut pdf, &format!("3 1\n{off3:010} 00000 n \n"));
        put(&mut pdf, &format!("5 1\n{off5:010} 00000 n \n"));
        put(
            &mut pdf,
            &format!("trailer\n<< /Size 7 /Root 1 0 R /XRefStm {off3} >>\n"),
        );
        put(&mut pdf, &format!("startxref\n{xref_off}\n%%EOF"));
        pdf
    }

    #[test]
    fn hybrid_xrefstm_merges_compressed_entries() {
        use crate::doc::Document;
        use crate::object::ObjRef;
        let data = hybrid_pdf();
        let xref = read_xref(&data).unwrap();
        // The compressed object 4 comes only from the hybrid /XRefStm.
        assert_eq!(
            xref.get(4),
            Some(XrefEntry::Compressed {
                stream: 2,
                index: 0
            })
        );
        assert!(matches!(xref.get(1), Some(XrefEntry::InUse { .. })));
        let doc = Document::parse(&data).unwrap();
        let obj4 = doc.get(ObjRef::new(4, 0)).unwrap();
        assert_eq!(
            obj4.as_dict().unwrap().get("V").unwrap().as_integer(),
            Some(7)
        );
    }

    #[test]
    fn non_stream_at_offset_errors() {
        // startxref points at a plain (non-stream) object → xref-stream parse fails.
        let mut pdf = String::from("%PDF-1.5\n");
        let off = pdf.len();
        pdf.push_str("1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        pdf.push_str(&format!("startxref\n{off}\n%%EOF"));
        assert!(read_xref(pdf.as_bytes()).is_err());
    }
}
