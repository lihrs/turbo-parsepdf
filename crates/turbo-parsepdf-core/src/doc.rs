//! The public document entry point.
//!
//! [`Document::parse`] validates the `%PDF-` header, reads the cross-reference
//! chain, and exposes a lazy [`Resolver`] over the body. At this phase the
//! surface is deliberately low-level (version, trailer, catalog, raw object
//! fetch); the page tree, content extraction, and the HTML/Markdown/JSON
//! serializers build on top of it in later phases.

use crate::content::parse_content;
use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::font::load_fonts;
use crate::image::extract_images;
use crate::layout::{layout_page, PageText};
use crate::object::{Dictionary, ObjRef, Object};
use crate::pagetree::{collect_pages, page_content, Page};
use crate::resolver::Resolver;
use crate::serialize::ExtractedDoc;
use crate::tables::detect_tables;
use crate::text::{extract_runs, TextRun};
use crate::xref::read_xref;

/// A parsed PDF document: a validated header plus a resolver over the body.
#[derive(Debug)]
pub struct Document<'a> {
    resolver: Resolver<'a>,
    version: String,
}

impl<'a> Document<'a> {
    /// Parse `data` into a document (trying an empty password for any
    /// encryption). For password-protected files, use [`Document::parse_with_password`].
    pub fn parse(data: &'a [u8]) -> Result<Document<'a>> {
        Document::parse_with_password(data, "")
    }

    /// Parse `data`, supplying a user or owner `password` for encrypted files.
    /// The password is ignored for unencrypted documents and when the `encrypt`
    /// feature is disabled.
    pub fn parse_with_password(data: &'a [u8], password: &str) -> Result<Document<'a>> {
        let version = parse_header(data)?;
        let xref = read_xref(data)?;
        require_root(xref.trailer())?;
        let mut resolver = Resolver::new(data, xref);
        setup_decryption(&mut resolver, password.as_bytes())?;
        Ok(Document { resolver, version })
    }

    /// The PDF version string from the header (e.g. `1.4`, `2.0`).
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The document trailer dictionary.
    pub fn trailer(&self) -> &Dictionary {
        self.resolver.xref().trailer()
    }

    /// The number of cross-reference entries (a coarse object count).
    pub fn object_count(&self) -> usize {
        self.resolver.xref().len()
    }

    /// Fetch an indirect object by reference.
    pub fn get(&self, r: ObjRef) -> Result<Object> {
        self.resolver.get(r)
    }

    /// Resolve a possibly-indirect value to a concrete object.
    pub fn resolve(&self, obj: &Object) -> Result<Object> {
        self.resolver.resolve(obj)
    }

    /// The document catalog (`/Root`), resolved to its dictionary.
    pub fn catalog(&self) -> Result<Dictionary> {
        let root = self.trailer().get("Root").ok_or_else(missing_root)?.clone();
        self.resolver.resolve_dict(&root)
    }

    /// Borrow the underlying resolver (used by higher layers).
    pub fn resolver(&self) -> &Resolver<'a> {
        &self.resolver
    }

    /// The document's pages, flattened in reading order with inherited
    /// attributes (`/MediaBox`, `/Resources`, `/Rotate`) resolved.
    pub fn pages(&self) -> Result<Vec<Page>> {
        collect_pages(&self.resolver, &self.catalog()?)
    }

    /// The positioned text runs of one page (content assembled, decoded with the
    /// page's fonts, and interpreted).
    pub fn page_text_runs(&self, page: &Page) -> Result<Vec<TextRun>> {
        let content = page_content(&self.resolver, page)?;
        let fonts = load_fonts(&self.resolver, &page.resources);
        Ok(extract_runs(&parse_content(&content), &fonts))
    }

    /// Extract the whole document: every page's reconstructed lines, tables,
    /// images, and geometry, ready to serialize to HTML / Markdown / JSON.
    pub fn extract(&self) -> Result<ExtractedDoc> {
        let pages = self.pages()?;
        let mut out = Vec::with_capacity(pages.len());
        for page in &pages {
            out.push(self.extract_page(page)?);
        }
        Ok(ExtractedDoc {
            version: self.version.clone(),
            pages: out,
        })
    }

    /// Extract one page: text lines, ruled tables, image XObjects, and the
    /// `needs_ocr` flag (text-free pages carrying images are likely scanned).
    fn extract_page(&self, page: &Page) -> Result<PageText> {
        let content = page_content(&self.resolver, page)?;
        let ops = parse_content(&content);
        let fonts = load_fonts(&self.resolver, &page.resources);
        let runs = extract_runs(&ops, &fonts);
        let mut text = layout_page(&runs, page.media_box);
        text.tables = detect_tables(&ops, &runs);
        text.images = extract_images(&self.resolver, &page.resources);
        text.needs_ocr = text.lines.is_empty() && !text.images.is_empty();
        Ok(text)
    }
}

/// Locate and parse the `%PDF-x.y` header within the first 1024 bytes.
fn parse_header(data: &[u8]) -> Result<String> {
    let sig = b"%PDF-";
    let scan = &data[..data.len().min(1024)];
    let at = scan
        .windows(sig.len())
        .position(|w| w == sig)
        .ok_or_else(|| TurboParsePdfError::new(ErrorCode::InvalidHeader, "no %PDF- header"))?;
    Ok(read_version(&data[at + sig.len()..]))
}

/// Read the version run (`1.4`, `2.0`, …) up to the first whitespace/EOL.
fn read_version(after: &[u8]) -> String {
    let end = after
        .iter()
        .position(|&b| b" \t\r\n".contains(&b))
        .unwrap_or(after.len());
    String::from_utf8_lossy(&after[..end.min(8)]).into_owned()
}

/// Require the trailer to carry a `/Root` entry.
fn require_root(trailer: &Dictionary) -> Result<()> {
    if trailer.get("Root").is_some() {
        Ok(())
    } else {
        Err(missing_root())
    }
}

fn missing_root() -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::MissingTrailer, "trailer has no /Root")
}

/// Configure transparent decryption from the trailer `/Encrypt` dictionary.
#[cfg(feature = "encrypt")]
fn setup_decryption(resolver: &mut Resolver, password: &[u8]) -> Result<()> {
    let trailer = resolver.xref().trailer().clone();
    let Some(enc) = trailer.get("Encrypt") else {
        return Ok(());
    };
    let exempt = enc.as_reference().map(|r| r.num);
    let enc_dict = resolver.resolve_dict(enc)?;
    let id0 = first_file_id(&trailer);
    if let Some(dec) = crate::crypt::Decryptor::from_encrypt(&enc_dict, &id0, password) {
        resolver.set_decryptor(dec, exempt);
    }
    Ok(())
}

/// The first element of the file `/ID` array (used in key derivation).
#[cfg(feature = "encrypt")]
fn first_file_id(trailer: &Dictionary) -> Vec<u8> {
    trailer
        .get("ID")
        .and_then(Object::as_array)
        .and_then(<[Object]>::first)
        .and_then(Object::as_string)
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}

/// No-op when the `encrypt` feature is disabled.
#[cfg(not(feature = "encrypt"))]
fn setup_decryption(_resolver: &mut Resolver, _password: &[u8]) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.7\n");
        let off1 = pdf.len();
        pdf.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let off2 = pdf.len();
        pdf.push_str("2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj\n");
        let xoff = pdf.len();
        pdf.push_str("xref\n0 3\n0000000000 65535 f \n");
        pdf.push_str(&format!("{off1:010} 00000 n \n{off2:010} 00000 n \n"));
        pdf.push_str("trailer\n<< /Size 3 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        pdf.into_bytes()
    }

    // A one-page PDF carrying a content stream that shows "Hello".
    fn one_page_pdf() -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offs = Vec::new();
        let put = |pdf: &mut String, offs: &mut Vec<usize>, s: &str| {
            offs.push(pdf.len());
            pdf.push_str(s);
        };
        put(
            &mut pdf,
            &mut offs,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n",
        );
        let c = "BT /F1 12 Tf 20 360 Td (Hello) Tj ET";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                c.len(),
                c
            ),
        );
        let xoff = pdf.len();
        pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str("trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        pdf.into_bytes()
    }

    // A one-page PDF whose page references a real font (object 5) with a
    // ToUnicode map, so text extraction goes through the font decoder.
    fn page_with_font_pdf() -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offs = Vec::new();
        let put = |pdf: &mut String, offs: &mut Vec<usize>, s: &str| {
            offs.push(pdf.len());
            pdf.push_str(s);
        };
        put(
            &mut pdf,
            &mut offs,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        );
        put(&mut pdf, &mut offs, "3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n");
        let c = "BT /F1 12 Tf 20 700 Td (AB) Tj ET";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                c.len(),
                c
            ),
        );
        put(&mut pdf, &mut offs, "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [700 700] >>\nendobj\n");
        let xoff = pdf.len();
        pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str("trailer\n<< /Size 6 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        pdf.into_bytes()
    }

    #[test]
    fn extract_to_all_formats() {
        let data = page_with_font_pdf();
        let doc = Document::parse(&data).unwrap();
        let extracted = doc.extract().unwrap();
        assert_eq!(extracted.version, "1.7");
        assert_eq!(extracted.pages.len(), 1);
        assert_eq!(extracted.pages[0].lines[0].text, "AB");
        assert!(extracted.to_html().contains("<p>AB</p>"));
        assert!(extracted.to_markdown().contains("AB"));
        assert!(extracted.to_json().contains("\"AB\""));
    }

    #[test]
    fn text_runs_use_tounicode_font() {
        // A page font with a /ToUnicode CMap (code 0x41 → 'X'); extraction's hot
        // path (show_into → append_text) must use it over the base encoding.
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offs = Vec::new();
        let put = |pdf: &mut String, offs: &mut Vec<usize>, s: &str| {
            offs.push(pdf.len());
            pdf.push_str(s);
        };
        put(
            &mut pdf,
            &mut offs,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        );
        put(&mut pdf, &mut offs, "3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n");
        let c = "BT /F1 12 Tf 20 700 Td (A) Tj ET";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                c.len(),
                c
            ),
        );
        put(
            &mut pdf,
            &mut offs,
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /ToUnicode 6 0 R >>\nendobj\n",
        );
        let cmap = "/CIDInit begincmap 1 beginbfchar <41> <0058> endbfchar endcmap";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "6 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                cmap.len(),
                cmap
            ),
        );
        let xoff = pdf.len();
        pdf.push_str("xref\n0 7\n0000000000 65535 f \n");
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str("trailer\n<< /Size 7 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        let data = pdf.into_bytes();
        let doc = Document::parse(&data).unwrap();
        let runs = doc.page_text_runs(&doc.pages().unwrap()[0]).unwrap();
        assert_eq!(runs[0].text, "X");
    }

    #[test]
    fn text_runs_use_page_font() {
        let data = page_with_font_pdf();
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        let runs = doc.page_text_runs(&pages[0]).unwrap();
        assert_eq!(runs[0].text, "AB");
        assert_eq!((runs[0].x, runs[0].y), (20.0, 700.0));
    }

    #[test]
    fn pages_and_text_runs() {
        let data = one_page_pdf();
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].media_box, [0.0, 0.0, 300.0, 400.0]);
        let runs = doc.page_text_runs(&pages[0]).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "Hello");
        assert_eq!((runs[0].x, runs[0].y), (20.0, 360.0));
    }

    #[test]
    fn parses_and_exposes_surface() {
        let data = sample();
        let doc = Document::parse(&data).unwrap();
        assert_eq!(doc.version(), "1.7");
        assert_eq!(doc.object_count(), 3);
        assert_eq!(doc.trailer().get("Size").unwrap().as_integer(), Some(3));
        let cat = doc.catalog().unwrap();
        assert_eq!(cat.get("Type").unwrap().as_name(), Some("Catalog"));
        // get + resolve reach the Pages object.
        let pages = doc.get(ObjRef::new(2, 0)).unwrap();
        assert_eq!(
            pages.as_dict().unwrap().get("Type").unwrap().as_name(),
            Some("Pages")
        );
        let pages_ref = cat.get("Pages").unwrap().clone();
        assert_eq!(doc.resolve(&pages_ref).unwrap(), pages);
        assert!(doc.resolver().xref().get(1).is_some());
    }

    #[test]
    fn header_scan_tolerates_leading_junk() {
        // The header may appear after a few bytes of preamble (BOM, comment).
        // (Full-file parse with leading junk needs offset compensation — phase 2.)
        assert_eq!(
            parse_header(b"\xef\xbb\xbfjunk\n%PDF-1.5\nbody").unwrap(),
            "1.5"
        );
    }

    #[test]
    fn missing_header_errors() {
        let err = Document::parse(b"not a pdf at all").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidHeader);
    }

    #[test]
    fn missing_root_errors() {
        let mut pdf = String::from("%PDF-1.4\n");
        let off1 = pdf.len();
        pdf.push_str("1 0 obj\n<< >>\nendobj\n");
        let xoff = pdf.len();
        pdf.push_str("xref\n0 2\n0000000000 65535 f \n");
        pdf.push_str(&format!("{off1:010} 00000 n \n"));
        pdf.push_str("trailer\n<< /Size 2 >>\n");
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        let data = pdf.into_bytes();
        assert_eq!(
            Document::parse(&data).unwrap_err().code,
            ErrorCode::MissingTrailer
        );
    }

    #[test]
    fn version_run_stops_at_whitespace_and_clamps() {
        assert_eq!(read_version(b"1.4\nrest"), "1.4");
        assert_eq!(read_version(b"2.0"), "2.0");
        assert_eq!(read_version(b"1.234567890123"), "1.234567"); // clamped to 8
    }

    #[test]
    fn catalog_requires_root_present() {
        // require_root passes/fails as expected.
        let mut d = Dictionary::new();
        assert!(require_root(&d).is_err());
        d.insert("Root", Object::Reference(ObjRef::new(1, 0)));
        assert!(require_root(&d).is_ok());
    }
}
