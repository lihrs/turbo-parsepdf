//! `RunLengthDecode` (ISO 32000-1 §7.4.5): byte-oriented run-length coding.
//!
//! Each length byte `L`: `0..=127` → copy the next `L+1` bytes literally;
//! `129..=255` → repeat the next byte `257-L` times; `128` is end-of-data.

use crate::error::{ErrorCode, Result, TurboParsePdfError};

fn bad() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::BadStream, "truncated RunLength data")
}

/// Decode RunLength data.
pub fn run_length_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let length = data[i];
        i += 1;
        if length == 128 {
            break;
        }
        i = run(&mut out, data, i, length)?;
    }
    Ok(out)
}

/// Apply one run starting at `i`; return the next input index.
fn run(out: &mut Vec<u8>, data: &[u8], i: usize, length: u8) -> Result<usize> {
    if length < 128 {
        copy_literal(out, data, i, length as usize + 1)
    } else {
        repeat_byte(out, data, i, 257 - length as usize)
    }
}

/// Copy `count` literal bytes.
fn copy_literal(out: &mut Vec<u8>, data: &[u8], i: usize, count: usize) -> Result<usize> {
    let end = i + count;
    let slice = data.get(i..end).ok_or_else(bad)?;
    out.extend_from_slice(slice);
    Ok(end)
}

/// Repeat one byte `count` times.
fn repeat_byte(out: &mut Vec<u8>, data: &[u8], i: usize, count: usize) -> Result<usize> {
    let &b = data.get(i).ok_or_else(bad)?;
    out.resize(out.len() + count, b);
    Ok(i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_run() {
        // length 2 → copy 3 literal bytes "abc".
        assert_eq!(run_length_decode(&[2, b'a', b'b', b'c']).unwrap(), b"abc");
    }

    #[test]
    fn repeat_run() {
        // length 254 → repeat next byte 257-254 = 3 times.
        assert_eq!(run_length_decode(&[254, b'x']).unwrap(), b"xxx");
    }

    #[test]
    fn eod_stops() {
        assert_eq!(run_length_decode(&[0, b'A', 128, 0, b'Z']).unwrap(), b"A");
    }

    #[test]
    fn mixed() {
        let data = [1, b'h', b'i', 255, b'!'];
        // literal 2 "hi", then repeat 257-255=2 of '!'.
        assert_eq!(run_length_decode(&data).unwrap(), b"hi!!");
    }

    #[test]
    fn truncated_errors() {
        assert!(run_length_decode(&[5, b'a']).is_err()); // wants 6 literal bytes
        assert!(run_length_decode(&[200]).is_err()); // wants a byte to repeat
    }
}
