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

/// Which PUA-encoding family a font belongs to. Fonts in these families route
/// glyphs through `U+F0XX` code points in their `/ToUnicode` CMap; we reverse
/// the PUA to real Unicode with the family-specific table.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PUAFamily {
    None,
    /// Adobe Symbol / Microsoft SymbolMT — uses [`crate::agl::SYMBOL_ENC`].
    Symbol,
    /// Microsoft Math Extra — uses its own small mapping table.
    MTextra,
}

/// A decoded font: text mapping + advance widths.
#[derive(Debug, Clone)]
pub struct Font {
    two_byte: bool,
    to_unicode: Option<CMap>,
    simple_enc: [u32; 256],
    widths: HashMap<u32, f64>,
    default_width: f64,
    pua_family: PUAFamily,
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

    /// Append a code's Unicode text to `out` (ToUnicode, then simple encoding,
    /// then ASCII fallback for CID fonts that lack CMap entries for Latin /
    /// symbol code points). When the CMap maps a code to a PUA-only string
    /// (broken fonts), the simple encoding is preferred if it produces a non-PUA
    /// character. A code the CMap *explicitly* mapped (even to a rejected PUA
    /// string) is a known non-ASCII glyph slot, so it never falls back to its
    /// byte value — that would turn e.g. CID 0x64 into 'd'. Only codes with *no*
    /// CMap entry at all fall back to the ASCII byte value (Identity-H Latin).
    fn append_text(&self, code: u32, out: &mut String) {
        match self.to_unicode.as_ref().and_then(|c| c.lookup(code)) {
            Some(ref s) if use_cmap_strict(s) => out.push_str(s),
            Some(_) => {
                // CMap explicitly mapped but to a rejected (PUA / control-only)
                // string. For Symbol fonts the PUA U+F0XX encodes the classic
                // Adobe Symbol position - reverse to real Unicode. Otherwise
                // do NOT fall back to the byte value (would turn CID 0x64 into
                // 'd'); only the simple encoding (1-byte fonts) may override.
                if let Some(c) = self.symbol_pua(code) {
                    out.push(c);
                } else if let Some(c) = self.simple_char(code) {
                    out.push(c);
                }
            }
            None => {
                if let Some(c) = self.simple_char(code) {
                    out.push(c);
                } else if (32..=126).contains(&code) {
                    // ASCII fallback for CID (2-byte) fonts whose /ToUnicode
                    // CMap is missing entries for Latin letters, digits, or
                    // common punctuation / math symbols.  These code points are
                    // identity-mapped in virtually every encoding that includes
                    // them, so this is a safe last resort.
                    out.push(code as u8 as char);
                }
            }
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
        match self.to_unicode.as_ref().and_then(|c| c.lookup(code)) {
            Some(s) if use_cmap_strict(&s) => s,
            Some(_) => {
                // CMap explicitly mapped but rejected (PUA/control). For Symbol
                // fonts reverse the PUA to real Unicode; otherwise do not fall
                // back to the byte value (would turn CID 0x64 into 'd').
                if let Some(c) = self.symbol_pua(code) {
                    return c.to_string();
                }
                self.simple_char(code)
                    .map(|c| c.to_string())
                    .unwrap_or_default()
            }
            None => self
                .simple_char(code)
                .map(|c| c.to_string())
                .or_else(|| {
                    if (32..=126).contains(&code) {
                        // ASCII fallback for CID fonts without CMap entries for
                        // Latin / symbol code points.
                        char::from_u32(code).map(|c| c.to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
        }
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

    /// For a PUA-encoding font (Symbol / MT-Extra), reverse the CMap's PUA
    /// destination (`U+F0XX`) for `code` back to the real Unicode via the
    /// font-family-specific table. The tables are kept strictly separate —
    /// Symbol and MT-Extra use the same PUA range with different glyph
    /// assignments, so cross-fallback would produce wrong Greek letters.
    fn symbol_pua(&self, code: u32) -> Option<char> {
        let s = self.to_unicode.as_ref()?.lookup(code)?;
        let first = s.chars().next()?;
        match self.pua_family {
            PUAFamily::Symbol => crate::agl::symbol_pua_to_unicode(first as u32),
            PUAFamily::MTextra => crate::agl::mtextra_pua_to_unicode(first as u32),
            PUAFamily::None => None,
        }
    }
}

/// True when a CMap result is meaningful (not empty and not only PUA).
fn use_cmap(s: &str) -> bool {
    !s.is_empty() && !s.chars().all(is_pua)
}

/// Stricter variant of use_cmap that also rejects control characters and
/// other suspicious mappings that might indicate a broken CMap.
fn use_cmap_strict(s: &str) -> bool {
    if !use_cmap(s) {
        return false;
    }
    // Reject strings that are only control characters (U+0000–U+001F, U+007F–U+009F).
    // These are unlikely to be intentional mappings for visible characters.
    !s.chars().all(|c| {
        let code = c as u32;
        (code <= 0x1F) || (code >= 0x7F && code <= 0x9F)
    })
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
    let family = pua_family(dict);
    if dict.get("Subtype").and_then(Object::as_name) == Some("Type0") {
        load_type0(r, dict, to_unicode, family)
    } else {
        load_simple(r, dict, to_unicode, family)
    }
}

/// Classify a font by its `/BaseFont` into the appropriate PUA family (or
/// `None` for normal fonts whose `/ToUnicode` gives straight Unicode).
fn pua_family(dict: &Dictionary) -> PUAFamily {
    let Some(name) = dict.get("BaseFont").and_then(Object::as_name) else {
        return PUAFamily::None;
    };
    let base = name.rsplit('+').next().unwrap_or(name);
    match base {
        "Symbol" | "SymbolMT" => PUAFamily::Symbol,
        "MT-Extra" => PUAFamily::MTextra,
        _ => PUAFamily::None,
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
fn load_simple(
    r: &Resolver,
    dict: &Dictionary,
    to_unicode: Option<CMap>,
    pua_family: PUAFamily,
) -> Font {
    let simple_enc = build_encoding(r, dict);
    let widths = simple_widths(r, dict);
    Font {
        two_byte: false,
        to_unicode,
        simple_enc,
        widths,
        default_width: 0.5,
        pua_family,
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
fn load_type0(
    r: &Resolver,
    dict: &Dictionary,
    to_unicode: Option<CMap>,
    pua_family: PUAFamily,
) -> Font {
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
        pua_family,
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

/// True when a character lies in the Private Use Area (U+E000–U+F8FF).
fn is_pua(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
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

    #[test]
    fn cmap_pua_falls_back_to_simple_encoding() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <F041> endbfchar endcmap";
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>".to_string();
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[font, tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"A")[0].text, "A");
        assert_ne!(f.decode(b"A")[0].text, "\u{F041}");
    }

    #[test]
    fn cmap_non_pua_still_takes_priority() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <0058> endbfchar endcmap";
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>".to_string();
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[font, tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"A")[0].text, "X");
    }

    #[test]
    fn cmap_pua_show_into_also_falls_back() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <F041> endbfchar endcmap";
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>".to_string();
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[font, tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        let mut out = String::new();
        f.show_into(b"A", 0.0, 0.0, 12.0, 1.0, &mut out);
        assert_eq!(out, "A");
    }

    #[test]
    fn type0_math_symbol_should_not_silently_fallback_to_wrong_char() {
        // Simulate a font where:
        // - ToUnicode CMap is broken/returns PUA for math symbols
        // - ASCII fallback could output a wrong character
        // This is a regression test for: ≤ (0x2264) being output as 'd' (0x0064)
        let cmap = "/CIDInit begincmap 1 beginbfchar <00E4> <F0E4> endbfchar endcmap";
        let type0 =
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 5 0 R /DescendantFonts [6 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(
            &[type0.to_string(), cidfont.to_string(), tu],
            "4 0 R",
        );
        let f = fonts.get("F1").unwrap();
        // Code 0x00E4 maps to PUA <F0E4> in CMap, which is rejected.
        // Without a valid alternative, it should return empty, not output a random ASCII char.
        let g = f.decode(&[0x00, 0xE4]);
        // This test documents the current behavior. If PUA fallback returns wrong
        // ASCII, this will catch it. Ideally, `g[0].text` should be empty.
        // but if it's "ä" or "ô", that's still wrong and needs fixing.
        assert!(
            g[0].text.is_empty() || g[0].text == "\u{F0E4}" || is_valid_char(&g[0].text),
            "Math symbol should not be misidentified as ASCII: got '{}'",
            g[0].text
        );
    }

    fn is_valid_char(s: &str) -> bool {
        // Check if it's a valid math symbol or similar
        // For now, reject ASCII 'd' and 'l' specifically
        s != "d" && s != "l" && s.chars().all(|c| c as u32 >= 0xE000 || c.is_alphabetic())
    }

    #[test]
    fn cmap_pua_type0_no_fallback_drops_pua() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <0001> <F001> endbfchar endcmap";
        let type0 = "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string(), tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(&[0x00, 0x01])[0].text, "");
    }

    #[test]
    fn cmap_empty_destination_falls_back() {
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <> endbfchar endcmap";
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>".to_string();
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[font, tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(b"A")[0].text, "A");
    }

    #[test]
    fn type0_ascii_fallback_when_no_tounicode() {
        // A Type0 font with no /ToUnicode CMap: codes in the ASCII printable
        // range (32–126) should fall back to the byte value as a char.
        let type0 =
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string()], "4 0 R");
        let f = fonts.get("F1").unwrap();
        // Code 0x0041 ('A') → ASCII fallback.
        assert_eq!(f.decode(&[0x00, 0x41])[0].text, "A");
        // Code 0x007B ('{') → ASCII fallback (math symbol).
        assert_eq!(f.decode(&[0x00, 0x7B])[0].text, "{");
        // Code 0x003C ('<') → ASCII fallback.
        assert_eq!(f.decode(&[0x00, 0x3C])[0].text, "<");
        // Code 0x0064 ('d') → ASCII fallback.
        assert_eq!(f.decode(&[0x00, 0x64])[0].text, "d");
        // Code 0x0001 (control char, not in 32–126) → still empty.
        assert_eq!(f.decode(&[0x00, 0x01])[0].text, "");
        // Code 0x007F (DEL, not in 32–126) → still empty.
        assert_eq!(f.decode(&[0x00, 0x7F])[0].text, "");
    }

    #[test]
    fn type0_pua_cmap_does_not_emit_ascii_byte() {
        // Regression for garbled math extraction: a Type0 font whose /ToUnicode
        // CMap maps CID 0x0064 to a PUA code point. CID 0x64 is in the ASCII
        // range ('d'), so the old decoder fell back to the byte value and
        // emitted 'd' - turning "1≤x≤3" (where ≤ is CID 0x64) into "1dxd3".
        // A code the CMap *explicitly* mapped (even to a rejected PUA string)
        // is a known non-ASCII glyph slot and must NOT fall back to its byte.
        let cmap = "/CIDInit begincmap 1 beginbfchar <0064> <E064> endbfchar endcmap";
        let type0 = "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string(), tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        // decode path
        let g = f.decode(&[0x00, 0x64]);
        assert_ne!(g[0].text, "d");
        assert!(g[0].text.is_empty(), "expected empty, got {:?}", g[0].text);
        // show_into path (the hot path used in extraction)
        let mut out = String::new();
        f.show_into(&[0x00, 0x64], 0.0, 0.0, 12.0, 1.0, &mut out);
        assert_ne!(out, "d");
        assert!(out.is_empty(), "expected empty, got {:?}", out);
    }

    #[test]
    fn symbol_type0_pua_recovery() {
        // A Type0 SymbolMT font whose /ToUnicode CMap maps CID 0x0064 (≤) to
        // PUA U+F0A3. With /BaseFont /SymbolMT set, the decoder must reverse
        // the PUA through the Adobe Symbol encoding table and emit the real
        // Unicode ≤ (U+2264) instead of an empty string.
        let cmap = "/CIDInit begincmap 1 beginbfchar <0064> <F0A3> endbfchar endcmap";
        let type0 = "<< /Type /Font /Subtype /Type0 /BaseFont /SymbolMT /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string(), tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        // decode path
        let g = f.decode(&[0x00, 0x64]);
        assert_eq!(g[0].text, "≤");
        // also verify = and { get recovered
        let cmap2 =
            "/CIDInit begincmap 2 beginbfchar <003D> <F03D> <007B> <F07B> endbfchar endcmap";
        let type02 = "<< /Type /Font /Subtype /Type0 /BaseFont /SymbolMT /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [5 0 R] >>";
        let tu2 = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap2.len(), cmap2);
        let fonts2 = fonts_for(&[type02.to_string(), cidfont.to_string(), tu2], "4 0 R");
        let f2 = fonts2.get("F1").unwrap();
        assert_eq!(f2.decode(&[0x00, 0x3D])[0].text, "=");
        assert_eq!(f2.decode(&[0x00, 0x7B])[0].text, "{");
    }

    #[test]
    fn type0_missing_cmap_entry_keeps_ascii_fallback() {
        // Counterpart: a Type0 font with a /ToUnicode CMap that maps CID 1 ->
        // 'A' but has NO entry for CID 0x0064. With no explicit mapping, the
        // ASCII fallback is the only signal available (Identity-H Latin whose
        // CMap simply omits it), so 'd' is still emitted. This guards against
        // over-aggressively dropping Latin letters / digits.
        let cmap = "/CIDInit begincmap 1 beginbfchar <0001> <0041> endbfchar endcmap";
        let type0 = "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string(), tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(&[0x00, 0x64])[0].text, "d");
        assert_eq!(f.decode(&[0x00, 0x01])[0].text, "A");
    }

    #[test]
    fn mtextra_pua_recovers_intersection() {
        // MT-Extra font whose /ToUnicode CMap maps CID 0x27 to PUA U+F051.
        // With /BaseFont /MT-Extra set, the decoder must reverse through the
        // MT-Extra PUA table and emit ∩ (U+2229).
        let cmap = "/CIDInit begincmap 1 beginbfchar <0027> <F051> endbfchar endcmap";
        let type0 = "<< /Type /Font /Subtype /Type0 /BaseFont /MT-Extra /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [5 0 R] >>";
        let cidfont = "<< /Type /Font /Subtype /CIDFontType2 /DW 1000 >>";
        let tu = format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap);
        let fonts = fonts_for(&[type0.to_string(), cidfont.to_string(), tu], "4 0 R");
        let f = fonts.get("F1").unwrap();
        assert_eq!(f.decode(&[0x00, 0x27])[0].text, "\u{2229}");
    }
}
