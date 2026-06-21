//! Text extraction from content-stream operations (ISO 32000-1 §9).
//!
//! The interpreter walks the [`Op`] list maintaining the graphics CTM and the
//! text state (text matrix, font, size, spacing, scale, leading, rise) and emits
//! a [`TextRun`] at the device-space origin of every text-showing operator
//! (`Tj`, `TJ`, `'`, `"`). Glyph-accurate advances need per-font widths (phase
//! 4); phase 3 advances by an em-fraction estimate so multiple shows on a line do
//! not collapse, which is enough to recover text and group it into lines.

use crate::content::{Op, Operator};
use crate::font::FontMap;
use crate::object::Object;

/// A run of text placed at a device-space origin, with its total advance width.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub font_size: f64,
    pub font: String,
}

/// A 2-D affine transform `[a b 0; c d 0; e f 1]` (PDF row-vector convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    /// The identity transform.
    pub fn identity() -> Self {
        Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// A translation transform.
    pub fn translate(tx: f64, ty: f64) -> Self {
        Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// `self` followed by `m` (matrix product `self × m`).
    pub fn then(&self, m: &Matrix) -> Matrix {
        Matrix {
            a: self.a * m.a + self.b * m.c,
            b: self.a * m.b + self.b * m.d,
            c: self.c * m.a + self.d * m.c,
            d: self.c * m.b + self.d * m.d,
            e: self.e * m.a + self.f * m.c + m.e,
            f: self.e * m.b + self.f * m.d + m.f,
        }
    }

    /// Apply the transform to a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

/// The running graphics + text state of the interpreter.
struct State<'f> {
    fonts: &'f FontMap,
    ctm: Matrix,
    gstack: Vec<Matrix>,
    tm: Matrix,
    tlm: Matrix,
    font: String,
    font_size: f64,
    char_spacing: f64,
    word_spacing: f64,
    h_scale: f64,
    leading: f64,
    rise: f64,
}

impl<'f> State<'f> {
    fn new(fonts: &'f FontMap) -> Self {
        State {
            fonts,
            ctm: Matrix::identity(),
            gstack: Vec::new(),
            tm: Matrix::identity(),
            tlm: Matrix::identity(),
            font: String::new(),
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
        }
    }
}

/// Extract positioned text runs from a content stream's operations, decoding
/// each shown string with the page's fonts.
pub fn extract_runs(ops: &[Op], fonts: &FontMap) -> Vec<TextRun> {
    let mut st = State::new(fonts);
    let mut runs = Vec::new();
    for op in ops {
        apply(&mut st, op.operator, &op.operands, &mut runs);
    }
    runs
}

/// Dispatch one operation (operator + operands) against the interpreter state.
fn apply(st: &mut State, operator: Operator, operands: &[Object], runs: &mut Vec<TextRun>) {
    match operator.as_str() {
        "q" => st.gstack.push(st.ctm),
        "Q" => restore(st),
        "cm" => st.ctm = matrix_of(operands).then(&st.ctm),
        "BT" => begin_text(st),
        "Td" => text_move(st, nth(operands, 0), nth(operands, 1)),
        "TD" => text_move_lead(st, nth(operands, 0), nth(operands, 1)),
        "Tm" => set_text_matrix(st, operands),
        "T*" => next_line(st),
        "Tf" => set_font(st, operands),
        "Tj" => show(st, first_string(operands), runs),
        "TJ" => show_array(st, operands, runs),
        "'" => show_quote(st, operands, runs),
        "\"" => show_dquote(st, operands, runs),
        _ => set_param(st, operator, operands),
    }
}

/// Restore the CTM from the graphics stack (ignored if empty).
fn restore(st: &mut State) {
    if let Some(m) = st.gstack.pop() {
        st.ctm = m;
    }
}

/// `BT`: reset the text and text-line matrices to identity.
fn begin_text(st: &mut State) {
    st.tm = Matrix::identity();
    st.tlm = Matrix::identity();
}

/// `Td`: move to the next line origin offset by `(tx, ty)`.
fn text_move(st: &mut State, tx: f64, ty: f64) {
    st.tlm = Matrix::translate(tx, ty).then(&st.tlm);
    st.tm = st.tlm;
}

/// `TD`: set leading to `-ty`, then behave as `Td`.
fn text_move_lead(st: &mut State, tx: f64, ty: f64) {
    st.leading = -ty;
    text_move(st, tx, ty);
}

/// `Tm`: set the text and text-line matrices directly.
fn set_text_matrix(st: &mut State, operands: &[Object]) {
    st.tlm = matrix_of(operands);
    st.tm = st.tlm;
}

/// `T*`: advance to the next line using the current leading.
fn next_line(st: &mut State) {
    text_move(st, 0.0, -st.leading);
}

/// `Tf`: set the current font name and size.
fn set_font(st: &mut State, operands: &[Object]) {
    if let Some(name) = operands.first().and_then(Object::as_name) {
        st.font = name.to_owned();
    }
    st.font_size = nth(operands, 1);
}

/// The text-state scalar operators `Tc Tw Tz TL Ts`.
fn set_param(st: &mut State, operator: Operator, operands: &[Object]) {
    let v = nth(operands, 0);
    match operator.as_str() {
        "Tc" => st.char_spacing = v,
        "Tw" => st.word_spacing = v,
        "Tz" => st.h_scale = v / 100.0,
        "TL" => st.leading = v,
        "Ts" => st.rise = v,
        _ => {}
    }
}

/// `'`: move to the next line and show a string.
fn show_quote(st: &mut State, operands: &[Object], runs: &mut Vec<TextRun>) {
    next_line(st);
    show(st, first_string(operands), runs);
}

/// `"`: set word/char spacing, move to the next line, and show a string.
fn show_dquote(st: &mut State, operands: &[Object], runs: &mut Vec<TextRun>) {
    st.word_spacing = nth(operands, 0);
    st.char_spacing = nth(operands, 1);
    next_line(st);
    show(st, operands.get(2).and_then(Object::as_string), runs);
}

/// `TJ`: show each string element, applying numeric position adjustments.
fn show_array(st: &mut State, operands: &[Object], runs: &mut Vec<TextRun>) {
    let Some(items) = operands.first().and_then(Object::as_array) else {
        return;
    };
    for item in items {
        show_array_item(st, item, runs);
    }
}

/// One `TJ` element: a string is shown, a number shifts the text position.
fn show_array_item(st: &mut State, item: &Object, runs: &mut Vec<TextRun>) {
    match item {
        Object::String(bytes) => show(st, Some(bytes.as_slice()), runs),
        _ => adjust(st, item.as_f64().unwrap_or(0.0)),
    }
}

/// Apply a `TJ` numeric adjustment (thousandths of an em, leftward).
fn adjust(st: &mut State, amount: f64) {
    let dx = -amount / 1000.0 * st.font_size * st.h_scale;
    st.tm = Matrix::translate(dx, 0.0).then(&st.tm);
}

/// Show a string: decode it with the current font straight into a run string,
/// emit the run at the current origin, and advance the text matrix.
fn show(st: &mut State, bytes: Option<&[u8]>, runs: &mut Vec<TextRun>) {
    let Some(bytes) = bytes else {
        return;
    };
    let trm = st.tm.then(&st.ctm);
    let (x, y) = trm.apply(0.0, st.rise);
    let mut text = String::with_capacity(bytes.len());
    let advance = decode_run(st, bytes, &mut text);
    runs.push(TextRun {
        text,
        x,
        y,
        width: advance,
        font_size: st.font_size,
        font: st.font.clone(),
    });
    st.tm = Matrix::translate(advance, 0.0).then(&st.tm);
}

/// Decode a shown string into `out` with the active font, or a Latin-1 fallback;
/// returns the text-space advance.
fn decode_run(st: &State, bytes: &[u8], out: &mut String) -> f64 {
    match st.fonts.get(&st.font) {
        Some(font) => font.show_into(
            bytes,
            st.char_spacing,
            st.word_spacing,
            st.font_size,
            st.h_scale,
            out,
        ),
        None => fallback_into(st, bytes, out),
    }
}

/// Fallback decode for a byte run with no font resource (Latin-1), estimating a
/// half-em advance per byte.
fn fallback_into(st: &State, bytes: &[u8], out: &mut String) -> f64 {
    let mut advance = 0.0;
    for &b in bytes {
        out.push(b as char);
        let word = if b == b' ' { st.word_spacing } else { 0.0 };
        advance += (0.5 * st.font_size + st.char_spacing + word) * st.h_scale;
    }
    advance
}

/// Build a matrix from six numeric operands (missing entries default to identity).
fn matrix_of(operands: &[Object]) -> Matrix {
    let n = |i: usize| operands.get(i).and_then(Object::as_f64);
    Matrix {
        a: n(0).unwrap_or(1.0),
        b: n(1).unwrap_or(0.0),
        c: n(2).unwrap_or(0.0),
        d: n(3).unwrap_or(1.0),
        e: n(4).unwrap_or(0.0),
        f: n(5).unwrap_or(0.0),
    }
}

/// The `i`-th operand as `f64` (0.0 when absent).
fn nth(operands: &[Object], i: usize) -> f64 {
    operands.get(i).and_then(Object::as_f64).unwrap_or(0.0)
}

/// The first operand as string bytes, if present.
fn first_string(operands: &[Object]) -> Option<&[u8]> {
    operands.first().and_then(Object::as_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::parse_content;

    fn runs(content: &[u8]) -> Vec<TextRun> {
        // No font resources → the Latin-1 fallback decoder is exercised.
        let fonts = FontMap::new();
        extract_runs(&parse_content(content), &fonts)
    }

    #[test]
    fn matrix_multiply_and_apply() {
        let m = Matrix::translate(10.0, 20.0).then(&Matrix::identity());
        assert_eq!(m.apply(0.0, 0.0), (10.0, 20.0));
        let scale = Matrix {
            a: 2.0,
            b: 0.0,
            c: 0.0,
            d: 3.0,
            e: 0.0,
            f: 0.0,
        };
        assert_eq!(scale.apply(1.0, 1.0), (2.0, 3.0));
    }

    #[test]
    fn single_show_position() {
        let r = runs(b"BT /F1 12 Tf 100 700 Td (Hi) Tj ET");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "Hi");
        assert_eq!((r[0].x, r[0].y), (100.0, 700.0));
        assert_eq!(r[0].font_size, 12.0);
        assert_eq!(r[0].font, "F1");
    }

    #[test]
    fn tm_sets_absolute_position() {
        let r = runs(b"BT /F1 10 Tf 1 0 0 1 50 400 Tm (X) Tj ET");
        assert_eq!((r[0].x, r[0].y), (50.0, 400.0));
    }

    #[test]
    fn cm_transforms_origin() {
        // Translate the whole coordinate system by (5, 5) via cm.
        let r = runs(b"1 0 0 1 5 5 cm BT 0 0 Td (A) Tj ET");
        assert_eq!((r[0].x, r[0].y), (5.0, 5.0));
    }

    #[test]
    fn tj_array_shows_strings() {
        let r = runs(b"BT 0 0 Td [(Wo) -250 (rld)] TJ ET");
        let joined: String = r.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(joined, "World");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn leading_and_next_line() {
        // TD sets leading; T* and ' move down by it.
        let r = runs(b"BT 0 800 Td 0 -14 TD (line1) Tj T* (line2) Tj ET");
        assert_eq!(r[0].text, "line1");
        assert!(r[1].y < r[0].y); // moved down a line
    }

    #[test]
    fn quote_and_dquote_operators() {
        let r = runs(b"BT 0 100 Td 0 -10 TD (a) Tj (b) ' 1 2 (c) \" ET");
        let texts: Vec<&str> = r.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c"]);
        // ' and " each advanced to a new (lower) line.
        assert!(r[1].y < r[0].y);
        assert!(r[2].y < r[1].y);
    }

    #[test]
    fn q_q_restore_ctm() {
        let r = runs(b"q 1 0 0 1 9 9 cm Q BT 0 0 Td (z) Tj ET");
        // The cm was inside q/Q, so it is rolled back: origin stays at (0,0).
        assert_eq!((r[0].x, r[0].y), (0.0, 0.0));
        // Q with an empty stack is a no-op.
        assert_eq!(runs(b"Q BT 0 0 Td (k) Tj ET").len(), 1);
    }

    #[test]
    fn text_state_params_apply() {
        // Tc/Tw/Tz/Ts parse without error and affect spacing/position.
        let r = runs(b"BT /F1 10 Tf 2 Tc 5 Tw 200 Tz 1 Ts 0 0 Td (a b) Tj (c) Tj ET");
        assert_eq!(r[0].text, "a b");
        // Second show is advanced to the right of the first.
        assert!(r[1].x > r[0].x);
    }

    #[test]
    fn empty_and_missing_operands() {
        assert!(runs(b"BT ET").is_empty());
        // Tj with no operand emits nothing; TJ with non-array emits nothing.
        assert!(runs(b"BT Tj 5 TJ ET").is_empty());
    }
}
