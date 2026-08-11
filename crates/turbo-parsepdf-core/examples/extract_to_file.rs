//! Extract PDF to markdown file in the specified directory.
//! Usage: cargo run --release --example extract_to_file -- 123.pdf output_dir

use std::fs;
use std::path::Path;
use std::process::exit;

use turbo_parsepdf_core::Document;

fn main() {
    let mut args = std::env::args().skip(1);
    let pdf_path = args
        .next()
        .unwrap_or_else(|| fail("usage: extract_to_file <file.pdf> <output_dir>"));
    let output_dir = args
        .next()
        .unwrap_or_else(|| fail("usage: extract_to_file <file.pdf> <output_dir>"));

    // Read PDF
    let bytes = fs::read(&pdf_path).unwrap_or_else(|e| fail(&format!("read {pdf_path}: {e}")));

    // Parse and extract
    let doc = Document::parse(&bytes).unwrap_or_else(|e| fail(&e.to_string()));
    let extracted = doc.extract().unwrap_or_else(|e| fail(&e.to_string()));

    // Create output directory if it doesn't exist
    fs::create_dir_all(&output_dir).unwrap_or_else(|e| {
        fail(&format!("create directory {output_dir}: {e}"));
    });

    // Extract markdown and save to file
    let markdown = extracted.to_markdown();
    let file_stem = Path::new(&pdf_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let output_path = format!("{}/{}.md", output_dir, file_stem);

    fs::write(&output_path, &markdown).unwrap_or_else(|e| {
        fail(&format!("write {output_path}: {e}"));
    });

    println!("✓ Extracted to: {}", output_path);
    println!("  Size: {} bytes, {} lines", markdown.len(), markdown.lines().count());
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1);
}
