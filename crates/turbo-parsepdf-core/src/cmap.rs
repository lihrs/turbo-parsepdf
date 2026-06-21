//! CMap parsing (ISO 32000-1 §9.10.3): the `/ToUnicode` code→text map.
//!
//! A `/ToUnicode` stream is a PostScript-flavoured CMap whose `bfchar` and
//! `bfrange` sections map character *codes* (1- or 2-byte, per the
//! `codespacerange`) to Unicode (UTF-16BE) strings. [`CMap::parse`] reads those
//! sections with the COS lexer; [`CMap::lookup`] resolves a code to its text, and
//! [`CMap::code_bytes`] reports how many bytes form one code so the caller can
//! split a shown string into codes.

use std::collections::HashMap;

use crate::cos::parse_object;
use crate::lex::Lexer;
use crate::object::Object;

/// A parsed `/ToUnicode` (or CID) CMap.
#[derive(Debug, Clone, Default)]
pub struct CMap {
    singles: HashMap<u32, String>,
    ranges: Vec<(u32, u32, u32)>,
    code_bytes: usize,
}

impl CMap {
    /// Parse a CMap stream's decoded bytes.
    pub fn parse(data: &[u8]) -> CMap {
        let mut lx = Lexer::new(data);
        let mut cmap = CMap::default();
        loop {
            lx.skip_whitespace();
            match lx.peek() {
                None => return cmap,
                Some(b) if is_value_start(b) => skip_value(&mut lx),
                _ => dispatch(&mut lx, &mut cmap),
            }
        }
    }

    /// How many bytes form one character code (1 if no codespace was declared).
    pub fn code_bytes(&self) -> usize {
        self.code_bytes.max(1)
    }

    /// Resolve a character code to its Unicode text.
    pub fn lookup(&self, code: u32) -> Option<String> {
        if let Some(s) = self.singles.get(&code) {
            return Some(s.clone());
        }
        self.ranges.iter().find_map(|r| range_lookup(r, code))
    }
}

/// Resolve a code within one contiguous `bfrange`.
fn range_lookup(range: &(u32, u32, u32), code: u32) -> Option<String> {
    let (lo, hi, dst) = *range;
    if code < lo || code > hi {
        return None;
    }
    char::from_u32(dst + (code - lo)).map(|c| c.to_string())
}

/// True for bytes that begin a CMap operand (count, hex string, array, name).
fn is_value_start(b: u8) -> bool {
    matches!(
        b,
        b'<' | b'[' | b'/' | b'(' | b'0'..=b'9' | b'+' | b'-' | b'.'
    )
}

/// Skip one operand (a count, a stray name, etc.).
fn skip_value(lx: &mut Lexer) {
    let _ = parse_object(lx);
}

/// Read a keyword and enter a section reader for the `begin…` markers.
fn dispatch(lx: &mut Lexer, cmap: &mut CMap) {
    let Some(kw) = lx.read_keyword() else {
        lx.bump();
        return;
    };
    match kw {
        b"begincodespacerange" => read_codespace(lx, cmap),
        b"beginbfchar" => read_bfchar(lx, cmap),
        b"beginbfrange" => read_bfrange(lx, cmap),
        _ => {}
    }
}

/// Read `<lo> <hi>` pairs, taking the code width from the first low code.
fn read_codespace(lx: &mut Lexer, cmap: &mut CMap) {
    while at_hex(lx) {
        let lo = read_hex(lx);
        let _hi = read_hex(lx);
        if let Some(bytes) = lo {
            if cmap.code_bytes == 0 {
                cmap.code_bytes = bytes.len();
            }
        }
    }
    consume_end(lx);
}

/// Read `<src> <dst>` pairs into single-code mappings.
fn read_bfchar(lx: &mut Lexer, cmap: &mut CMap) {
    while at_hex(lx) {
        let src = read_hex(lx);
        let dst = read_hex(lx);
        if let (Some(s), Some(d)) = (src, dst) {
            cmap.singles.insert(be(&s), utf16_be(&d));
        }
    }
    consume_end(lx);
}

/// Read `<lo> <hi> <dst>` / `<lo> <hi> [ … ]` range mappings.
fn read_bfrange(lx: &mut Lexer, cmap: &mut CMap) {
    while at_hex(lx) {
        read_bfrange_entry(lx, cmap);
    }
    consume_end(lx);
}

/// One `bfrange` entry: contiguous destination, or an explicit array.
fn read_bfrange_entry(lx: &mut Lexer, cmap: &mut CMap) {
    let lo = read_hex(lx).map(|b| be(&b));
    let hi = read_hex(lx).map(|b| be(&b));
    lx.skip_whitespace();
    if lx.peek() == Some(b'[') {
        read_bfrange_array(lx, cmap, lo);
    } else if let (Some(lo), Some(hi), Some(dst)) = (lo, hi, read_hex(lx)) {
        cmap.ranges.push((lo, hi, first_scalar(&dst)));
    }
}

/// The array form of a `bfrange`: one destination string per code.
fn read_bfrange_array(lx: &mut Lexer, cmap: &mut CMap, lo: Option<u32>) {
    let Some(start) = lo else { return };
    let Ok(Object::Array(items)) = parse_object(lx) else {
        return;
    };
    for (i, item) in items.iter().enumerate() {
        if let Object::String(bytes) = item {
            cmap.singles.insert(start + i as u32, utf16_be(bytes));
        }
    }
}

/// True when the next non-whitespace byte begins a hex string entry.
fn at_hex(lx: &mut Lexer) -> bool {
    lx.skip_whitespace();
    lx.peek() == Some(b'<')
}

/// Consume the section's `end…` keyword.
fn consume_end(lx: &mut Lexer) {
    lx.skip_whitespace();
    let _ = lx.read_keyword();
}

/// Read one hex string's bytes.
fn read_hex(lx: &mut Lexer) -> Option<Vec<u8>> {
    lx.skip_whitespace();
    match parse_object(lx) {
        Ok(Object::String(bytes)) => Some(bytes),
        _ => None,
    }
}

/// Interpret up to four big-endian bytes as a code.
fn be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
}

/// Decode UTF-16BE bytes into a string (odd trailing byte treated as a unit).
fn utf16_be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks(2).map(chunk_to_u16).collect();
    String::from_utf16_lossy(&units)
}

fn chunk_to_u16(chunk: &[u8]) -> u16 {
    // `chunks(2)` yields slices of length 1 or 2 only.
    match chunk.len() {
        1 => u16::from(chunk[0]),
        _ => u16::from_be_bytes([chunk[0], chunk[1]]),
    }
}

/// The first Unicode scalar of a UTF-16BE destination (range start).
fn first_scalar(bytes: &[u8]) -> u32 {
    utf16_be(bytes).chars().next().map_or(0, |c| c as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOUNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin\n\
begincmap\n\
1 begincodespacerange <00> <FF> endcodespacerange\n\
2 beginbfchar\n\
<41> <0041>\n\
<42> <0042>\n\
endbfchar\n\
1 beginbfrange\n\
<43> <45> <0043>\n\
endbfrange\n\
1 beginbfrange\n\
<61> <61> [<0078>]\n\
endbfrange\n\
endcmap end end";

    #[test]
    fn parses_bfchar_and_bfrange() {
        let cm = CMap::parse(TOUNICODE);
        assert_eq!(cm.code_bytes(), 1);
        assert_eq!(cm.lookup(0x41).as_deref(), Some("A"));
        assert_eq!(cm.lookup(0x42).as_deref(), Some("B"));
        // contiguous range 0x43..0x45 → C, D, E
        assert_eq!(cm.lookup(0x43).as_deref(), Some("C"));
        assert_eq!(cm.lookup(0x45).as_deref(), Some("E"));
        // array form: 0x61 → x
        assert_eq!(cm.lookup(0x61).as_deref(), Some("x"));
        assert_eq!(cm.lookup(0x99), None);
    }

    #[test]
    fn two_byte_codespace() {
        let data = b"begincodespacerange <0000> <FFFF> endcodespacerange\n\
1 beginbfchar <0041> <0041> endbfchar";
        let cm = CMap::parse(data);
        assert_eq!(cm.code_bytes(), 2);
        assert_eq!(cm.lookup(0x41).as_deref(), Some("A"));
    }

    #[test]
    fn multi_unit_destination() {
        // A surrogate pair destination decodes to a non-BMP scalar.
        let data = b"1 beginbfchar <01> <D83DDE00> endbfchar";
        let cm = CMap::parse(data);
        assert_eq!(cm.lookup(1), char::from_u32(0x1F600).map(|c| c.to_string()));
    }

    #[test]
    fn default_code_bytes_is_one() {
        assert_eq!(CMap::default().code_bytes(), 1);
        assert!(CMap::default().lookup(0).is_none());
    }

    #[test]
    fn malformed_sections_are_tolerated() {
        // Truncated entries / stray tokens do not panic.
        assert!(CMap::parse(b"beginbfchar <41>").lookup(0x41).is_none());
        assert!(CMap::parse(b"beginbfrange <41> <42>")
            .lookup(0x41)
            .is_none());
        assert!(CMap::parse(b"beginbfrange <41> <42> [ ] endbfrange")
            .lookup(0x41)
            .is_none());
        // An unterminated array destination bails out of the entry.
        assert!(CMap::parse(b"beginbfrange <41> <42> [<0041>")
            .lookup(0x41)
            .is_none());
        // A stray delimiter keyword position is skipped.
        CMap::parse(b"]]] endcmap");
    }

    #[test]
    fn helpers() {
        assert_eq!(be(&[0x12, 0x34]), 0x1234);
        assert_eq!(be(&[1, 2, 3, 4, 5]), 0x01020304); // capped at 4 bytes
        assert_eq!(utf16_be(&[0x00, 0x41]), "A");
        assert_eq!(chunk_to_u16(&[0x00, 0x41]), 0x41);
        assert_eq!(chunk_to_u16(&[0x41]), 0x41);
        assert_eq!(first_scalar(&[0x00, 0x42]), 0x42);
        assert_eq!(first_scalar(&[]), 0);
    }
}
