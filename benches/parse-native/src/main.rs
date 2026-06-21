//! Rust-vs-Rust perf harness: turbo-parsepdf vs the Rust PDF stack.
//!
//! Times text extraction (best-of-N) on the shared corpus against
//! [`pdf-extract`](https://crates.io/crates/pdf-extract) (the common Rust text
//! extractor, built on `lopdf`) and a raw `lopdf` structural parse. Generate the
//! corpus first: `python3 benches/gen-corpus.py`. Run: `cargo run --release`.

use std::time::Instant;

fn best(mut f: impl FnMut(), n: usize) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..n {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    best
}

fn turbo(data: &[u8]) {
    let doc = turbo_parsepdf_core::Document::parse(data).unwrap();
    std::hint::black_box(doc.extract().unwrap().to_markdown());
}

fn pdf_extract(data: &[u8]) {
    std::hint::black_box(pdf_extract::extract_text_from_mem(data).unwrap());
}

fn lopdf_parse(data: &[u8]) {
    // Raw structural load only (lopdf has no high-level text extraction).
    let doc = lopdf::Document::load_mem(data).unwrap();
    std::hint::black_box(doc.get_pages().len());
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../competitive/corpus".to_string());
    println!("# turbo-parsepdf — Rust-native perf (best-of-20, ms)\n");
    println!("| file | turbo-parsepdf | pdf-extract | lopdf (parse only) |");
    println!("|---|---|---|---|");
    for name in ["small", "medium", "large"] {
        let path = format!("{dir}/{name}.pdf");
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("skip {path} (run `python3 benches/gen-corpus.py`)");
            continue;
        };
        let t = best(|| turbo(&data), 20);
        let p = best(|| pdf_extract(&data), 20);
        let l = best(|| lopdf_parse(&data), 20);
        println!("| {name} | {t:.2} | {p:.2} | {l:.2} |");
    }
}
