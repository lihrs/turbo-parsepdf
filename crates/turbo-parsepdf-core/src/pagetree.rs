//! Page-tree traversal (ISO 32000-1 §7.7.3).
//!
//! The catalog's `/Pages` is the root of a tree of `/Pages` nodes (with `/Kids`)
//! and `/Page` leaves. Several attributes — `/MediaBox`, `/Resources`,
//! `/Rotate` — are *inheritable*: a leaf without its own value uses the nearest
//! ancestor's. [`collect_pages`] flattens the tree into leaves in reading order
//! with those attributes resolved, and [`page_content`] concatenates a page's
//! `/Contents` into one decoded byte stream.

use std::collections::HashSet;

use crate::error::{ErrorCode, Result, TurboParsePdfError};
use crate::filter::decode_stream;
use crate::object::{Dictionary, Object};
use crate::resolver::Resolver;

fn bad(msg: &str) -> TurboParsePdfError {
    TurboParsePdfError::new(ErrorCode::InvalidObject, msg)
}

/// A flattened page leaf with its inherited attributes resolved.
#[derive(Debug, Clone)]
pub struct Page {
    pub dict: Dictionary,
    pub media_box: [f64; 4],
    pub resources: Dictionary,
    pub rotate: i64,
}

/// Inheritable attributes carried down the page tree.
#[derive(Debug, Clone)]
struct Inherited {
    media_box: [f64; 4],
    resources: Object,
    rotate: i64,
}

impl Inherited {
    /// The document-level defaults (US-Letter media box, no resources, 0°).
    fn root() -> Self {
        Inherited {
            media_box: [0.0, 0.0, 612.0, 792.0],
            resources: Object::Null,
            rotate: 0,
        }
    }

    /// Overlay this node's own attributes on the inherited ones.
    fn descend(&self, r: &Resolver, node: &Dictionary) -> Inherited {
        Inherited {
            media_box: read_rect(r, node).unwrap_or(self.media_box),
            resources: node
                .get("Resources")
                .cloned()
                .unwrap_or_else(|| self.resources.clone()),
            rotate: read_rotate(r, node).unwrap_or(self.rotate),
        }
    }
}

/// Flatten the page tree under `catalog` into ordered page leaves.
pub fn collect_pages(r: &Resolver, catalog: &Dictionary) -> Result<Vec<Page>> {
    let pages_obj = catalog
        .get("Pages")
        .ok_or_else(|| bad("catalog has no /Pages"))?;
    let root = r.resolve_dict(pages_obj)?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(r, &root, &Inherited::root(), &mut out, &mut seen, 0)?;
    Ok(out)
}

/// Visit one node: recurse a `/Pages` subtree, or emit a `/Page` leaf.
fn walk(
    r: &Resolver,
    node: &Dictionary,
    inh: &Inherited,
    out: &mut Vec<Page>,
    seen: &mut HashSet<u32>,
    depth: u32,
) -> Result<()> {
    if depth > 64 {
        return Err(bad("page tree too deep"));
    }
    let here = inh.descend(r, node);
    if node.get("Kids").is_some() {
        walk_kids(r, node, &here, out, seen, depth)
    } else {
        out.push(make_page(r, node, &here));
        Ok(())
    }
}

/// Recurse each child of a `/Pages` node.
fn walk_kids(
    r: &Resolver,
    node: &Dictionary,
    inh: &Inherited,
    out: &mut Vec<Page>,
    seen: &mut HashSet<u32>,
    depth: u32,
) -> Result<()> {
    let kids = node
        .get("Kids")
        .and_then(Object::as_array)
        .ok_or_else(|| bad("/Kids not array"))?;
    for kid in kids {
        walk_kid(r, kid, inh, out, seen, depth)?;
    }
    Ok(())
}

/// Resolve and recurse one child reference, guarding against cycles.
fn walk_kid(
    r: &Resolver,
    kid: &Object,
    inh: &Inherited,
    out: &mut Vec<Page>,
    seen: &mut HashSet<u32>,
    depth: u32,
) -> Result<()> {
    if let Some(rf) = kid.as_reference() {
        if !seen.insert(rf.num) {
            return Ok(());
        }
    }
    let kdict = r.resolve_dict(kid)?;
    walk(r, &kdict, inh, out, seen, depth + 1)
}

/// Build a page leaf, resolving its inherited resources dictionary.
fn make_page(r: &Resolver, node: &Dictionary, inh: &Inherited) -> Page {
    let resources = r.resolve_dict(&inh.resources).unwrap_or_default();
    Page {
        dict: node.clone(),
        media_box: inh.media_box,
        resources,
        rotate: inh.rotate,
    }
}

/// Read a node's `/MediaBox` as four numbers.
fn read_rect(r: &Resolver, node: &Dictionary) -> Option<[f64; 4]> {
    let resolved = r.resolve(node.get("MediaBox")?).ok()?;
    let arr = resolved.as_array()?;
    let v: Vec<f64> = arr.iter().filter_map(Object::as_f64).collect();
    match v.as_slice() {
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

/// Read a node's `/Rotate` (normalized into `0..360`, multiple of 90).
fn read_rotate(r: &Resolver, node: &Dictionary) -> Option<i64> {
    let n = r.resolve(node.get("Rotate")?).ok()?.as_integer()?;
    Some(n.rem_euclid(360))
}

/// Concatenate and decode a page's `/Contents` into one content-stream buffer.
pub fn page_content(r: &Resolver, page: &Page) -> Result<Vec<u8>> {
    let contents = match page.dict.get("Contents") {
        Some(c) => c.clone(),
        None => return Ok(Vec::new()),
    };
    match r.resolve(&contents)? {
        Object::Stream(s) => decode_stream(&s.dict, &s.data),
        Object::Array(items) => concat_streams(r, &items),
        _ => Ok(Vec::new()),
    }
}

/// Concatenate an array of content streams, separated by newlines.
fn concat_streams(r: &Resolver, items: &[Object]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for item in items {
        append_stream(r, item, &mut out)?;
    }
    Ok(out)
}

/// Resolve, decode, and append one content stream.
fn append_stream(r: &Resolver, item: &Object, out: &mut Vec<u8>) -> Result<()> {
    if let Object::Stream(s) = r.resolve(item)? {
        out.extend_from_slice(&decode_stream(&s.dict, &s.data)?);
        out.push(b'\n');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use crate::object::ObjRef;

    // Build a 2-page PDF: catalog → Pages(2 kids) with inherited MediaBox, each
    // page carrying a small content stream.
    fn two_page_pdf() -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.5\n");
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
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 300] /Rotate 90 >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut offs,
            "4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 99 99] /Contents 6 0 R >>\nendobj\n",
        );
        let c1 = "BT /F1 12 Tf 10 280 Td (Page one) Tj ET";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "5 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                c1.len(),
                c1
            ),
        );
        let c2 = "BT /F1 12 Tf 10 80 Td (Page two) Tj ET";
        put(
            &mut pdf,
            &mut offs,
            &format!(
                "6 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                c2.len(),
                c2
            ),
        );
        let xref_off = pdf.len();
        pdf.push_str("xref\n0 7\n0000000000 65535 f \n");
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str("trailer\n<< /Size 7 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xref_off}\n%%EOF"));
        pdf.into_bytes()
    }

    #[test]
    fn collects_pages_with_inheritance() {
        let data = two_page_pdf();
        let doc = Document::parse(&data).unwrap();
        let pages = collect_pages(doc.resolver(), &doc.catalog().unwrap()).unwrap();
        assert_eq!(pages.len(), 2);
        // Page 1 inherits MediaBox + Rotate from the Pages node.
        assert_eq!(pages[0].media_box, [0.0, 0.0, 200.0, 300.0]);
        assert_eq!(pages[0].rotate, 90);
        // Page 2 overrides MediaBox but still inherits Rotate.
        assert_eq!(pages[1].media_box, [0.0, 0.0, 99.0, 99.0]);
        assert_eq!(pages[1].rotate, 90);
    }

    #[test]
    fn assembles_page_content() {
        let data = two_page_pdf();
        let doc = Document::parse(&data).unwrap();
        let pages = collect_pages(doc.resolver(), &doc.catalog().unwrap()).unwrap();
        let bytes = page_content(doc.resolver(), &pages[0]).unwrap();
        assert!(bytes.windows(8).any(|w| w == b"Page one"));
    }

    #[test]
    fn content_array_is_concatenated() {
        // A page whose /Contents is an array of two streams.
        let mut pdf = String::from("%PDF-1.5\n");
        let mut o = Vec::new();
        let put = |p: &mut String, o: &mut Vec<usize>, s: &str| {
            o.push(p.len());
            p.push_str(s);
        };
        put(
            &mut pdf,
            &mut o,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut o,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut o,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents [4 0 R 5 0 R] >>\nendobj\n",
        );
        put(
            &mut pdf,
            &mut o,
            "4 0 obj\n<< /Length 1 >>\nstream\nq\nendstream\nendobj\n",
        );
        put(
            &mut pdf,
            &mut o,
            "5 0 obj\n<< /Length 1 >>\nstream\nQ\nendstream\nendobj\n",
        );
        let xo = pdf.len();
        pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
        for off in &o {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str("trailer\n<< /Size 6 /Root 1 0 R >>\n");
        pdf.push_str(&format!("startxref\n{xo}\n%%EOF"));
        let data = pdf.into_bytes();
        let doc = Document::parse(&data).unwrap();
        let pages = collect_pages(doc.resolver(), &doc.catalog().unwrap()).unwrap();
        let bytes = page_content(doc.resolver(), &pages[0]).unwrap();
        assert_eq!(bytes, b"q\nQ\n");
    }

    #[test]
    fn no_contents_is_empty() {
        let data = two_page_pdf();
        let doc = Document::parse(&data).unwrap();
        let mut page = collect_pages(doc.resolver(), &doc.catalog().unwrap()).unwrap()[0].clone();
        page.dict = Dictionary::new();
        assert!(page_content(doc.resolver(), &page).unwrap().is_empty());
    }

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

    fn doc_pages(objs: &[String]) -> Result<Vec<Page>> {
        let data = assemble(objs);
        let doc = Document::parse(&data).unwrap();
        collect_pages(doc.resolver(), &doc.catalog().unwrap())
    }

    #[test]
    fn over_deep_tree_errors() {
        let mut objs = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
        for i in 2..=67 {
            objs.push(format!("<< /Type /Pages /Kids [{} 0 R] /Count 1 >>", i + 1));
        }
        let data = assemble(&objs);
        let doc = Document::parse(&data).unwrap();
        assert!(collect_pages(doc.resolver(), &doc.catalog().unwrap()).is_err());
    }

    #[test]
    fn kids_cycle_is_guarded() {
        let objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Pages /Kids [2 0 R] /Count 1 >>".to_string(),
        ];
        // The cycle terminates and yields no leaves rather than looping.
        assert!(doc_pages(&objs).unwrap().is_empty());
    }

    #[test]
    fn malformed_media_box_falls_back() {
        let objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 9 9] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 5] >>".to_string(),
        ];
        let pages = doc_pages(&objs).unwrap();
        // The 3-element MediaBox is rejected; the inherited box is used.
        assert_eq!(pages[0].media_box, [0.0, 0.0, 9.0, 9.0]);
    }

    #[test]
    fn non_stream_contents_is_empty() {
        let objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
            "42".to_string(), // /Contents resolves to a non-stream object
        ];
        let data = assemble(&objs);
        let doc = Document::parse(&data).unwrap();
        let pages = collect_pages(doc.resolver(), &doc.catalog().unwrap()).unwrap();
        assert!(page_content(doc.resolver(), &pages[0]).unwrap().is_empty());
    }

    #[test]
    fn missing_pages_errors() {
        let mut d = Dictionary::new();
        d.insert("X", Object::Integer(1));
        let data = two_page_pdf();
        let doc = Document::parse(&data).unwrap();
        assert!(collect_pages(doc.resolver(), &d).is_err());
        // A non-array /Kids is rejected.
        let mut bad_root = Dictionary::new();
        bad_root.insert("Pages", Object::Reference(ObjRef::new(999, 0)));
        assert!(collect_pages(doc.resolver(), &bad_root).is_err());
    }
}
