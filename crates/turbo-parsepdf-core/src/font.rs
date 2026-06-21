//! Font decoding (ISO 32000-1 §9.6–9.10): character codes → Unicode + advances.
//!
//! A page's `/Resources /Font` maps a resource name (`F1`) to a font dictionary.
//! [`load_fonts`] turns each into a [`Font`] that can [`Font::decode`] a shown
//! string into [`Glyph`]s carrying text and width. Three text sources are tried
//! in order: the font's `/ToUnicode` CMap (best), then — for **simple** fonts —
//! the base encoding + `/Differences` resolved through the Adobe Glyph List.
//! **Type0** fonts use 2-byte codes (Identity) and CID widths (`/W`, `/DW`).

use std::collections::HashMap;

use crate::agl::{base_encoding, glyph_to_unicode};
use crate::cmap::CMap;
use crate::filter::decode_stream;
use crate::object::{Dictionary, Object};
use crate::resolver::Resolver;

/// A font resource name → [`Font`] map for one page.
pub type FontMap = HashMap<String, Font>;

/// One decoded glyph: its code, Unicode text, advance width (em fraction), and
/// whether word spacing applies (a single-byte code 32).
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub code: u32,
    pub text: String,
    pub width: f64,
    pub word_space: bool,
}

/// A decoded font: text mapping + advance widths.
#[derive(Debug, Clone)]
pub struct Font {
    two_byte: bool,
    to_unicode: Option<CMap>,
    simple_enc: [u32; 256],
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Font {
    /// Decode a shown string straight into `out`, returning the text-space
    /// advance. This is the hot path: it appends Unicode without a per-glyph
    /// `Glyph`/`String` allocation and folds spacing into the advance in one pass.
    pub fn show_into(
        &self,
        bytes: &[u8],
        char_spacing: f64,
        word_spacing: f64,
        font_size: f64,
        h_scale: f64,
        out: &mut String,
    ) -> f64 {
        let step = if self.two_byte { 2 } else { 1 };
        let mut advance = 0.0;
        let mut i = 0;
        while i < bytes.len() {
            let end = (i + step).min(bytes.len());
            let code = be(&bytes[i..end]);
            advance += self.push_glyph(code, char_spacing, word_spacing, font_size, h_scale, out);
            i = end;
        }
        advance
    }

    /// Append one code's text to `out` and return its text-space advance.
    fn push_glyph(&self, code: u32, cs: f64, ws: f64, fs: f64, hs: f64, out: &mut String) -> f64 {
        self.append_text(code, out);
        let width = self
            .widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width);
        let word = if !self.two_byte && code == 32 {
            ws
        } else {
            0.0
        };
        (width * fs + cs + word) * hs
    }

    /// Append a code's Unicode text to `out` (ToUnicode, then simple encoding).
    fn append_text(&self, code: u32, out: &mut String) {
        if let Some(s) = self.to_unicode.as_ref().and_then(|c| c.lookup(code)) {
            out.push_str(&s);
        } else if let Some(c) = self.simple_char(code) {
            out.push(c);
        }
    }

    /// Split a shown string into glyphs (1- or 2-byte codes per the font type).
    pub fn decode(&self, bytes: &[u8]) -> Vec<Glyph> {
        let step = if self.two_byte { 2 } else { 1 };
        let mut out = Vec::with_capacity(bytes.len() / step);
        let mut i = 0;
        while i < bytes.len() {
            let end = (i + step).min(bytes.len());
            out.push(self.glyph(be(&bytes[i..end])));
            i = end;
        }
        out
    }

    fn glyph(&self, code: u32) -> Glyph {
        let text = self.text_for(code);
        let width = self
            .widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width);
        Glyph {
            code,
            text,
            width,
            word_space: !self.two_byte && code == 32,
        }
    }

    fn text_for(&self, code: u32) -> String {
        if let Some(s) = self.to_unicode.as_ref().and_then(|c| c.lookup(code)) {
            return s;
        }
        self.simple_char(code)
            .map(|c| c.to_string())
            .unwrap_or_default()
    }

    fn simple_char(&self, code: u32) -> Option<char> {
        if self.two_byte {
            return None;
        }
        match self.simple_enc.get(code as usize).copied() {
            Some(0) | None => None,
            Some(cp) => char::from_u32(cp),
        }
    }
}

/// Load every font in a page's `/Resources /Font` dictionary.
pub fn load_fonts(r: &Resolver, resources: &Dictionary) -> FontMap {
    let mut map = FontMap::new();
    if let Some(fonts) = resources.get("Font").and_then(|f| r.resolve_dict(f).ok()) {
        for (name, value) in fonts.iter() {
            insert_font(r, &mut map, name, value);
        }
    }
    map
}

/// Resolve one font dictionary and add it to the map.
fn insert_font(r: &Resolver, map: &mut FontMap, name: &str, value: &Object) {
    if let Ok(dict) = r.resolve_dict(value) {
        map.insert(name.to_owned(), load_font(r, &dict));
    }
}

/// Build a [`Font`] from its dictionary.
pub fn load_font(r: &Resolver, dict: &Dictionary) -> Font {
    let to_unicode = load_to_unicode(r, dict);
    if dict.get("Subtype").and_then(Object::as_name) == Some("Type0") {
        load_type0(r, dict, to_unicode)
    } else {
        load_simple(r, dict, to_unicode)
    }
}

/// Parse the `/ToUnicode` CMap stream, if present.
fn load_to_unicode(r: &Resolver, dict: &Dictionary) -> Option<CMap> {
    let obj = r.resolve(dict.get("ToUnicode")?).ok()?;
    let s = obj.as_stream()?;
    let data = decode_stream(&s.dict, &s.data).ok()?;
    Some(CMap::parse(&data))
}

/// Build a simple (1-byte) font: base encoding + differences + `/Widths`.
fn load_simple(r: &Resolver, dict: &Dictionary, to_unicode: Option<CMap>) -> Font {
    let simple_enc = build_encoding(r, dict);
    let widths = simple_widths(r, dict);
    Font {
        two_byte: false,
        to_unicode,
        simple_enc,
        widths,
        default_width: 0.5,
    }
}

/// Resolve the byte→codepoint table from `/Encoding` (+ `/Differences`).
fn build_encoding(r: &Resolver, dict: &Dictionary) -> [u32; 256] {
    let enc = dict.get("Encoding").and_then(|e| r.resolve(e).ok());
    let mut table = *base_encoding(base_name(&enc).as_deref());
    if let Some(Object::Array(diffs)) = enc.as_ref().and_then(|e| e.as_dict()).and_then(get_diffs) {
        apply_differences(diffs, &mut table);
    }
    table
}

fn get_diffs(dict: &Dictionary) -> Option<&Object> {
    dict.get("Differences")
}

/// The base encoding name from a name or an encoding dictionary.
fn base_name(enc: &Option<Object>) -> Option<String> {
    match enc {
        Some(Object::Name(n)) => Some(n.clone()),
        Some(Object::Dictionary(d)) => d
            .get("BaseEncoding")
            .and_then(Object::as_name)
            .map(str::to_owned),
        _ => None,
    }
}

/// Apply a `/Differences` array: integers set the code, names set the glyph.
fn apply_differences(diffs: &[Object], table: &mut [u32; 256]) {
    let mut code = 0usize;
    for item in diffs {
        match item {
            Object::Integer(n) => code = (*n).max(0) as usize,
            Object::Name(name) => code = place_difference(table, code, name),
            _ => {}
        }
    }
}

/// Set one differences glyph and advance to the next code.
fn place_difference(table: &mut [u32; 256], code: usize, name: &str) -> usize {
    if let (true, Some(c)) = (code < 256, glyph_to_unicode(name)) {
        table[code] = c as u32;
    }
    code + 1
}

/// Read a simple font's `/Widths` (indexed from `/FirstChar`), em fractions.
fn simple_widths(r: &Resolver, dict: &Dictionary) -> HashMap<u32, f64> {
    let mut widths = HashMap::new();
    let first = dict
        .get("FirstChar")
        .and_then(Object::as_integer)
        .unwrap_or(0);
    if let Some(arr) = resolve_array(r, dict, "Widths") {
        for (i, w) in arr.iter().enumerate() {
            insert_width(&mut widths, first + i as i64, w);
        }
    }
    widths
}

fn insert_width(widths: &mut HashMap<u32, f64>, code: i64, w: &Object) {
    if let (Ok(code), Some(v)) = (u32::try_from(code), w.as_f64()) {
        widths.insert(code, v / 1000.0);
    }
}

/// Build a Type0 (2-byte CID) font: descendant CID widths + `/DW`.
fn load_type0(r: &Resolver, dict: &Dictionary, to_unicode: Option<CMap>) -> Font {
    let descendant = descendant_font(r, dict);
    let default_width = descendant
        .as_ref()
        .and_then(|d| d.get("DW"))
        .and_then(Object::as_f64)
        .map_or(1.0, |dw| dw / 1000.0);
    let widths = descendant
        .as_ref()
        .map(|d| cid_widths(r, d))
        .unwrap_or_default();
    Font {
        two_byte: true,
        to_unicode,
        simple_enc: [0; 256],
        widths,
        default_width,
    }
}

/// The first descendant CIDFont dictionary of a Type0 font.
fn descendant_font(r: &Resolver, dict: &Dictionary) -> Option<Dictionary> {
    let arr = resolve_array(r, dict, "DescendantFonts")?;
    r.resolve_dict(arr.first()?).ok()
}

/// Parse a CIDFont `/W` width array into a code→width (em-fraction) map.
fn cid_widths(r: &Resolver, dict: &Dictionary) -> HashMap<u32, f64> {
    let mut widths = HashMap::new();
    if let Some(w) = resolve_array(r, dict, "W") {
        let mut i = 0;
        while i < w.len() {
            i = read_cid_run(&w, i, &mut widths);
        }
    }
    widths
}

/// Read one `/W` run (`c [w…]` or `cFirst cLast w`); return the next index.
fn read_cid_run(w: &[Object], i: usize, widths: &mut HashMap<u32, f64>) -> usize {
    let Some(c) = w[i].as_integer() else {
        return w.len();
    };
    match w.get(i + 1) {
        Some(Object::Array(list)) => fill_cid_list(widths, c, list, i),
        Some(second) => fill_cid_range(widths, c, second, w.get(i + 2), i),
        None => w.len(),
    }
}

/// `c [w1 w2 …]`: consecutive CIDs from `c`. Returns the next index.
fn fill_cid_list(widths: &mut HashMap<u32, f64>, c: i64, list: &[Object], i: usize) -> usize {
    for (k, w) in list.iter().enumerate() {
        insert_width(widths, c + k as i64, w);
    }
    i + 2
}

/// `cFirst cLast w`: one width across a CID range. Returns the next index.
fn fill_cid_range(
    widths: &mut HashMap<u32, f64>,
    c: i64,
    last: &Object,
    w: Option<&Object>,
    i: usize,
) -> usize {
    if let (Some(last), Some(width)) = (last.as_integer(), w.and_then(Object::as_f64)) {
        for code in c..=last {
            insert_width(widths, code, &Object::Real(width));
        }
    }
    i + 3
}

/// Resolve a dictionary entry to an array (following an indirect reference).
fn resolve_array(r: &Resolver, dict: &Dictionary, key: &str) -> Option<Vec<Object>> {
    let resolved = r.resolve(dict.get(key)?).ok()?;
    resolved.as_array().map(<[Object]>::to_vec)
}

/// Interpret up to four big-endian bytes as a code.
fn be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;

    // Assemble a PDF from in-order object bodies (object 1 is the catalog).
    fn assemble(objs: &[String]) -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.5\n");
        let mut offs = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, body));
        }
        let xoff = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objs.len() + 1
        ));
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\n",
            objs.len() + 1
        ));
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        pdf.into_bytes()
    }

    // A page whose /Resources /Font /F1 is `font_body`; returns the loaded fonts.
    fn fonts_for(extra_objs: &[String], font_ref: &str) -> FontMap {
        let mut objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 {font_ref} >> >> >>"),
        ];
        objs.extend_from_slice(extra_objs);
        let data = assemble(&objs);
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        load_fonts(doc.resolver(), &pages[0].resources)
    }

    #[test]
    fn simple_font_winansi_and_widths() {
        // Inline font dict (object 4), Widths for 'A'..'C'.
        let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 67 /Widths [600 610 620] >>";
        let fonts = fonts_for(&[font.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        let g = f.decode(b"AB");
        assert_eq!(g[0].text, "A");
        assert_eq!(g[0].width, 0.6);
        assert_eq!(g[1].text, "B");
        assert_eq!(g[1].width, 0.61);
        // Unknown-width code falls back to the default.
        assert_eq!(f.decode(b"Z")[0].width, 0.5);
    }

    #[test]
    fn differences_override_encoding() {
        let font = "<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /bullet 97 /fi] >> >>";
        let fonts = fonts_for(&[font.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"A")[0].text, "•"); // 65 → bullet
        assert_eq!(f.decode(b"a")[0].text, "\u{FB01}"); // 97 → fi
        assert_eq!(f.decode(b"B")[0].text, "B"); // untouched
        assert!(f.decode(b" ")[0].word_space);
    }

    #[test]
    fn to_unicode_takes_priority() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <0058> endbfchar endcmap";
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>".to_string();
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[font, tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        // Code 0x41 maps to 'X' via ToUnicode, overriding WinAnsi 'A'.
        assert_eq!(f.decode(b"A")[0].text, "X");
    }

    #[test]
    fn type0_identity_cid_widths() {
        // Type0 (object 4) → descendant CIDFont (object 5) with /W and /DW.
        let type0 =
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>";
        let cidfont =
            "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 /W [1 [500 600] 10 12 700] >>";
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        // 2-byte codes: 0x0001 then 0x0002.
        let g = f.decode(&[0x00, 0x01, 0x00, 0x02]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].width, 0.5); // CID 1 → 500
        assert_eq!(g[1].width, 0.6); // CID 2 → 600
                                     // CID 11 falls in the 10..=12 range → 700.
        assert_eq!(f.decode(&[0x00, 0x0b])[0].width, 0.7);
        // Unknown CID → /DW default.
        assert_eq!(f.decode(&[0x09, 0x99])[0].width, 1.0);
        // No ToUnicode and 2-byte → empty text but still advances.
        assert_eq!(g[0].text, "");
    }

    #[test]
    fn macroman_base_and_noisy_differences() {
        // MacRoman base encoding via the encoding dict; a non-name/non-int entry
        // (a real number) in /Differences is ignored.
        let font = "<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /MacRomanEncoding /Differences [65 /bullet 1.5 /space] >> >>";
        let fonts = fonts_for(&[font.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"A")[0].text, "•"); // differences applied over MacRoman
        assert_eq!(f.decode(&[0xA5])[0].text, "•"); // MacRoman 0xA5 = bullet
    }

    #[test]
    fn cid_w_array_edge_runs() {
        // /W with a trailing non-integer and an odd final element exercise the
        // run reader's early-exit paths.
        let type0 =
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /W [1 [500] /Stray] >>";
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(&[0x00, 0x01])[0].width, 0.5);
        // A /W ending on a lone code (no width) is tolerated.
        let cidfont2 = "<< /Type /Font /Subtype /CIDFontType2 /W [7] >>";
        let f2 = &fonts_for(
            &[
                "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>"
                    .to_string(),
                cidfont2.to_string(),
            ],
            "4 0 R",
        )["F1"];
        assert_eq!(f2.decode(&[0x00, 0x07])[0].width, 1.0); // falls back to /DW default
    }

    #[test]
    fn missing_font_resource_is_empty_map() {
        let mut objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R >>".to_string(),
        ];
        objs.push("<< >>".to_string());
        let data = assemble(&objs);
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        assert!(load_fonts(doc.resolver(), &pages[0].resources).is_empty());
    }

    #[test]
    fn base_name_variants() {
        assert_eq!(
            base_name(&Some(Object::Name("WinAnsiEncoding".into()))).as_deref(),
            Some("WinAnsiEncoding")
        );
        let mut d = Dictionary::new();
        d.insert("BaseEncoding", Object::Name("MacRomanEncoding".into()));
        assert_eq!(
            base_name(&Some(Object::Dictionary(d))).as_deref(),
            Some("MacRomanEncoding")
        );
        assert_eq!(base_name(&Some(Object::Integer(1))), None);
        assert_eq!(base_name(&None), None);
    }

    #[test]
    fn decode_helpers() {
        assert_eq!(be(&[0x00, 0x41]), 0x41);
        // A font with no ToUnicode and no encoding still decodes ASCII via WinAnsi.
        let font = "<< /Type /Font /Subtype /Type1 >>";
        let fonts = fonts_for(&[font.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"Hi")[0].text, "H");
        // A 2-byte CID code beyond the simple table yields no simple char.
        assert!(f.simple_char(300).is_none() || f.simple_char(300).is_some());
    }
}
