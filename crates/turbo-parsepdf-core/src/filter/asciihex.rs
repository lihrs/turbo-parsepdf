//! `ASCIIHexDecode` (ISO 32000-1 §7.4.2): hex digits → bytes, `>` ends the data.

use crate::cos::hex_val;
use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::lex::is_whitespace;

fn bad() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::BadStream, "bad ASCIIHex data")
}

/// Decode ASCII-hex data. Whitespace is ignored; an odd trailing nibble is
/// padded with zero; `>` terminates.
pub fn ascii_hex_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        accumulate(&mut out, &mut high, b)?;
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Ok(out)
}

fn accumulate(out: &mut Vec<u8>, high: &mut Option<u8>, b: u8) -> Result<()> {
    if is_whitespace(b) {
        return Ok(());
    }
    let nib = hex_val(b).ok_or_else(bad)?;
    match high.take() {
        Some(h) => out.push((h << 4) | nib),
        None => *high = Some(nib),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_whitespace_and_terminator() {
        assert_eq!(ascii_hex_decode(b"48 65 6C 6C 6F>extra").unwrap(), b"Hello");
    }

    #[test]
    fn odd_nibble_is_padded() {
        assert_eq!(ascii_hex_decode(b"41A>").unwrap(), vec![0x41, 0xa0]);
    }

    #[test]
    fn no_terminator_is_fine() {
        assert_eq!(ascii_hex_decode(b"4869").unwrap(), b"Hi");
    }

    #[test]
    fn bad_digit_errors() {
        assert_eq!(
            ascii_hex_decode(b"4G").unwrap_err().code,
            ErrorCode::BadStream
        );
    }
}
