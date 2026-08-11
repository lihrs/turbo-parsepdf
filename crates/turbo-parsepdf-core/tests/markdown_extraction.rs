//! Test markdown extraction from 123.pdf.

use std::fs;
use std::path::Path;
use turbo_parsepdf_core::Document;

const PDF_123: &[u8] = include_bytes!("./123.pdf");

fn save_markdown(markdown: &str, filename: &str) {
    let output_dir = Path::new("output");
    fs::create_dir_all(output_dir).ok();
    let path = output_dir.join(filename);
    fs::write(&path, markdown).expect("failed to write markdown file");
    println!("✓ Saved: {}", path.display());
}

#[test]
fn extract_markdown_from_123_pdf() {
    let doc = Document::parse(PDF_123).expect("failed to parse 123.pdf");
    let extracted = doc.extract().expect("failed to extract content");
    let markdown = extracted.to_markdown();

    // Verify markdown is not empty
    assert!(!markdown.is_empty(), "markdown output should not be empty");

    // Verify it contains expected markdown elements
    assert!(
        markdown.contains("# ") || markdown.contains("## ") || markdown.contains("---"),
        "markdown should contain headers or page breaks"
    );

    // Save to output directory
    save_markdown(&markdown, "123.md");
}

#[test]
fn markdown_structure_from_123_pdf() {
    let doc = Document::parse(PDF_123).expect("failed to parse 123.pdf");
    let extracted = doc.extract().expect("failed to extract content");
    let markdown = extracted.to_markdown();

    // Verify page breaks (---)
    let page_count = extracted.pages.len();
    let page_breaks = markdown.matches("---").count();

    // Each page except the last should have a break
    assert!(
        page_breaks >= page_count.saturating_sub(1),
        "expected at least {} page breaks, got {}",
        page_count.saturating_sub(1),
        page_breaks
    );
}

#[test]
fn markdown_preserves_text_content() {
    let doc = Document::parse(PDF_123).expect("failed to parse 123.pdf");
    let extracted = doc.extract().expect("failed to extract content");
    let markdown = extracted.to_markdown();

    // Get plain text for comparison
    let plain_text = extracted.pages.iter().map(|p| p.text()).collect::<String>();

    // Markdown should contain at least as much text as plain extraction
    let plain_text_len = plain_text.split_whitespace().count();
    let markdown_text_len = markdown.split_whitespace().count();

    assert!(
        markdown_text_len >= plain_text_len * 80 / 100,
        "markdown should preserve most text content"
    );
}
