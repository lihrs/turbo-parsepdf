//! Content-stream tokenizer (ISO 32000-1 §7.8.2).
//!
//! A page content stream is postfix: zero or more operands (COS objects) follow
//! by an operator keyword (`Tj`, `Td`, `re`, `cm`, …). [`parse_content`] splits
//! the byte stream into a flat list of [`Op`]s, reusing the COS value parser for
//! operands and reading bare keywords as operators. Inline images (`BI … ID …
//! EI`) carry raw binary that is not COS, so they are recognized and skipped.

use crate::cos::parse_object;
use crate::lex::Lexer;
use crate::object::Object;

/// A content-stream operator name, stored inline (every PDF operator is ≤3
/// bytes), so tokenizing a content stream allocates nothing per operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operator {
    buf: [u8; 4],
    len: u8,
}

impl Operator {
    /// Build an operator from its keyword bytes (truncated to 4 — longer runs are
    /// never valid operators and fall through to the interpreter's no-op arm).
    fn from_bytes(bytes: &[u8]) -> Self {
        let len = bytes.len().min(4);
        let mut buf = [0u8; 4];
        buf[..len].copy_from_slice(&bytes[..len]);
        Operator {
            buf,
            len: len as u8,
        }
    }

    /// The operator as a string slice (empty for non-UTF-8 bytes).
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

impl PartialEq<&str> for Operator {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// One content-stream operation: an operator and the operands that preceded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub operator: Operator,
    pub operands: Vec<Object>,
}

/// Tokenize a (decoded) content stream into a list of operations.
pub fn parse_content(data: &[u8]) -> Vec<Op> {
    let mut lx = Lexer::new(data);
    let mut operands: Vec<Object> = Vec::new();
    let mut ops: Vec<Op> = Vec::new();
    loop {
        lx.skip_whitespace();
        match lx.peek() {
            None => return ops,
            Some(b) if is_operand_start(b) => {
                if !push_operand(&mut lx, &mut operands) {
                    return ops;
                }
            }
            _ => read_operator(&mut lx, &mut operands, &mut ops),
        }
    }
}

/// Read an operator and move the pending operands into an [`Op`] (no clone): the
/// moved-out buffer keeps its exact length and the scratch is replaced with a
/// pre-sized one, so no operand `Vec` grows from zero.
fn read_operator(lx: &mut Lexer, operands: &mut Vec<Object>, ops: &mut Vec<Op>) {
    let Some(kw) = lx.read_keyword() else {
        lx.bump();
        operands.clear();
        return;
    };
    let operator = Operator::from_bytes(kw);
    if operator == "BI" {
        skip_inline_image(lx);
        operands.clear();
        return;
    }
    let taken = std::mem::replace(operands, Vec::with_capacity(8));
    ops.push(Op {
        operator,
        operands: taken,
    });
}

/// True for bytes that begin a COS operand (number, name, string, array, dict).
fn is_operand_start(b: u8) -> bool {
    matches!(
        b,
        b'/' | b'(' | b'<' | b'[' | b'0'..=b'9' | b'+' | b'-' | b'.'
    )
}

/// Parse one operand; returns false on a parse error (stop tokenizing).
fn push_operand(lx: &mut Lexer, operands: &mut Vec<Object>) -> bool {
    match parse_object(lx) {
        Ok(obj) => {
            operands.push(obj);
            true
        }
        Err(_) => false,
    }
}

/// Skip an inline image's binary data through its `EI` terminator.
fn skip_inline_image(lx: &mut Lexer) {
    let data = lx.data();
    let from = lx.pos();
    let rel = data[from..]
        .windows(2)
        .position(|w| w == b"EI")
        .map(|i| i + 2);
    lx.seek(from + rel.unwrap_or(data.len() - from));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(data: &[u8]) -> Vec<Op> {
        parse_content(data)
    }

    #[test]
    fn simple_text_block() {
        let got = ops(b"BT /F1 12 Tf 100 700 Td (Hello) Tj ET");
        let names: Vec<&str> = got.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["BT", "Tf", "Td", "Tj", "ET"]);
        // Tf carries the font name + size.
        let tf = &got[1];
        assert_eq!(tf.operands[0].as_name(), Some("F1"));
        assert_eq!(tf.operands[1].as_integer(), Some(12));
        // Tj carries the literal string.
        assert_eq!(got[3].operands[0].as_string(), Some(&b"Hello"[..]));
    }

    #[test]
    fn tj_array_and_negatives() {
        let got = ops(b"[(A) -250 (B)] TJ");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].operator, "TJ");
        assert_eq!(got[0].operands[0].as_array().unwrap().len(), 3);
    }

    #[test]
    fn quote_operators_are_regular() {
        let got = ops(b"(x) ' 1 2 (y) \"");
        let names: Vec<&str> = got.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["'", "\""]);
    }

    #[test]
    fn star_and_multichar_operators() {
        let got = ops(b"0 0 1 1 re T* b*");
        let names: Vec<&str> = got.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["re", "T*", "b*"]);
    }

    #[test]
    fn inline_image_is_skipped() {
        let got = ops(b"q BI /W 2 /H 2 ID \x00\x01\x02\x03 EI Q (after) Tj");
        let names: Vec<&str> = got.iter().map(|o| o.operator.as_str()).collect();
        // BI..EI dropped; q, Q, and the trailing Tj survive.
        assert_eq!(names, ["q", "Q", "Tj"]);
        assert_eq!(
            got.last().unwrap().operands[0].as_string(),
            Some(&b"after"[..])
        );
    }

    #[test]
    fn inline_image_without_ei_consumes_rest() {
        let got = ops(b"BI /W 1 ID \x00\x00\x00");
        assert!(got.is_empty());
    }

    #[test]
    fn malformed_operand_stops() {
        // An unterminated string ends the stream cleanly.
        let got = ops(b"(good) Tj (unterminated");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].operator, "Tj");
    }

    #[test]
    fn stray_delimiter_makes_progress() {
        // A lone ')' starts no operand and is no operator; tokenizer skips it.
        let got = ops(b") (ok) Tj");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].operator, "Tj");
    }

    #[test]
    fn empty_stream() {
        assert!(ops(b"   ").is_empty());
    }
}
