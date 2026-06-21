# Changelog

All notable changes to turbo-parsepdf are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/); the project uses semantic
versioning across the workspace (one shared `X.Y.Z`).

## [0.1.0] — unreleased

First release. A native PDF text/table/image extractor with HTML/Markdown/JSON
output, shipped as a Rust crate plus N-API, PyO3, wasm, and MCP bindings.

### Added

- **Core parser** (`turbo-parsepdf-core`):
  - COS lexer + recursive-descent object parser.
  - Cross-reference **tables and streams** (PDF 1.5+), **object streams**, hybrid
    `/XRefStm`, and `/Prev` incremental updates.
  - Lazy, cached indirect-object resolver.
  - Stream filters: `FlateDecode`, `LZWDecode`, `ASCII85Decode`, `ASCIIHexDecode`,
    `RunLengthDecode`, with PNG/TIFF predictors (all hand-rolled).
  - Page-tree traversal with inherited attributes; content assembly.
  - Content-stream interpreter (CTM + text state); positioned text runs.
  - Font decoding: `/ToUnicode` CMaps, simple encodings (WinAnsi/MacRoman/Standard)
    + `/Differences` via the Adobe Glyph List, Type0/CID, glyph widths.
  - Layout reconstruction (lines/words, reading order); `needs_ocr` flag.
  - Ruled-table detection; image XObject extraction (JPEG passthrough + raw).
  - `Document::extract` → `ExtractedDoc` with `to_html` / `to_markdown` / `to_json`.
  - Optional `encrypt` feature: standard security handler decryption (RC4 +
    AES-128/256, R2–R6, empty/user/owner password).
- **Bindings**: `turbo-parsepdf` (npm/N-API), `turbo-parsepdf` (PyPI/PyO3),
  `turbo-parsepdf-wasm` (browser), `turbo-parsepdf-mcp` (MCP server with
  `parse_pdf` / `inspect_pdf` / `extract_tables` / `extract_images`).

### Performance

- ~9.7× faster than pdf.js, 38× pypdf, 62× PyMuPDF, 307× pdfminer on a 100-page
  document; extracted text byte-identical to PyMuPDF.

### Out of scope (planned)

- OCR of scanned/image-only pages (flagged `needs_ocr`).
- `.docx` output.

[0.1.0]: https://github.com/miaskiewicz/turbo-parsepdf/releases/tag/v0.1.0
