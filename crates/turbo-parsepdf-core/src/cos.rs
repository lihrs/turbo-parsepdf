//! Recursive-descent parser for COS *values* (ISO 32000-1 §7.3).
//!
//! Given a [`Lexer`] positioned at the start of an object, [`parse_object`]
//! produces one [`Object`]: a number, string, name, array, dictionary, boolean,
//! null, or an indirect reference (`n g R`). Streams are *not* handled here — a
//! stream is only ever the value of a top-level indirect object, where the
//! `/Length` is known, so [`crate::resolver`] assembles it. Every function is
//! kept under the cc-6 gate by pushing each sub-shape into its own helper.

use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::lex::{is_whitespace, Lexer};
use crate::object::{Dictionary, ObjRef, Object};

fn err(code: ErrorCode, msg: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(code, msg)
}

fn eof() -> TurboParsePdfError {
    err(ErrorCode::UnexpectedEof, "input ended mid-object")
}

/// Parse one COS value at the lexer's current position (after skipping leading
/// whitespace/comments).
pub fn parse_object(lx: &mut Lexer) -> Result<Object> {
    lx.skip_whitespace();
    let b = lx.peek().ok_or_else(eof)?;
    match b {
        b'/' => parse_name(lx),
        b'(' => parse_literal_string(lx),
        b'<' => parse_angle(lx),
        b'[' => parse_array(lx),
        b'0'..=b'9' | b'+' | b'-' | b'.' => parse_number_or_ref(lx),
        _ => parse_keyword_literal(lx),
    }
}

/// Dispatch the two `<`-prefixed shapes: `<<` a dictionary, `<` a hex string.
fn parse_angle(lx: &mut Lexer) -> Result<Object> {
    if lx.peek_at(1) == Some(b'<') {
        Ok(Object::Dictionary(parse_dictionary(lx)?))
    } else {
        parse_hex_string(lx)
    }
}

/// Parse a name token (`/Foo`), decoding `#xx` hex escapes.
fn parse_name(lx: &mut Lexer) -> Result<Object> {
    lx.bump(); // consume '/'
    let mut bytes = Vec::new();
    while lx.peek().is_some_and(crate::lex::is_regular) {
        let b = lx.bump().ok_or_else(eof)?;
        push_name_byte(lx, &mut bytes, b)?;
    }
    Ok(Object::Name(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Append one name byte, decoding a `#xx` escape when present.
fn push_name_byte(lx: &mut Lexer, out: &mut Vec<u8>, b: u8) -> Result<()> {
    if b == b'#' {
        out.push(read_name_hex(lx)?);
    } else {
        out.push(b);
    }
    Ok(())
}

/// Decode the two hex digits of a `#xx` name escape into one byte.
fn read_name_hex(lx: &mut Lexer) -> Result<u8> {
    let hi = hex_val(lx.bump().ok_or_else(eof)?).ok_or_else(bad_name)?;
    let lo = hex_val(lx.bump().ok_or_else(eof)?).ok_or_else(bad_name)?;
    Ok((hi << 4) | lo)
}

fn bad_name() -> TurboParsePdfError {
    err(ErrorCode::UnexpectedToken, "bad #xx escape in name")
}

/// Parse a literal string `(...)` with balanced parens and backslash escapes.
fn parse_literal_string(lx: &mut Lexer) -> Result<Object> {
    lx.bump(); // consume '('
    let mut out = Vec::new();
    let mut depth = 1usize;
    while let Some(b) = lx.bump() {
        match b {
            b'\\' => handle_escape(lx, &mut out)?,
            b'(' => string_open_paren(&mut out, &mut depth),
            b')' => {
                if string_close_paren(&mut out, &mut depth) {
                    return Ok(Object::String(out));
                }
            }
            _ => out.push(b),
        }
    }
    Err(eof())
}

/// `(` inside a string: increase nesting and keep the literal byte.
fn string_open_paren(out: &mut Vec<u8>, depth: &mut usize) {
    *depth += 1;
    out.push(b'(');
}

/// `)` inside a string: decrease nesting, keeping the literal byte for an inner
/// paren. Returns true only for the outermost `)`, which closes the string.
fn string_close_paren(out: &mut Vec<u8>, depth: &mut usize) -> bool {
    *depth -= 1;
    if *depth > 0 {
        out.push(b')');
    }
    *depth == 0
}

/// Decode one backslash escape inside a literal string.
fn handle_escape(lx: &mut Lexer, out: &mut Vec<u8>) -> Result<()> {
    let b = lx.bump().ok_or_else(eof)?;
    match b {
        b'n' => out.push(b'\n'),
        b'r' => out.push(b'\r'),
        b't' => out.push(b'\t'),
        b'b' => out.push(0x08),
        b'f' => out.push(0x0c),
        b'\r' => skip_line_continuation(lx),
        b'\n' => {}
        b'0'..=b'7' => read_octal(lx, out, b),
        _ => out.push(b),
    }
    Ok(())
}

/// A backslash before CR (optionally CRLF) is a line continuation: emit nothing.
fn skip_line_continuation(lx: &mut Lexer) {
    if lx.peek() == Some(b'\n') {
        lx.bump();
    }
}

/// Read up to two further octal digits after the first, pushing the byte.
fn read_octal(lx: &mut Lexer, out: &mut Vec<u8>, first: u8) {
    let mut val = u16::from(first - b'0');
    for _ in 0..2 {
        match lx.peek() {
            Some(d @ b'0'..=b'7') => val = push_octal_digit(lx, val, d),
            _ => break,
        }
    }
    out.push(val as u8);
}

fn push_octal_digit(lx: &mut Lexer, val: u16, digit: u8) -> u16 {
    lx.bump();
    val * 8 + u16::from(digit - b'0')
}

/// Parse a hex string `<...>` (whitespace ignored, odd final nibble padded).
fn parse_hex_string(lx: &mut Lexer) -> Result<Object> {
    lx.bump(); // consume '<'
    let mut out = Vec::new();
    let mut high: Option<u8> = None;
    while let Some(b) = lx.bump() {
        if b == b'>' {
            flush_nibble(&mut out, high);
            return Ok(Object::String(out));
        }
        accumulate_hex(&mut out, &mut high, b)?;
    }
    Err(eof())
}

/// Add one hex digit to the pending byte (whitespace is skipped).
fn accumulate_hex(out: &mut Vec<u8>, high: &mut Option<u8>, b: u8) -> Result<()> {
    if is_whitespace(b) {
        return Ok(());
    }
    let nib = hex_val(b).ok_or_else(|| err(ErrorCode::UnexpectedToken, "bad hex digit"))?;
    match high.take() {
        Some(h) => out.push((h << 4) | nib),
        None => *high = Some(nib),
    }
    Ok(())
}

/// Flush a dangling high nibble (odd digit count → low nibble is zero).
fn flush_nibble(out: &mut Vec<u8>, high: Option<u8>) {
    if let Some(h) = high {
        out.push(h << 4);
    }
}

/// Parse an array `[ obj obj ... ]`.
fn parse_array(lx: &mut Lexer) -> Result<Object> {
    lx.bump(); // consume '['
    let mut items = Vec::new();
    loop {
        lx.skip_whitespace();
        match lx.peek() {
            None => return Err(eof()),
            Some(b']') => return finish_array(lx, items),
            _ => items.push(parse_object(lx)?),
        }
    }
}

fn finish_array(lx: &mut Lexer, items: Vec<Object>) -> Result<Object> {
    lx.bump(); // consume ']'
    Ok(Object::Array(items))
}

/// Parse a dictionary `<< /Key value ... >>`.
pub fn parse_dictionary(lx: &mut Lexer) -> Result<Dictionary> {
    lx.advance(2); // consume '<<'
    let mut dict = Dictionary::new();
    loop {
        lx.skip_whitespace();
        if close_dictionary(lx)? {
            return Ok(dict);
        }
        read_dict_entry(lx, &mut dict)?;
    }
}

/// Read one `/Key value` pair into the dictionary.
fn read_dict_entry(lx: &mut Lexer, dict: &mut Dictionary) -> Result<()> {
    let key = dict_key(lx)?;
    let value = parse_object(lx)?;
    dict.insert(key, value);
    Ok(())
}

/// At a dictionary boundary: consume `>>` and report closure.
fn close_dictionary(lx: &mut Lexer) -> Result<bool> {
    match lx.peek() {
        Some(b'>') => {
            lx.advance(2);
            Ok(true)
        }
        Some(_) => Ok(false),
        None => Err(eof()),
    }
}

/// Parse a dictionary key, which must be a name.
fn dict_key(lx: &mut Lexer) -> Result<String> {
    match parse_object(lx)? {
        Object::Name(n) => Ok(n),
        _ => Err(err(
            ErrorCode::UnexpectedToken,
            "dictionary key is not a name",
        )),
    }
}

/// Parse a numeric token, promoting `n g R` to a reference on lookahead.
fn parse_number_or_ref(lx: &mut Lexer) -> Result<Object> {
    let tok = lx
        .read_keyword()
        .ok_or_else(|| err(ErrorCode::UnexpectedToken, "empty number"))?;
    let value = parse_numeric(tok)?;
    match reference_lookahead(lx, &value) {
        Some(r) => Ok(Object::Reference(r)),
        None => Ok(value),
    }
}

/// Exact powers of ten (all representable in `f64`) for the fast real parser.
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

/// Parse a numeric literal into an integer or real object. The hot path
/// (`fast_numeric`) hand-parses PDF's simple `[+-]?digits[.digits]?` form with no
/// allocation and bit-exact results for short decimals; only over-long or
/// overflowing numbers fall back to the standard parser.
fn parse_numeric(tok: &[u8]) -> Result<Object> {
    match fast_numeric(tok) {
        Some(obj) => Ok(obj),
        None => slow_numeric(tok),
    }
}

/// Accumulated digits of a numeric token: `frac` is the fractional digit count
/// (`-1` before any `.`), `mantissa` the integer of all digits.
struct NumAcc {
    mantissa: i64,
    frac: i32,
    any: bool,
}

impl NumAcc {
    /// A fresh accumulator (`frac = -1`: no decimal point seen yet).
    fn new() -> Self {
        NumAcc {
            mantissa: 0,
            frac: -1,
            any: false,
        }
    }

    fn push(&mut self, b: u8) -> Option<()> {
        match b {
            b'0'..=b'9' => self.push_digit(b),
            b'.' if self.frac < 0 => {
                self.frac = 0;
                Some(())
            }
            _ => None,
        }
    }

    fn push_digit(&mut self, b: u8) -> Option<()> {
        self.mantissa = self
            .mantissa
            .checked_mul(10)?
            .checked_add(i64::from(b - b'0'))?;
        if self.frac >= 0 {
            self.frac += 1;
        }
        self.any = true;
        Some(())
    }

    fn finish(&self, neg: bool) -> Option<Object> {
        if !self.any {
            return None;
        }
        let mantissa = if neg { -self.mantissa } else { self.mantissa };
        match self.frac {
            f if f < 0 => Some(Object::Integer(mantissa)),
            f => POW10
                .get(f as usize)
                .map(|p| Object::Real(mantissa as f64 / p)),
        }
    }
}

/// The no-allocation fast path; `None` falls back to [`slow_numeric`].
fn fast_numeric(tok: &[u8]) -> Option<Object> {
    let (neg, rest) = split_sign(tok);
    let mut acc = NumAcc::new();
    for &b in rest {
        acc.push(b)?;
    }
    acc.finish(neg)
}

/// Split a leading `+`/`-` sign, returning whether it was negative.
fn split_sign(tok: &[u8]) -> (bool, &[u8]) {
    match tok.first() {
        Some(b'-') => (true, &tok[1..]),
        Some(b'+') => (false, &tok[1..]),
        _ => (false, tok),
    }
}

/// Fallback for over-long / overflowing numbers (parsed as a real).
fn slow_numeric(tok: &[u8]) -> Result<Object> {
    let s =
        std::str::from_utf8(tok).map_err(|_| err(ErrorCode::UnexpectedToken, "non-utf8 number"))?;
    s.parse::<f64>()
        .map(Object::Real)
        .map_err(|_| err(ErrorCode::UnexpectedToken, "bad number"))
}

/// If `value` is a non-negative integer and `gen R` follows, build a reference.
/// Restores the cursor and returns `None` when the lookahead does not match.
fn reference_lookahead(lx: &mut Lexer, value: &Object) -> Option<ObjRef> {
    let num = non_negative_u32(value)?;
    let save = lx.pos();
    match read_gen_and_marker(lx, num) {
        Some(r) => Some(r),
        None => {
            lx.seek(save);
            None
        }
    }
}

/// Read `gen R` after an already-parsed object number.
fn read_gen_and_marker(lx: &mut Lexer, num: u32) -> Option<ObjRef> {
    lx.skip_whitespace();
    let gen = parse_u16(lx.read_keyword()?)?;
    lx.skip_whitespace();
    if lx.read_keyword()? == b"R" {
        Some(ObjRef::new(num, gen))
    } else {
        None
    }
}

fn non_negative_u32(value: &Object) -> Option<u32> {
    match value.as_integer() {
        Some(n) if (0..=i64::from(u32::MAX)).contains(&n) => Some(n as u32),
        _ => None,
    }
}

fn parse_u16(tok: &[u8]) -> Option<u16> {
    std::str::from_utf8(tok).ok()?.parse().ok()
}

/// Parse the bare keyword literals `true`, `false`, and `null`.
fn parse_keyword_literal(lx: &mut Lexer) -> Result<Object> {
    match lx.read_keyword() {
        Some(b"true") => Ok(Object::Boolean(true)),
        Some(b"false") => Ok(Object::Boolean(false)),
        Some(b"null") => Ok(Object::Null),
        _ => Err(err(ErrorCode::UnexpectedToken, "unexpected token")),
    }
}

/// Hex digit value (0–15) for an ASCII hex byte, or `None`.
pub fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Object {
        parse_object(&mut Lexer::new(bytes)).unwrap()
    }

    #[test]
    fn numbers_integer_and_real() {
        assert_eq!(parse(b"42"), Object::Integer(42));
        assert_eq!(parse(b"-17"), Object::Integer(-17));
        assert_eq!(parse(b"+8"), Object::Integer(8));
        assert_eq!(parse(b"3.5"), Object::Real(3.5));
        assert_eq!(parse(b"-.5"), Object::Real(-0.5));
        assert_eq!(parse(b"5."), Object::Real(5.0));
    }

    #[test]
    fn booleans_and_null() {
        assert_eq!(parse(b"true"), Object::Boolean(true));
        assert_eq!(parse(b"false"), Object::Boolean(false));
        assert_eq!(parse(b"null"), Object::Null);
    }

    #[test]
    fn names_with_escapes() {
        assert_eq!(parse(b"/Type"), Object::Name("Type".into()));
        assert_eq!(parse(b"/A#20B"), Object::Name("A B".into()));
    }

    #[test]
    fn literal_strings() {
        assert_eq!(parse(b"(hello)"), Object::String(b"hello".to_vec()));
        assert_eq!(parse(b"(a(b)c)"), Object::String(b"a(b)c".to_vec()));
        assert_eq!(parse(b"(a\\nb)"), Object::String(b"a\nb".to_vec()));
        assert_eq!(parse(b"(\\101)"), Object::String(b"A".to_vec()));
        // single octal digit terminated by the closing paren (octal loop breaks).
        assert_eq!(parse(b"(\\7)"), Object::String(vec![0x07]));
        assert_eq!(parse(b"(tab\\t)"), Object::String(b"tab\t".to_vec()));
        assert_eq!(
            parse(b"(esc\\(\\)\\\\)"),
            Object::String(b"esc()\\".to_vec())
        );
    }

    #[test]
    fn literal_string_escape_extras() {
        // \r \b \f, unknown escape (\z → z), CR line continuation.
        assert_eq!(
            parse(b"(\\r\\b\\f)"),
            Object::String(vec![b'\r', 0x08, 0x0c])
        );
        assert_eq!(parse(b"(\\z)"), Object::String(b"z".to_vec()));
        assert_eq!(parse(b"(a\\\r\nb)"), Object::String(b"ab".to_vec()));
        assert_eq!(parse(b"(a\\\nb)"), Object::String(b"ab".to_vec()));
    }

    #[test]
    fn hex_strings() {
        assert_eq!(parse(b"<48656C6C6F>"), Object::String(b"Hello".to_vec()));
        assert_eq!(parse(b"<4 8>"), Object::String(vec![0x48]));
        // odd nibble count pads the trailing low nibble with zero.
        assert_eq!(parse(b"<41A>"), Object::String(vec![0x41, 0xa0]));
    }

    #[test]
    fn number_parser_fast_and_slow_paths() {
        // A bare '.' has no digits → error.
        assert!(parse_object(&mut Lexer::new(b".")).is_err());
        // A real with more fractional digits than the fast table covers falls back
        // to the standard parser and still yields a Real.
        let long = b"0.000000000000000000000001"; // 24 fractional digits
        assert!(matches!(parse(long), Object::Real(_)));
        // A non-UTF-8 byte in a numeric token errors via the fallback.
        assert!(parse_object(&mut Lexer::new(b"1\xff")).is_err());
        // Big integers within i64 are exact via the fast path.
        assert_eq!(
            parse(b"123456789012345"),
            Object::Integer(123_456_789_012_345)
        );
    }

    #[test]
    fn arrays_nested() {
        let got = parse(b"[1 2 [3 true] /X]");
        let inner = Object::Array(vec![Object::Integer(3), Object::Boolean(true)]);
        assert_eq!(
            got,
            Object::Array(vec![
                Object::Integer(1),
                Object::Integer(2),
                inner,
                Object::Name("X".into()),
            ])
        );
    }

    #[test]
    fn dictionaries_and_references() {
        let got = parse(b"<< /Type /Page /Parent 4 0 R /Count 3 >>");
        let d = got.as_dict().unwrap();
        assert_eq!(d.get("Type").unwrap().as_name(), Some("Page"));
        assert_eq!(
            d.get("Parent").unwrap().as_reference(),
            Some(ObjRef::new(4, 0))
        );
        assert_eq!(d.get("Count").unwrap().as_integer(), Some(3));
    }

    #[test]
    fn reference_lookahead_restores_on_non_match() {
        // "5 0 X" is not a reference: parse the 5, leave "0 X" untouched.
        let mut lx = Lexer::new(b"5 0 X");
        assert_eq!(parse_object(&mut lx).unwrap(), Object::Integer(5));
        assert_eq!(parse_object(&mut lx).unwrap(), Object::Integer(0));
    }

    #[test]
    fn negative_is_never_a_reference() {
        let mut lx = Lexer::new(b"-1 0 R");
        assert_eq!(parse_object(&mut lx).unwrap(), Object::Integer(-1));
    }

    #[test]
    fn errors() {
        assert!(parse_object(&mut Lexer::new(b"")).is_err());
        assert!(parse_object(&mut Lexer::new(b"]")).is_err());
        assert!(parse_object(&mut Lexer::new(b"[1 2")).is_err()); // unterminated array
        assert!(parse_object(&mut Lexer::new(b"bogus")).is_err());
        assert!(parse_object(&mut Lexer::new(b"(unterminated")).is_err());
        assert!(parse_object(&mut Lexer::new(b"<deadbeef")).is_err());
        assert!(parse_object(&mut Lexer::new(b"<<")).is_err());
        assert!(parse_object(&mut Lexer::new(b"<zz>")).is_err());
        assert!(parse_object(&mut Lexer::new(b"<< 1 2 >>")).is_err()); // key not a name
        assert!(parse_object(&mut Lexer::new(b"/A#zz")).is_err());
        assert!(parse_object(&mut Lexer::new(b"1.2.3")).is_err());
    }

    #[test]
    fn hex_val_table() {
        assert_eq!(hex_val(b'0'), Some(0));
        assert_eq!(hex_val(b'9'), Some(9));
        assert_eq!(hex_val(b'a'), Some(10));
        assert_eq!(hex_val(b'F'), Some(15));
        assert_eq!(hex_val(b'g'), None);
    }
}
