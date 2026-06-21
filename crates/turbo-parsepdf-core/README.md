# turbo-parsepdf-core

Native, dependency-light **PDF text / table / image extraction** core. Pure Rust;
the N-API, PyO3, and wasm bindings are thin marshaling shims over this crate.

Reads a PDF byte buffer and extracts:

- **Text** + positioned tokens (words/chars with bounding boxes and font).
- **Tables** (ruled tables, from positioned text + line graphics).
- **Embedded images** (image XObjects pulled out as JPEG/raster bytes).

Image-only (scanned) pages with no extractable text layer are detected and flagged
`needs_ocr` rather than OCR'd. Output serializes to **HTML**, **Markdown**, and
**JSON** (`.docx` is a future target).

## Pipeline

```
bytes → lex/cos (COS object parser)
      → xref/resolver (cross-reference chain, lazy indirect objects)
      → filter (FlateDecode + the other stream filters)
      → pagetree → content interpreter → font/Unicode decode
      → layout (words/lines/reading order) → tables/images
      → serialize (HTML / Markdown / JSON)
```

## Status

Complete: COS parser; cross-reference **tables and streams** (PDF 1.5+), object
streams, hybrid `/XRefStm`, `/Prev`; all standard stream filters
(`Flate`/`LZW`/`ASCII85`/`ASCIIHex`/`RunLength`) + PNG/TIFF predictors; the
content-stream interpreter; font decoding (`/ToUnicode`, encodings + AGL, CID);
layout reconstruction; ruled tables; image XObject extraction; HTML/Markdown/JSON
serializers; and (behind the optional `encrypt` feature) standard-handler
decryption (RC4 + AES-128/256, R2–R6).

```rust
use turbo_parsepdf_core::Document;
let extracted = Document::parse(&bytes)?.extract()?;
println!("{}", extracted.to_markdown());
```

## Performance vs the Rust PDF stack

Best-of-N text extraction (Apple M-series, release). Reproduce:
`cd benches/parse-native && cargo run --release` (after `python3 benches/gen-corpus.py`).

| document | **turbo-parsepdf** | pdf-extract | lopdf (parse only) |
|---|---|---|---|
| 100 pages | **4.7 ms** | 275 ms · **59× faster** | 0.23 ms¹ |
| 20 pages | **0.9 ms** | 52 ms · **59× faster** | 0.10 ms¹ |

¹ `lopdf` only loads object structure (no text/font/layout extraction), so it is a
floor, not a same-task competitor. Across ecosystems turbo is also ~9.7× faster
than pdf.js and 38–307× faster than the Python stack (`benches/`); extracted text
is byte-identical to PyMuPDF.

## Engineering gates

`cargo fmt` · `cargo clippy -D warnings` · `cargo test` · `cc-check` (cyclomatic
complexity < 6) · `cargo tarpaulin` (**100%** line coverage on the gated surface).
The ported DEFLATE decoder (`filter/inflate.rs`) and the crypto module
(`crypt.rs`) are validated functionally and excluded from line instrumentation,
as in the sibling turbo-xlsx.
