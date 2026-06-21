//! Stream filter dispatch (ISO 32000-1 §7.4).
//!
//! A stream's `/Filter` is a single name or an array of names applied in order;
//! [`decode_stream`] walks that chain, applying each filter and the matching
//! `/DecodeParms` predictor. Supported: `FlateDecode`, `LZWDecode`,
//! `ASCII85Decode`, `ASCIIHexDecode`, `RunLengthDecode`, with PNG/TIFF predictors
//! for the two compression filters. Image filters (`DCTDecode`/`JPXDecode`/
//! `CCITTFaxDecode`/`JBIG2Decode`) are passed through untouched — the image
//! extractor takes their bytes directly; they are never decoded to a raster here.

pub mod ascii85;
pub mod asciihex;
pub mod inflate;
pub mod lzw;
pub mod predictor;
pub mod runlength;

use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::object::{Dictionary, Object};

pub use ascii85::ascii85_decode;
pub use asciihex::ascii_hex_decode;
pub use inflate::{flate_decode, inflate};
pub use lzw::lzw_decode;
pub use runlength::run_length_decode;

/// Decode a stream body by applying its `/Filter` chain (+ predictors) to `raw`.
pub fn decode_stream(dict: &Dictionary, raw: &[u8]) -> Result<Vec<u8>> {
    let filters = filter_names(dict);
    let parms = parms_list(dict, filters.len());
    let mut data = raw.to_vec();
    for (name, parm) in filters.iter().zip(parms.iter()) {
        data = apply_one(name, parm.as_ref(), &data)?;
    }
    Ok(data)
}

/// Collect the ordered filter names from `/Filter` / `/F` (name, array, absent).
fn filter_names(dict: &Dictionary) -> Vec<String> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(items)) => items.iter().filter_map(name_of).collect(),
        _ => Vec::new(),
    }
}

fn name_of(obj: &Object) -> Option<String> {
    obj.as_name().map(str::to_owned)
}

/// Collect the per-filter `/DecodeParms` (`/DP`): single dict, array, or absent.
fn parms_list(dict: &Dictionary, n: usize) -> Vec<Option<Dictionary>> {
    match dict.get("DecodeParms").or_else(|| dict.get("DP")) {
        Some(Object::Dictionary(d)) => one_then_none(d.clone(), n),
        Some(Object::Array(items)) => items.iter().map(parm_of).collect(),
        _ => vec![None; n],
    }
}

fn one_then_none(d: Dictionary, n: usize) -> Vec<Option<Dictionary>> {
    let mut v = vec![None; n];
    if let Some(first) = v.first_mut() {
        *first = Some(d);
    }
    v
}

fn parm_of(obj: &Object) -> Option<Dictionary> {
    obj.as_dict().cloned()
}

/// Apply a single named filter (and its predictor for compression filters).
fn apply_one(name: &str, parms: Option<&Dictionary>, data: &[u8]) -> Result<Vec<u8>> {
    match name {
        "FlateDecode" | "Fl" => predictor_maybe(parms, flate_decode(data)?),
        "LZWDecode" | "LZW" => predictor_maybe(parms, lzw_decode(data, early_change(parms))?),
        "ASCII85Decode" | "A85" => ascii85_decode(data),
        "ASCIIHexDecode" | "AHx" => ascii_hex_decode(data),
        "RunLengthDecode" | "RL" => run_length_decode(data),
        "DCTDecode" | "JPXDecode" | "CCITTFaxDecode" | "JBIG2Decode" => Ok(data.to_vec()),
        other => Err(unsupported(other)),
    }
}

/// Apply the `/DecodeParms` predictor when one is present.
fn predictor_maybe(parms: Option<&Dictionary>, data: Vec<u8>) -> Result<Vec<u8>> {
    match parms {
        Some(p) => predictor::apply(&data, p),
        None => Ok(data),
    }
}

/// LZW `/EarlyChange` (default 1 → true).
fn early_change(parms: Option<&Dictionary>) -> bool {
    parms
        .and_then(|p| p.get("EarlyChange"))
        .and_then(|o| o.as_integer())
        .map(|n| n != 0)
        .unwrap_or(true)
}

fn unsupported(name: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(
        ErrorCode::Unsupported,
        format!("unsupported filter: {name}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(filter: Object, parms: Option<Object>, raw: &[u8]) -> Result<Vec<u8>> {
        let mut d = Dictionary::new();
        d.insert("Filter", filter);
        if let Some(p) = parms {
            d.insert("DecodeParms", p);
        }
        decode_stream(&d, raw)
    }

    const HI_ZLIB: [u8; 13] = [
        0x78, 0x9c, 0x01, 0x02, 0x00, 0xfd, 0xff, b'H', b'i', 0x00, 0x62, 0x00, 0x62,
    ];

    #[test]
    fn no_filter_returns_raw() {
        assert_eq!(
            decode_stream(&Dictionary::new(), b"plain").unwrap(),
            b"plain"
        );
    }

    #[test]
    fn flate_and_abbreviation() {
        assert_eq!(
            stream(Object::Name("FlateDecode".into()), None, &HI_ZLIB).unwrap(),
            b"Hi"
        );
        assert_eq!(
            stream(Object::Name("Fl".into()), None, &HI_ZLIB).unwrap(),
            b"Hi"
        );
    }

    #[test]
    fn ascii_filters() {
        assert_eq!(
            stream(Object::Name("ASCIIHexDecode".into()), None, b"4869>").unwrap(),
            b"Hi"
        );
        assert_eq!(
            stream(Object::Name("ASCII85Decode".into()), None, b"9jqo^~>").unwrap(),
            b"Man "
        );
        assert_eq!(
            stream(
                Object::Name("RunLengthDecode".into()),
                None,
                &[2, b'a', b'b', b'c', 128]
            )
            .unwrap(),
            b"abc"
        );
    }

    #[test]
    fn image_filters_pass_through() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];
        assert_eq!(
            stream(Object::Name("DCTDecode".into()), None, &jpeg).unwrap(),
            jpeg
        );
    }

    #[test]
    fn filter_array_with_parms_array() {
        // A85 then Flate, with parallel DecodeParms (null for A85, predictor for Flate).
        let chain = Object::Array(vec![
            Object::Name("ASCIIHexDecode".into()),
            Object::Name("FlateDecode".into()),
        ]);
        // ASCIIHex of the zlib "Hi" stream, then Flate.
        let hex: String = HI_ZLIB.iter().map(|b| format!("{b:02X}")).collect();
        let parms = Object::Array(vec![Object::Null, Object::Null]);
        let mut hexbytes = hex.into_bytes();
        hexbytes.push(b'>');
        assert_eq!(stream(chain, Some(parms), &hexbytes).unwrap(), b"Hi");
    }

    #[test]
    fn flate_with_png_predictor() {
        // Build a predictor-12 payload, deflate it, then decode_stream must invert.
        // Row data (Up filter, rowlen 2): [0,9,9, 2,1,1] → decoded [9,9,10,10].
        let payload = [0u8, 9, 9, 2, 1, 1];
        let deflated = deflate(&payload);
        let mut parms = Dictionary::new();
        parms.insert("Predictor", Object::Integer(12));
        parms.insert("Columns", Object::Integer(2));
        let got = stream(
            Object::Name("FlateDecode".into()),
            Some(Object::Dictionary(parms)),
            &deflated,
        )
        .unwrap();
        assert_eq!(got, [9, 9, 10, 10]);
    }

    #[test]
    fn lzw_round_trip_via_dispatch() {
        // Encode with the lzw test encoder is internal; instead feed a known stream.
        // Use flate path already covered; here assert early_change default + LZW name.
        // A single EOD-only LZW stream (clear, eod) → empty.
        // clear=256 (9 bits) 100000000, eod=257 100000001 → packed.
        let lzw_empty = encode_lzw_empty();
        assert_eq!(
            stream(Object::Name("LZWDecode".into()), None, &lzw_empty).unwrap(),
            b""
        );
    }

    #[test]
    fn unknown_filter_errors() {
        let e = stream(Object::Name("Bogus".into()), None, b"x").unwrap_err();
        assert_eq!(e.code, ErrorCode::Unsupported);
    }

    #[test]
    fn early_change_flag_read() {
        let mut p = Dictionary::new();
        p.insert("EarlyChange", Object::Integer(0));
        assert!(!early_change(Some(&p)));
        assert!(early_change(None));
    }

    #[test]
    fn non_name_filter_entry_ignored() {
        let chain = Object::Array(vec![Object::Integer(1)]);
        assert_eq!(stream(chain, None, b"raw").unwrap(), b"raw");
    }

    #[test]
    fn parm_helpers() {
        assert!(parm_of(&Object::Null).is_none());
        assert!(parm_of(&Object::Dictionary(Dictionary::new())).is_some());
        assert_eq!(one_then_none(Dictionary::new(), 0).len(), 0);
    }

    // --- tiny test helpers ---

    fn deflate(data: &[u8]) -> Vec<u8> {
        // Minimal stored-block zlib wrapper (BFINAL stored), good enough for tests.
        let mut out = vec![0x78, 0x01];
        out.push(0x01); // BFINAL=1, stored
        let len = data.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // dummy adler
        out
    }

    fn encode_lzw_empty() -> Vec<u8> {
        // clear (256) then eod (257), each 9 bits, MSB-first.
        // 256 = 1_0000_0000, 257 = 1_0000_0001 → 18 bits → 3 bytes.
        let mut bits = Vec::new();
        for code in [256u16, 257] {
            for i in (0..9).rev() {
                bits.push(((code >> i) & 1) as u8);
            }
        }
        let mut out = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            out[i / 8] |= b << (7 - (i % 8));
        }
        out
    }
}
