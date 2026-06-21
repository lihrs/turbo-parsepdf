# turbo-parsepdf

Fast native **PDF text / table / image extraction** for Node — a pure-Rust core
(N-API). Output as a structured object, **HTML**, **Markdown**, or **JSON**.

```sh
npm install turbo-parsepdf
```

## Benchmark vs pdf.js

Wall-clock to extract every page's text, best-of-N (Apple M-series, release).
Reproduce: `node benches/competitive/bench.mjs` (after `python3 benches/gen-corpus.py`).

| document | **turbo-parsepdf** | pdf.js (`pdf-parse`) |
|---|---|---|
| 100 pages | **5.6 ms** | 54 ms · **9.7× faster** |
| 20 pages | **1.0 ms** | 8.3 ms · **8.3× faster** |

Extracted text is **byte-identical to PyMuPDF** (100% word recall) — fast *and*
correct.

```js
import { parse, parseToMarkdown, parseToHtml, parseToJson } from "turbo-parsepdf";
import { readFileSync } from "node:fs";

const pdf = readFileSync("doc.pdf");

const doc = parse(pdf);
// { version: "1.7", pages: [{ width, height, needs_ocr,
//   lines: [{ text, x, y }], tables: [{ rows, cols, cells }],
//   images: [{ name, format, width, height, bits_per_component, color_space }] }] }

parseToMarkdown(pdf); // string — reading text + pipe tables, page-broken
parseToHtml(pdf);     // string — one <section> per page
parseToJson(pdf);     // string — the doc above, pretty-printed

// Encrypted PDFs: pass the user or owner password.
parse(readFileSync("locked.pdf"), "secret");
```

A fatal parse fault throws an `Error` whose message carries a stable code
(`InvalidHeader`, `BadStream`, …). Image-only/scanned pages come back with
`needs_ocr: true` and no text (OCR is out of scope).

| feature | support |
|---|---|
| xref tables + streams, object streams, hybrid, incremental | ✓ |
| Flate / LZW / ASCII85 / ASCIIHex / RunLength + predictors | ✓ |
| `/ToUnicode`, WinAnsi/MacRoman/Standard + Differences (AGL), Type0/CID | ✓ |
| ruled tables, image XObjects (JPEG passthrough + raw) | ✓ |
| encryption: RC4 + AES-128/256, R2–R6, empty/user/owner password | ✓ |

Part of the [turbo-parsepdf](https://github.com/miaskiewicz/turbo-parsepdf)
workspace (also on PyPI as `turbo-parsepdf` and as `turbo-parsepdf-wasm`).
MIT.
