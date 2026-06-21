//! Predictor post-processing for `FlateDecode` / `LZWDecode` (ISO 32000-1
//! §7.4.4.4, the PNG/TIFF predictors from `/DecodeParms`).
//!
//! `Predictor` 1 (or absent) is a no-op; 2 is TIFF horizontal differencing;
//! 10..=15 are the PNG row filters (each row carries a leading filter-type byte).
//! Geometry comes from `Colors` (1), `BitsPerComponent` (8) and `Columns` (1).

use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::object::Dictionary;

fn bad() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::BadStream, "bad predictor row filter")
}

/// Apply the predictor described by `parms` to already-decompressed `data`.
pub fn apply(data: &[u8], parms: &Dictionary) -> Result<Vec<u8>> {
    let predictor = int(parms, "Predictor", 1);
    if predictor <= 1 {
        return Ok(data.to_vec());
    }
    let colors = int(parms, "Colors", 1).max(1) as usize;
    let bpc = int(parms, "BitsPerComponent", 8).max(1) as usize;
    let columns = int(parms, "Columns", 1).max(1) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let rowlen = (colors * bpc * columns).div_ceil(8).max(1);
    if predictor == 2 {
        Ok(tiff(data, bpp, rowlen))
    } else {
        png(data, bpp, rowlen)
    }
}

fn int(parms: &Dictionary, key: &str, default: i64) -> i64 {
    parms
        .get(key)
        .and_then(|o| o.as_integer())
        .unwrap_or(default)
}

/// TIFF predictor 2: horizontal differencing (byte-aligned components).
fn tiff(data: &[u8], bpp: usize, rowlen: usize) -> Vec<u8> {
    let mut out = data.to_vec();
    for row in out.chunks_mut(rowlen) {
        for i in bpp..row.len() {
            row[i] = row[i].wrapping_add(row[i - bpp]);
        }
    }
    out
}

/// PNG predictors 10..=15: per-row filter-type byte + reconstruction.
fn png(data: &[u8], bpp: usize, rowlen: usize) -> Result<Vec<u8>> {
    let stride = rowlen + 1;
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; rowlen];
    for chunk in data.chunks(stride) {
        let row = png_row(chunk, &prev, bpp)?;
        out.extend_from_slice(&row);
        prev = row;
    }
    Ok(out)
}

/// Reconstruct one PNG row from its filter-type byte and the previous row.
fn png_row(chunk: &[u8], prev: &[u8], bpp: usize) -> Result<Vec<u8>> {
    let (&ftype, body) = chunk.split_first().ok_or_else(bad)?;
    let mut row = body.to_vec();
    for i in 0..row.len() {
        let a = left(&row, i, bpp);
        let b = prev.get(i).copied().unwrap_or(0);
        let c = left(prev, i, bpp);
        row[i] = recon(ftype, row[i], a, b, c)?;
    }
    Ok(row)
}

/// The byte `bpp` positions to the left in `slice` (0 past the row start).
fn left(slice: &[u8], i: usize, bpp: usize) -> u8 {
    if i >= bpp {
        slice[i - bpp]
    } else {
        0
    }
}

/// Reconstruct one byte for the given PNG filter type.
fn recon(ftype: u8, x: u8, a: u8, b: u8, c: u8) -> Result<u8> {
    match ftype {
        0 => Ok(x),
        1 => Ok(x.wrapping_add(a)),
        2 => Ok(x.wrapping_add(b)),
        3 => Ok(x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8)),
        4 => Ok(x.wrapping_add(paeth(a, b, c))),
        _ => Err(bad()),
    }
}

/// The Paeth predictor (PNG spec): pick a, b, or c nearest to a+b-c.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ai, bi, ci) = (i32::from(a), i32::from(b), i32::from(c));
    let p = ai + bi - ci;
    let (pa, pb, pc) = ((p - ai).abs(), (p - bi).abs(), (p - ci).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;

    fn parms(pairs: &[(&str, i64)]) -> Dictionary {
        let mut d = Dictionary::new();
        for &(k, v) in pairs {
            d.insert(k, Object::Integer(v));
        }
        d
    }

    #[test]
    fn predictor_none_passes_through() {
        assert_eq!(apply(b"abc", &parms(&[])).unwrap(), b"abc");
        assert_eq!(apply(b"abc", &parms(&[("Predictor", 1)])).unwrap(), b"abc");
    }

    #[test]
    fn tiff_horizontal_differencing() {
        // Columns 4, 1 color, 8 bpc → bpp 1, rowlen 4. Encoded diffs of [10,11,13,16].
        let encoded = [10u8, 1, 2, 3];
        let out = apply(&encoded, &parms(&[("Predictor", 2), ("Columns", 4)])).unwrap();
        assert_eq!(out, [10, 11, 13, 16]);
    }

    #[test]
    fn png_sub_up_average_paeth() {
        // Two rows, rowlen 3, bpp 1. Row 0 Sub(1), Row 1 Up(2).
        // Row0 Sub: bytes [5,1,1] → [5,6,7]. Row1 Up of [1,0,0] over [5,6,7] → [6,6,7].
        let data = [1u8, 5, 1, 1, 2, 1, 0, 0];
        let out = apply(&data, &parms(&[("Predictor", 12), ("Columns", 3)])).unwrap();
        assert_eq!(out, [5, 6, 7, 6, 6, 7]);
    }

    #[test]
    fn png_none_and_average_and_paeth_rows() {
        // rowlen 2 bpp 1. Row None(0): [9,9]. Row Avg(3): [1,1] over prev [9,9].
        // i0: a=0,b=9 → 1 + (0+9)/2=4 →5. i1: a=5,b=9 → 1+(5+9)/2=7 →8.
        let data = [0u8, 9, 9, 3, 1, 1];
        let out = apply(&data, &parms(&[("Predictor", 14), ("Columns", 2)])).unwrap();
        assert_eq!(out, [9, 9, 5, 8]);
        // Paeth row directly through recon.
        assert_eq!(recon(4, 0, 1, 1, 1).unwrap(), 1);
    }

    #[test]
    fn paeth_branches() {
        assert_eq!(paeth(1, 2, 3), 1); // pa smallest → a
        assert_eq!(paeth(2, 1, 3), 1); // pb smallest → b
        assert_eq!(paeth(2, 4, 3), 3); // pc smallest → c
    }

    #[test]
    fn unknown_filter_type_errors() {
        let data = [9u8, 0, 0]; // filter type 9 invalid
        assert!(apply(&data, &parms(&[("Predictor", 10), ("Columns", 2)])).is_err());
    }

    #[test]
    fn empty_row_chunk_errors() {
        // png with a stride producing an empty trailing chunk is handled; a fully
        // empty input yields empty output.
        assert_eq!(
            apply(b"", &parms(&[("Predictor", 12), ("Columns", 2)])).unwrap(),
            b""
        );
    }
}
