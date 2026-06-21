//! `LZWDecode` (ISO 32000-1 §7.4.4): variable-width (9–12 bit), MSB-first LZW.
//!
//! Code 256 clears the table (back to 9-bit width), 257 is end-of-data, 0..=255
//! are single-byte roots, and new strings are assigned from 258. PDF defaults to
//! `EarlyChange = 1` (widen one code early); the TIFF predictor that often
//! follows is applied separately.

use crate::error::{ErrorCode, Result, TurboParsePdfError};

const CLEAR: u16 = 256;
const EOD: u16 = 257;
const FIRST: usize = 258;

fn bad() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::BadStream, "bad LZW data")
}

/// Decode an LZW stream. `early_change` matches the `/DecodeParms` flag (default
/// true in PDF).
pub fn lzw_decode(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    let mut reader = MsbReader::new(data);
    let mut table = Table::new(early_change);
    let mut out = Vec::with_capacity(data.len() * 3);
    let mut prev: Option<u16> = None;
    while let Some(code) = reader.read(table.width) {
        let code = code as u16;
        if code == EOD {
            break;
        }
        handle_code(&mut table, &mut out, &mut prev, code)?;
    }
    Ok(out)
}

/// Process one code: clear, or emit + extend the table.
fn handle_code(
    table: &mut Table,
    out: &mut Vec<u8>,
    prev: &mut Option<u16>,
    code: u16,
) -> Result<()> {
    if code == CLEAR {
        table.reset();
        *prev = None;
        return Ok(());
    }
    let entry = resolve_entry(table, *prev, code)?;
    out.extend_from_slice(&entry);
    extend_table(table, *prev, &entry);
    *prev = Some(code);
    Ok(())
}

/// The string for `code`, handling the KwKwK case (code not yet defined).
fn resolve_entry(table: &Table, prev: Option<u16>, code: u16) -> Result<Vec<u8>> {
    if let Some(existing) = table.get(code) {
        return Ok(existing.to_vec());
    }
    let p = table.get(prev.ok_or_else(bad)?).ok_or_else(bad)?;
    let mut entry = p.to_vec();
    entry.push(p[0]);
    Ok(entry)
}

/// Add `prev_string + entry[0]` as the next table string.
fn extend_table(table: &mut Table, prev: Option<u16>, entry: &[u8]) {
    if let Some(p) = prev {
        if let Some(pe) = table.get(p) {
            let mut new = pe.to_vec();
            new.push(entry[0]);
            table.push(new);
        }
    }
}

/// MSB-first bit reader.
struct MsbReader<'a> {
    data: &'a [u8],
    bitpos: usize,
}

impl<'a> MsbReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        MsbReader { data, bitpos: 0 }
    }

    /// Read `width` bits MSB-first, or `None` when the data is exhausted.
    fn read(&mut self, width: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..width {
            let byte = *self.data.get(self.bitpos / 8)?;
            let bit = (byte >> (7 - (self.bitpos % 8))) & 1;
            value = (value << 1) | u32::from(bit);
            self.bitpos += 1;
        }
        Some(value)
    }
}

/// The growing string table with its current code width.
struct Table {
    entries: Vec<Vec<u8>>,
    width: u32,
    early_change: bool,
}

impl Table {
    fn new(early_change: bool) -> Self {
        let mut t = Table {
            entries: Vec::with_capacity(512),
            width: 9,
            early_change,
        };
        t.reset();
        t
    }

    /// Reset to the 258 reserved entries and 9-bit width.
    fn reset(&mut self) {
        self.entries.clear();
        for b in 0u16..=255 {
            self.entries.push(vec![b as u8]);
        }
        self.entries.push(Vec::new()); // 256 CLEAR placeholder
        self.entries.push(Vec::new()); // 257 EOD placeholder
        self.width = 9;
    }

    fn get(&self, code: u16) -> Option<&[u8]> {
        match code as usize {
            i if i < FIRST => Some(&self.entries[i]),
            i => self.entries.get(i).map(Vec::as_slice),
        }
    }

    /// Append a new string and widen the code length when the next index needs
    /// it. The decoder builds its table one entry behind the encoder, so it must
    /// anticipate by one (`len + 1`) to switch width on the same code position.
    fn push(&mut self, entry: Vec<u8>) {
        self.entries.push(entry);
        let early = usize::from(self.early_change);
        let threshold = (1usize << self.width) - early;
        if self.entries.len() + 1 >= threshold && self.width < 12 {
            self.width += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A reference MSB-first LZW encoder mirroring the decoder, for round-trips.
    struct Writer {
        out: Vec<u8>,
        bitpos: usize,
    }
    impl Writer {
        fn new() -> Self {
            Writer {
                out: Vec::new(),
                bitpos: 0,
            }
        }
        fn write(&mut self, code: u16, width: u32) {
            for i in (0..width).rev() {
                let bit = ((code >> i) & 1) as u8;
                let byte = self.bitpos / 8;
                if byte >= self.out.len() {
                    self.out.push(0);
                }
                self.out[byte] |= bit << (7 - (self.bitpos % 8));
                self.bitpos += 1;
            }
        }
    }

    fn enc_bump_width(width: u32, next: usize, early: usize) -> u32 {
        if next >= (1usize << width) - early && width < 12 {
            width + 1
        } else {
            width
        }
    }

    fn encode(data: &[u8], early_change: bool) -> Vec<u8> {
        use std::collections::HashMap;
        let mut w = Writer::new();
        let mut dict: HashMap<Vec<u8>, u16> = HashMap::new();
        let reset = |d: &mut HashMap<Vec<u8>, u16>| {
            d.clear();
            for b in 0u16..=255 {
                d.insert(vec![b as u8], b);
            }
        };
        reset(&mut dict);
        let mut next = FIRST as u16;
        let mut width = 9u32;
        let early = usize::from(early_change);
        w.write(CLEAR, width);
        let mut current: Vec<u8> = Vec::new();
        for &b in data {
            let mut probe = current.clone();
            probe.push(b);
            if dict.contains_key(&probe) {
                current = probe;
            } else {
                w.write(dict[&current], width);
                dict.insert(probe, next);
                next += 1;
                width = enc_bump_width(width, next as usize, early);
                current = vec![b];
            }
        }
        if !current.is_empty() {
            w.write(dict[&current], width);
        }
        w.write(EOD, width);
        w.out
    }

    fn roundtrip(plain: &[u8], early: bool) {
        let encoded = encode(plain, early);
        assert_eq!(lzw_decode(&encoded, early).unwrap(), plain);
    }

    #[test]
    fn short_string() {
        roundtrip(b"TOBEORNOTTOBEORTOBEORNOT", true);
    }

    #[test]
    fn kwkwk_case() {
        // Repeated runs trigger the "code not yet in table" path.
        roundtrip(b"AAAAAAAAAAAAAAAAAA", true);
    }

    #[test]
    fn early_change_off() {
        roundtrip(b"the quick brown fox the quick brown fox jumped", false);
    }

    #[test]
    fn width_growth_over_512_codes() {
        // A long, varied input forces the table past 9- and 10-bit widths.
        let mut big = Vec::new();
        for i in 0u32..2000 {
            big.push((i % 251) as u8);
        }
        roundtrip(&big, true);
    }

    #[test]
    fn empty_and_eod() {
        // A bare EOD code decodes to nothing.
        assert_eq!(lzw_decode(&encode(b"", true), true).unwrap(), b"");
    }

    #[test]
    fn corrupt_first_code_errors() {
        // First code 258 is undefined with no prev string → KwKwK has no base.
        // 258 = 0b1_0000_0010, packed MSB-first → 0x81, 0x00.
        assert!(lzw_decode(&[0x81, 0x00], true).is_err());
    }
}
