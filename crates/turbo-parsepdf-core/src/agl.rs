//! Glyph-name and base-encoding tables (Adobe Glyph List subset + the standard
//! simple-font encodings).
//!
//! Simple fonts map a single byte to a glyph *name* via a base encoding
//! (`WinAnsiEncoding`, `MacRomanEncoding`, `StandardEncoding`) plus a
//! `/Differences` override; the glyph name maps to Unicode through the Adobe
//! Glyph List ([`glyph_to_unicode`]). The base encodings are kept as direct
//! byte→codepoint tables (`0` = undefined) for the common no-`/Differences` case.

#[rustfmt::skip]
pub(crate) const WIN_ANSI: [u32; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7,
    8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23,
    24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55,
    56, 57, 58, 59, 60, 61, 62, 63,
    64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 94, 95,
    96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111,
    112, 113, 114, 115, 116, 117, 118, 119,
    120, 121, 122, 123, 124, 125, 126, 127,
    8364, 0, 8218, 402, 8222, 8230, 8224, 8225,
    710, 8240, 352, 8249, 338, 0, 381, 0,
    0, 8216, 8217, 8220, 8221, 8226, 8211, 8212,
    732, 8482, 353, 8250, 339, 0, 382, 376,
    160, 161, 162, 163, 164, 165, 166, 167,
    168, 169, 170, 171, 172, 173, 174, 175,
    176, 177, 178, 179, 180, 181, 182, 183,
    184, 185, 186, 187, 188, 189, 190, 191,
    192, 193, 194, 195, 196, 197, 198, 199,
    200, 201, 202, 203, 204, 205, 206, 207,
    208, 209, 210, 211, 212, 213, 214, 215,
    216, 217, 218, 219, 220, 221, 222, 223,
    224, 225, 226, 227, 228, 229, 230, 231,
    232, 233, 234, 235, 236, 237, 238, 239,
    240, 241, 242, 243, 244, 245, 246, 247,
    248, 249, 250, 251, 252, 253, 254, 255,
];

#[rustfmt::skip]
pub(crate) const MAC_ROMAN: [u32; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7,
    8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23,
    24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55,
    56, 57, 58, 59, 60, 61, 62, 63,
    64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 94, 95,
    96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111,
    112, 113, 114, 115, 116, 117, 118, 119,
    120, 121, 122, 123, 124, 125, 126, 127,
    196, 197, 199, 201, 209, 214, 220, 225,
    224, 226, 228, 227, 229, 231, 233, 232,
    234, 235, 237, 236, 238, 239, 241, 243,
    242, 244, 246, 245, 250, 249, 251, 252,
    8224, 176, 162, 163, 167, 8226, 182, 223,
    174, 169, 8482, 180, 168, 8800, 198, 216,
    8734, 177, 8804, 8805, 165, 181, 8706, 8721,
    8719, 960, 8747, 170, 186, 937, 230, 248,
    191, 161, 172, 8730, 402, 8776, 8710, 171,
    187, 8230, 160, 192, 195, 213, 338, 339,
    8211, 8212, 8220, 8221, 8216, 8217, 247, 9674,
    255, 376, 8260, 8364, 8249, 8250, 64257, 64258,
    8225, 183, 8218, 8222, 8240, 194, 202, 193,
    203, 200, 205, 206, 207, 204, 211, 212,
    63743, 210, 218, 219, 217, 305, 710, 732,
    175, 728, 729, 730, 184, 733, 731, 711,
];

#[rustfmt::skip]
pub(crate) const STANDARD: [u32; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    32, 33, 34, 35, 36, 37, 38, 8217,
    40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55,
    56, 57, 58, 59, 60, 61, 62, 63,
    64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 94, 95,
    8216, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111,
    112, 113, 114, 115, 116, 117, 118, 119,
    120, 121, 122, 123, 124, 125, 126, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const AGL: [(&str, u32); 164] = [
    ("A", 0x0041), ("AE", 0x00C6), ("B", 0x0042), ("C", 0x0043), ("D", 0x0044),
    ("E", 0x0045), ("F", 0x0046), ("G", 0x0047), ("H", 0x0048), ("I", 0x0049),
    ("J", 0x004A), ("K", 0x004B), ("L", 0x004C), ("M", 0x004D), ("N", 0x004E),
    ("O", 0x004F), ("Oslash", 0x00D8), ("P", 0x0050), ("Q", 0x0051), ("R", 0x0052),
    ("S", 0x0053), ("T", 0x0054), ("U", 0x0055), ("V", 0x0056), ("W", 0x0057),
    ("X", 0x0058), ("Y", 0x0059), ("Z", 0x005A), ("a", 0x0061), ("aacute", 0x00E1),
    ("adieresis", 0x00E4), ("ae", 0x00E6), ("agrave", 0x00E0), ("ampersand", 0x0026),
    ("asciicircum", 0x005E), ("asciitilde", 0x007E), ("asterisk", 0x002A), ("at", 0x0040),
    ("b", 0x0062), ("backslash", 0x005C), ("bar", 0x007C), ("braceleft", 0x007B),
    ("braceright", 0x007D), ("bracketleft", 0x005B), ("bracketright", 0x005D),
    ("bullet", 0x2022), ("c", 0x0063), ("ccedilla", 0x00E7), ("cent", 0x00A2),
    ("colon", 0x003A), ("comma", 0x002C), ("copyright", 0x00A9), ("d", 0x0064),
    ("dagger", 0x2020), ("daggerdbl", 0x2021), ("degree", 0x00B0), ("divide", 0x00F7),
    ("dollar", 0x0024), ("e", 0x0065), ("eacute", 0x00E9), ("edieresis", 0x00EB),
    ("egrave", 0x00E8), ("eight", 0x0038), ("element", 0x2208), ("ellipsis", 0x2026), ("emdash", 0x2014),
    ("endash", 0x2013), ("equal", 0x003D), ("euro", 0x20AC), ("exclam", 0x0021),
    ("f", 0x0066), ("ff", 0xFB00), ("ffi", 0xFB03), ("ffl", 0xFB04), ("fi", 0xFB01),
    ("five", 0x0035), ("fl", 0xFB02), ("florin", 0x0192), ("fraction", 0x2044), ("four", 0x0034), ("g", 0x0067),
    ("germandbls", 0x00DF), ("grave", 0x0060), ("greater", 0x003E), ("greaterequal", 0x2265), ("guilsinglleft", 0x2039),
    ("guilsinglright", 0x203A), ("h", 0x0068), ("hyphen", 0x002D), ("i", 0x0069),
    ("iacute", 0x00ED), ("imaginaryunit", 0x2148), ("infinity", 0x221E), ("integral", 0x222B), ("intersection", 0x2229), ("j", 0x006A), ("k", 0x006B), ("l", 0x006C), ("less", 0x003C), ("lessequal", 0x2264),
    ("m", 0x006D), ("minus", 0x2212), ("multiply", 0x00D7), ("n", 0x006E),
    ("nbspace", 0x00A0), ("nine", 0x0039), ("notequal", 0x2260), ("ntilde", 0x00F1), ("numbersign", 0x0023),
    ("o", 0x006F), ("oacute", 0x00F3), ("odieresis", 0x00F6), ("one", 0x0031),
    ("oslash", 0x00F8), ("p", 0x0070), ("paragraph", 0x00B6), ("parenleft", 0x0028), ("parenleftinferior", 0x208D),
    ("parenright", 0x0029), ("parenrightinferior", 0x208E), ("percent", 0x0025), ("period", 0x002E), ("periodcentered", 0x00B7),
    ("perthousand", 0x2030), ("plus", 0x002B), ("product", 0x220F), ("q", 0x0071), ("question", 0x003F),
    ("quotedbl", 0x0022), ("quotedblbase", 0x201E), ("quotedblleft", 0x201C),
    ("quotedblright", 0x201D), ("quoteleft", 0x2018), ("quoteright", 0x2019),
    ("quotesinglbase", 0x201A), ("quotesingle", 0x0027), ("r", 0x0072), ("radical", 0x221A), ("registered", 0x00AE),
    ("s", 0x0073), ("section", 0x00A7), ("semicolon", 0x003B), ("seven", 0x0037),
    ("six", 0x0036), ("slash", 0x002F), ("space", 0x0020), ("sterling", 0x00A3), ("summation", 0x2211),
    ("t", 0x0074), ("three", 0x0033), ("trademark", 0x2122), ("two", 0x0032), ("u", 0x0075),
    ("uacute", 0x00FA), ("udieresis", 0x00FC), ("underscore", 0x005F), ("union", 0x222A), ("v", 0x0076),
    ("w", 0x0077), ("x", 0x0078), ("y", 0x0079), ("yen", 0x00A5), ("z", 0x007A),
    ("zero", 0x0030),
];

/// The base encoding table for a `/BaseEncoding` / `/Encoding` name (defaults to
/// WinAnsi, the most common simple-font encoding).
pub(crate) fn base_encoding(name: Option<&str>) -> &'static [u32; 256] {
    match name {
        Some("MacRomanEncoding") => &MAC_ROMAN,
        Some("StandardEncoding") => &STANDARD,
        _ => &WIN_ANSI,
    }
}

/// Map a glyph name to a Unicode scalar: AGL table, then the algorithmic
/// `uniXXXX` / `uXXXXXX` forms, ignoring any `.variant` suffix.
pub(crate) fn glyph_to_unicode(name: &str) -> Option<char> {
    let base = name.split('.').next().unwrap_or(name);
    agl_lookup(base).or_else(|| uni_form(base))
}

/// Binary-search the AGL subset.
fn agl_lookup(name: &str) -> Option<char> {
    let idx = AGL.binary_search_by(|(k, _)| (*k).cmp(name)).ok()?;
    char::from_u32(AGL[idx].1)
}

/// Decode `uniXXXX` (one BMP scalar) and `uXXXX`..`uXXXXXX` glyph names.
fn uni_form(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("uni") {
        return from_hex(hex.get(0..4)?);
    }
    let rest = name.strip_prefix('u')?;
    if (4..=6).contains(&rest.len()) {
        return from_hex(rest);
    }
    None
}

fn from_hex(hex: &str) -> Option<char> {
    char::from_u32(u32::from_str_radix(hex, 16).ok()?)
}

/// A codepoint table entry as a `char` (`0` entries are "undefined" → `None`).
#[cfg(test)]
pub(crate) fn table_char(table: &[u32; 256], code: u8) -> Option<char> {
    match table[code as usize] {
        0 => None,
        cp => char::from_u32(cp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winansi_high_range() {
        assert_eq!(table_char(&WIN_ANSI, b'A'), Some('A'));
        assert_eq!(table_char(&WIN_ANSI, 0x80), Some('€'));
        assert_eq!(table_char(&WIN_ANSI, 0x81), None); // undefined slot
    }

    #[test]
    fn macroman_and_standard() {
        assert_eq!(table_char(&MAC_ROMAN, 0xA5), Some('•'));
        assert_eq!(table_char(&STANDARD, b'A'), Some('A'));
        assert_eq!(table_char(&STANDARD, 0x27), Some('\u{2019}')); // quoteright
        assert_eq!(table_char(&STANDARD, 0x00), None);
    }

    #[test]
    fn base_encoding_selection() {
        assert_eq!(base_encoding(Some("MacRomanEncoding"))[0xA5], 8226);
        assert_eq!(base_encoding(Some("StandardEncoding"))[0x60], 8216);
        assert_eq!(base_encoding(Some("WinAnsiEncoding"))[0x80], 8364);
        assert_eq!(base_encoding(None)[b'A' as usize], 65);
    }

    #[test]
    fn glyph_names_from_agl() {
        assert_eq!(glyph_to_unicode("A"), Some('A'));
        assert_eq!(glyph_to_unicode("fi"), Some('\u{FB01}'));
        assert_eq!(glyph_to_unicode("bullet"), Some('•'));
        assert_eq!(glyph_to_unicode("space"), Some(' '));
        // A ".variant" suffix is stripped before lookup.
        assert_eq!(glyph_to_unicode("a.sc"), Some('a'));
        assert_eq!(glyph_to_unicode("notaglyphname"), None);
    }

    #[test]
    fn glyph_names_algorithmic() {
        assert_eq!(glyph_to_unicode("uni0041"), Some('A'));
        assert_eq!(glyph_to_unicode("u1F600"), char::from_u32(0x1F600));
        assert_eq!(glyph_to_unicode("uni00"), None); // too short
        assert_eq!(glyph_to_unicode("uABC"), None); // too short for u-form
        assert_eq!(glyph_to_unicode("uXYZW"), None); // not hex
    }

    #[test]
    fn math_symbols_from_agl() {
        assert_eq!(glyph_to_unicode("lessequal"), Some('≤'));
        assert_eq!(glyph_to_unicode("greaterequal"), Some('≥'));
        assert_eq!(glyph_to_unicode("notequal"), Some('≠'));
        assert_eq!(glyph_to_unicode("element"), Some('∈'));
        assert_eq!(glyph_to_unicode("union"), Some('∪'));
        assert_eq!(glyph_to_unicode("intersection"), Some('∩'));
        assert_eq!(glyph_to_unicode("radical"), Some('√'));
        assert_eq!(glyph_to_unicode("infinity"), Some('∞'));
        assert_eq!(glyph_to_unicode("summation"), Some('∑'));
        assert_eq!(glyph_to_unicode("product"), Some('∏'));
        assert_eq!(glyph_to_unicode("integral"), Some('∫'));
    }

    #[test]
    fn fraction_and_complex_symbols() {
        assert_eq!(glyph_to_unicode("fraction"), Some('⁄'));
        assert_eq!(glyph_to_unicode("imaginaryunit"), Some('ⅈ'));
        assert_eq!(glyph_to_unicode("parenleftinferior"), Some('₍'));
        assert_eq!(glyph_to_unicode("parenrightinferior"), Some('₎'));
    }
}
