//! Minimal CLI example: `cargo run --example extract -- file.pdf [text|md|html|json]`.

use std::process::exit;

use turbo_parsepdf_core::Document;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| fail("usage: extract <file.pdf> [text|md|html|json]"));
    let format = args.next().unwrap_or_else(|| "text".to_string());
    let bytes = std::fs::read(&path).unwrap_or_else(|e| fail(&format!("read {path}: {e}")));
    let doc = Document::parse(&bytes).unwrap_or_else(|e| fail(&e.to_string()));
    let extracted = doc.extract().unwrap_or_else(|e| fail(&e.to_string()));
    match format.as_str() {
        "md" | "markdown" => println!("{}", extracted.to_markdown()),
        "html" => println!("{}", extracted.to_html()),
        "json" => println!("{}", extracted.to_json()),
        _ => print_text(&extracted),
    }
}

fn print_text(extracted: &turbo_parsepdf_core::ExtractedDoc) {
    for (i, page) in extracted.pages.iter().enumerate() {
        println!(
            "--- page {} ({:.0}x{:.0}) ---",
            i + 1,
            page.width,
            page.height
        );
        println!("{}", page.text());
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1);
}
