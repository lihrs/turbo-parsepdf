//! Best-of-N timing of the full extract pipeline: `cargo run --release --example timeit -- file.pdf`.

use std::time::Instant;

use turbo_parsepdf_core::Document;

fn main() {
    let path = std::env::args().nth(1).expect("usage: timeit <file.pdf>");
    let data = std::fs::read(&path).expect("read");
    let mut best = f64::MAX;
    let mut pages = 0;
    for _ in 0..50 {
        let t = Instant::now();
        let doc = Document::parse(&data).expect("parse");
        let extracted = doc.extract().expect("extract");
        pages = extracted.pages.len();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    println!("turbo-parsepdf:   {best:8.3} ms  ({pages} pages)");
}
