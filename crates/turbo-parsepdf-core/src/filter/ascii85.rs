//! `ASCII85Decode` (ISO 32000-1 §7.4.3): base-85 groups → bytes.
//!
//! Five chars `!`..`u` encode four bytes; `z` is shorthand for four zero bytes;
//! whitespace is ignored; `~>` terminates. A final partial group of `n` chars
//! (2..5) decodes to `n-1` bytes after padding with `u`.

use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::lex::is_whitespace;

fn bad() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::BadStream, "bad ASCII85 data")
}

/// Decode ASCII85 data.
pub fn ascii85_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut group: Vec<u8> = Vec::with_capacity(5);
    for &b in data {
        if !step(&mut out, &mut group, b)? {
            break;
        }
    }
    flush_partial(&mut out, &group)?;
    Ok(out)
}

/// Process one input byte. Returns false at the `~>` terminator.
fn step(out: &mut Vec<u8>, group: &mut Vec<u8>, b: u8) -> Result<bool> {
    match b {
        b'~' => Ok(false),
        b'z' => emit_zero_group(out, group),
        _ if is_whitespace(b) => Ok(true),
        _ => push_char(out, group, b),
    }
}

/// `z` shorthand: only valid at a group boundary → four zero bytes.
fn emit_zero_group(out: &mut Vec<u8>, group: &[u8]) -> Result<bool> {
    if group.is_empty() {
        out.extend_from_slice(&[0, 0, 0, 0]);
        Ok(true)
    } else {
        Err(bad())
    }
}

/// Push one base-85 digit, flushing a full group of five.
fn push_char(out: &mut Vec<u8>, group: &mut Vec<u8>, b: u8) -> Result<bool> {
    let digit = b.checked_sub(b'!').filter(|&d| d <= 84).ok_or_else(bad)?;
    group.push(digit);
    if group.len() == 5 {
        emit_full_group(out, group);
    }
    Ok(true)
}

/// Emit four bytes from a full five-digit group, clearing it.
fn emit_full_group(out: &mut Vec<u8>, group: &mut Vec<u8>) {
    let value = group.iter().fold(0u32, |acc, &d| {
        acc.wrapping_mul(85).wrapping_add(u32::from(d))
    });
    out.extend_from_slice(&value.to_be_bytes());
    group.clear();
}

/// Flush a trailing partial group (`n` digits → `n-1` bytes).
fn flush_partial(out: &mut Vec<u8>, group: &[u8]) -> Result<()> {
    if group.is_empty() {
        return Ok(());
    }
    if group.len() == 1 {
        return Err(bad());
    }
    let mut padded = group.to_vec();
    padded.resize(5, 84); // pad with 'u' (84)
    let value = padded.iter().fold(0u32, |acc, &d| {
        acc.wrapping_mul(85).wrapping_add(u32::from(d))
    });
    out.extend_from_slice(&value.to_be_bytes()[..group.len() - 1]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_roundtrip(plain: &[u8], encoded: &[u8]) {
        assert_eq!(ascii85_decode(encoded).unwrap(), plain);
    }

    #[test]
    fn decodes_full_groups() {
        // "Man " encodes to "9jqo^" in ASCII85.
        enc_roundtrip(b"Man ", b"9jqo^~>");
    }

    #[test]
    fn z_shorthand_is_four_zeros() {
        assert_eq!(ascii85_decode(b"z~>").unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn whitespace_ignored_and_partial_group() {
        // Single char 'M' → partial group "9j" style; use a known partial.
        // "M" (1 byte) encodes to "9`" (2 chars → 1 byte).
        let out = ascii85_decode(b"9 `~>").unwrap();
        assert_eq!(out, b"M");
    }

    #[test]
    fn terminator_without_tilde_gt_ok() {
        // No explicit ~> still flushes.
        assert_eq!(ascii85_decode(b"9jqo^").unwrap(), b"Man ");
    }

    #[test]
    fn errors() {
        assert!(ascii85_decode(b"z9~>").is_err()); // z mid-group? no — z at start ok
        assert!(ascii85_decode(b"9z~>").is_err()); // z inside a group
        assert!(ascii85_decode(b"9~>").is_err()); // 1-char partial group invalid
        assert!(ascii85_decode(b"\xff~>").is_err()); // digit out of range
    }
}
