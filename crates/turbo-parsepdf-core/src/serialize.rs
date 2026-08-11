//! Output serializers: an extracted document → HTML, Markdown, or JSON.
//!
//! [`ExtractedDoc`] is the public result of [`crate::Document::extract`]: per-page
//! geometry + reconstructed lines (+ a `needs_ocr` flag for scanned pages). It
//! renders to three v1 formats — `to_json` (machine-readable, schema-aligned),
//! `to_markdown` (plain reading text), and `to_html` (one `<section>` per page).
//! A future version adds `.docx`.

use serde::Serialize;

use crate::layout::PageText;

/// The extracted content of a whole document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractedDoc {
    pub version: String,
    pub pages: Vec<PageText>,
}

impl ExtractedDoc {
    /// Serialize to pretty JSON (schema: `schema/turbo-parsepdf.doc.schema.json`).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Render to Markdown: each page's reading text, separated by a rule.
    pub fn to_markdown(&self) -> String {
        self.pages
            .iter()
            .map(page_markdown)
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    /// Render to a standalone HTML document, one `<section>` per page.
    pub fn to_html(&self) -> String {
        let mut s = String::from(
            "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>turbo-parsepdf</title></head>\n<body>\n",
        );
        for page in &self.pages {
            push_page_html(&mut s, page);
        }
        s.push_str("</body>\n</html>\n");
        s
    }
}

/// One page's Markdown (text + GitHub-style pipe tables, or an OCR placeholder).
fn page_markdown(page: &PageText) -> String {
    if page.needs_ocr {
        return "_[scanned page — text extraction requires OCR]_".to_string();
    }
    let mut parts = vec![page.text()];
    for table in &page.tables {
        parts.push(markdown_table(table));
    }
    parts.extend(page.images.iter().filter_map(image_markdown));
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One image as a Markdown image node, or `None` when it has no viewable bytes.
fn image_markdown(image: &crate::image::ParsedImage) -> Option<String> {
    if image.data_url.is_empty() {
        None
    } else {
        Some(format!("![{}]({})", image.name, image.data_url))
    }
}

/// Render one table as a GitHub-flavoured Markdown pipe table.
fn markdown_table(table: &crate::tables::Table) -> String {
    let mut rows: Vec<String> = table.cells.iter().map(|r| md_row(r)).collect();
    if table.rows > 0 && table.cols > 0 {
        rows.insert(1.min(rows.len()), md_divider(table.cols));
    }
    rows.join("\n")
}

fn md_row(cells: &[String]) -> String {
    format!(
        "| {} |",
        cells
            .iter()
            .map(|c| c.replace('|', "\\|"))
            .collect::<Vec<_>>()
            .join(" | ")
    )
}

fn md_divider(cols: usize) -> String {
    format!("| {} |", vec!["---"; cols].join(" | "))
}

/// Append one page's HTML section (paragraphs + tables).
fn push_page_html(s: &mut String, page: &PageText) {
    s.push_str(&format!(
        "<section class=\"page\" style=\"width:{:.0}px;height:{:.0}px\"{}>\n",
        page.width,
        page.height,
        ocr_attr(page),
    ));
    for line in &page.lines {
        s.push_str(&format!("<p>{}</p>\n", escape_html(&line.text)));
    }
    for table in &page.tables {
        push_table_html(s, table);
    }
    for image in &page.images {
        push_image_html(s, image);
    }
    s.push_str("</section>\n");
}

/// Append one image as an HTML `<img>` (skipped when it has no viewable bytes).
fn push_image_html(s: &mut String, image: &crate::image::ParsedImage) {
    if image.data_url.is_empty() {
        return;
    }
    s.push_str(&format!(
        "<img src=\"{}\" alt=\"{}\" />\n",
        image.data_url,
        escape_html(&image.name)
    ));
}

/// Append one table as an HTML `<table>`.
fn push_table_html(s: &mut String, table: &crate::tables::Table) {
    s.push_str("<table>\n");
    for row in &table.cells {
        s.push_str("<tr>");
        for cell in row {
            s.push_str(&format!("<td>{}</td>", escape_html(cell)));
        }
        s.push_str("</tr>\n");
    }
    s.push_str("</table>\n");
}

/// The `data-needs-ocr` attribute for a scanned page.
fn ocr_attr(page: &PageText) -> &'static str {
    if page.needs_ocr {
        " data-needs-ocr=\"true\""
    } else {
        ""
    }
}

/// Escape the five HTML-significant characters.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        push_escaped(&mut out, c);
    }
    out
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        _ => out.push(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Line;

    fn line(text: &str) -> Line {
        Line {
            text: text.into(),
            x: 0.0,
            y: 0.0,
        }
    }

    fn page(lines: Vec<Line>, needs_ocr: bool, tables: Vec<crate::tables::Table>) -> PageText {
        PageText {
            width: 300.0,
            height: 400.0,
            lines,
            needs_ocr,
            tables,
            images: vec![],
        }
    }

    fn doc() -> ExtractedDoc {
        ExtractedDoc {
            version: "1.7".into(),
            pages: vec![
                page(
                    vec![line("Hello <world>"), line("second & line")],
                    false,
                    vec![],
                ),
                page(vec![], true, vec![]),
            ],
        }
    }

    #[test]
    fn json_round_trips_shape() {
        let json = doc().to_json();
        assert!(json.contains("\"version\": \"1.7\""));
        assert!(json.contains("\"needs_ocr\": true"));
        assert!(json.contains("Hello <world>")); // JSON keeps raw text
    }

    #[test]
    fn markdown_text_and_ocr_placeholder() {
        let md = doc().to_markdown();
        assert!(md.contains("Hello <world>\nsecond & line"));
        assert!(md.contains("---")); // page separator
        assert!(md.contains("requires OCR"));
    }

    #[test]
    fn html_escapes_and_marks_ocr() {
        let html = doc().to_html();
        assert!(html.contains("<p>Hello &lt;world&gt;</p>"));
        assert!(html.contains("second &amp; line"));
        assert!(html.contains("width:300px;height:400px"));
        assert!(html.contains("data-needs-ocr=\"true\""));
        assert!(html.starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn tables_render_in_html_and_markdown() {
        let table = crate::tables::Table {
            rows: 2,
            cols: 2,
            cells: vec![vec!["a".into(), "b|c".into()], vec!["d".into(), "e".into()]],
        };
        let d = ExtractedDoc {
            version: "1".into(),
            pages: vec![page(vec![], false, vec![table])],
        };
        let html = d.to_html();
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>a</td><td>b|c</td>"));
        let md = d.to_markdown();
        assert!(md.contains("| a | b\\|c |")); // pipe escaped
        assert!(md.contains("| --- | --- |")); // header divider
        assert!(md.contains("| d | e |"));
    }

    #[test]
    fn escape_covers_all_specials() {
        assert_eq!(
            escape_html("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&#39;f"
        );
        assert_eq!(escape_html("plain"), "plain");
    }

    #[test]
    fn images_render_in_html_and_markdown() {
        let viewable = crate::image::ParsedImage {
            name: "Im0".into(),
            format: crate::image::ImageFormat::Jpeg,
            width: 2,
            height: 2,
            bits_per_component: 8,
            color_space: "DeviceRGB".into(),
            data_url: "data:image/jpeg;base64,AAAA".into(),
            data: vec![],
        };
        let skipped = crate::image::ParsedImage {
            name: "Im1".into(),
            format: crate::image::ImageFormat::Ccitt,
            width: 2,
            height: 2,
            bits_per_component: 1,
            color_space: "DeviceGray".into(),
            data_url: String::new(),
            data: vec![],
        };
        let d = ExtractedDoc {
            version: "1".into(),
            pages: vec![PageText {
                width: 100.0,
                height: 100.0,
                lines: vec![line("capt")],
                needs_ocr: false,
                tables: vec![],
                images: vec![viewable, skipped],
            }],
        };
        let html = d.to_html();
        assert!(html.contains("<img src=\"data:image/jpeg;base64,AAAA\" alt=\"Im0\" />"));
        assert!(!html.contains("Im1"));
        let md = d.to_markdown();
        assert!(md.contains("![Im0](data:image/jpeg;base64,AAAA)"));
        assert!(!md.contains("Im1"));
    }
}
