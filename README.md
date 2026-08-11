# turbo-parsepdf

**Fast, native PDF text / table / image extraction** — a pure-Rust core shipped to
Node (N-API), Python (PyO3), the browser (WebAssembly), and the CLI/MCP, with
**HTML / Markdown / JSON** output.

> Extracts the same text as PyMuPDF **byte-for-byte**, at **~10× the speed of
> pdf.js and 38–307× the speed of the Python PDF stack.**

```
PDF bytes ─► COS parser ─► xref / objects ─► stream filters ─► page tree
        ─► content interpreter ─► font/Unicode decode ─► layout (lines/words)
        ─► tables + images ─► HTML | Markdown | JSON
```

---

## Why

Reading PDFs in Node or Python is slow: pdf.js is JavaScript, `pdfminer`/`pypdf`
are pure Python, and even MuPDF's bindings carry per-call overhead. turbo-parsepdf
is a single, allocation-frugal Rust core with thin bindings — so the same
extraction is an order of magnitude faster everywhere, while doing **more**
(layout reconstruction, ruled tables, embedded images, encryption) than a
text-only reader.

## Benchmarks

Best-of-N wall-clock to extract every page's text (Apple M-series, release
builds). Reproduce with `python3 benches/gen-corpus.py` then the harnesses in
`benches/` (see [`benches/competitive-py/README.md`](benches/competitive-py/README.md)).

| document | **turbo-parsepdf** | pdf.js (`pdf-parse`) | pypdf | PyMuPDF (MuPDF, C) | pdfminer.six |
|---|---|---|---|---|---|
| 100 pages | **5.6 ms** | 54 ms · **9.7×** | 237 ms · **38×** | 389 ms · **62×** | 1920 ms · **307×** |
| 20 pages | **0.92 ms** | 8.3 ms · **9.0×** | 80 ms | 103 ms | 419 ms |
| 2 pages | **0.05 ms** | — | 2.6 ms | 4.0 ms | 18 ms |

Against the Rust stack ([`benches/parse-native`](benches/parse-native/README.md)),
turbo-parsepdf is **~59× faster than `pdf-extract`** (the common Rust text
extractor) on the same 100-page document — 4.7 ms vs 275 ms.

**Accuracy:** on the corpus, turbo-parsepdf's extracted text is **identical to
PyMuPDF** (same character count, 100% word recall) — it is fast *and* correct,
not fast by cutting corners.

## Install & use

### Node (npm)

```sh
npm install turbo-parsepdf
```

```js
import { parse, parseToMarkdown, parseToHtml, parseToJson } from "turbo-parsepdf";
import { readFileSync } from "node:fs";

const pdf = readFileSync("doc.pdf");
const doc = parse(pdf);            // { version, pages: [{ width, height, lines, tables, images, needs_ocr }] }
const md = parseToMarkdown(pdf);   // string
const html = parseToHtml(pdf);     // string
const json = parseToJson(pdf);     // string
const locked = parse(pdf2, "secret"); // encrypted PDFs: pass the user/owner password
```

### Python (PyPI)

```sh
pip install turbo-parsepdf   # imports as `turbo_parsepdf`
```

```python
import turbo_parsepdf
data = open("doc.pdf", "rb").read()
doc = turbo_parsepdf.parse(data)                 # dict
md = turbo_parsepdf.parse_to_markdown(data)      # str
html = turbo_parsepdf.parse_to_html(data)
js = turbo_parsepdf.parse_to_json(data)
locked = turbo_parsepdf.parse(data2, password="secret")
```

### Rust (crates.io)

```toml
[dependencies]
turbo-parsepdf-core = "0.1"
```

```rust
use turbo_parsepdf_core::Document;

let bytes = std::fs::read("doc.pdf")?;
let doc = Document::parse(&bytes)?;          // or parse_with_password(&bytes, "secret")
let extracted = doc.extract()?;              // ExtractedDoc { version, pages }
println!("{}", extracted.to_markdown());
```

### Browser (WebAssembly)

```sh
npm install turbo-parsepdf-wasm
```

```js
import init, { parse, parseToMarkdown } from "turbo-parsepdf-wasm";
await init();
const doc = parse(new Uint8Array(buffer));
```

**Build from source:**

```sh
rustup target add wasm32-unknown-unknown
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
wasm-pack build crates/turbo-parsepdf-wasm --release --target web --out-dir pkg
```

The WebAssembly build outputs to `crates/turbo-parsepdf-wasm/pkg/` ready for npm
or direct bundler integration.

### CLI

```sh
cargo run --release --example extract -- doc.pdf markdown   # text | md | html | json
```

### MCP server (use it in Claude)

`turbo-parsepdf-mcp` is a stdio JSON-RPC MCP server exposing four tools:
`parse_pdf` (text/markdown/html/json), `inspect_pdf` (version, page geometry,
metadata, encryption), `extract_tables`, and `extract_images`. Each takes
`{ "path": "<file.pdf>", "password"?: "<pw>" }`.

Build the binary:

```sh
cargo build -p turbo-parsepdf-mcp --release
# → target/release/turbo-parsepdf-mcp
```

**Claude Code** (CLI):

```sh
claude mcp add turbo-parsepdf -- /absolute/path/to/target/release/turbo-parsepdf-mcp
```

**Claude Desktop** — add to `claude_desktop_config.json`
(macOS: `~/Library/Application Support/Claude/`, Windows: `%APPDATA%\Claude\`):

```jsonc
{
  "mcpServers": {
    "turbo-parsepdf": {
      "command": "/absolute/path/to/target/release/turbo-parsepdf-mcp"
    }
  }
}
```

Restart Claude, then ask it to e.g. *"use turbo-parsepdf to extract the tables
from ~/invoice.pdf"*. Full protocol details:
[`crates/turbo-parsepdf-mcp`](crates/turbo-parsepdf-mcp/README.md).

## Output

| format | shape |
|---|---|
| **JSON** | `{ version, pages: [{ width, height, needs_ocr, lines: [{text,x,y}], tables: [{rows,cols,cells}], images: [{name,format,width,height,...}] }] }` (see [`schema/`](schema/)) |
| **Markdown** | reading text per page, GitHub pipe-tables, `---` page breaks |
| **HTML** | one `<section class="page">` per page, `<p>` lines, `<table>`s, `data-needs-ocr` for scanned pages |

`.docx` output is a planned future format.

## Features

| area | support |
|---|---|
| **Parsing** | classic xref tables **and** cross-reference streams (PDF 1.5+), object streams, hybrid `/XRefStm`, incremental updates (`/Prev`) |
| **Filters** | `FlateDecode`, `LZWDecode`, `ASCII85Decode`, `ASCIIHexDecode`, `RunLengthDecode` + PNG/TIFF predictors (all hand-rolled, dependency-free) |
| **Text** | content-stream interpreter (CTM + full text state), `Tj`/`TJ`/`'`/`"`, positioned runs, line/word reconstruction with reading order |
| **Fonts** | `/ToUnicode` CMaps, simple-font encodings (WinAnsi/MacRoman/Standard) + `/Differences` via the Adobe Glyph List, Type0/CID (Identity), glyph widths |
| **Tables** | ruled-table detection from `re`/`m`/`l` graphics + positioned text |
| **Images** | image XObject extraction (JPEG/JP2 passthrough, raw raster) with geometry |
| **Encryption** | standard security handler — RC4 + AES-128/256, revisions R2–R6, empty / user / owner password |
| **Scanned pages** | detected and flagged `needs_ocr` (OCR itself is out of scope) |

## Engineering

The core crate is held to strict gates (CI re-runs all of them):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --features turbo-parsepdf-core/encrypt
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cyclomatic complexity < 6
cargo tarpaulin                                                          # 100% line coverage
```

**Running tests locally:**

```sh
cargo test -p turbo-parsepdf-core                  # core tests
cargo test --test markdown_extraction              # markdown extraction tests
cargo test --workspace                              # all tests
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md#running-tests) for full test commands.

- **100% line coverage** on the core (the binding shims and the ported DEFLATE /
  crypto modules are validated functionally and excluded, as in the sibling
  turbo-xlsx).
- **Cyclomatic complexity < 6** on every function.
- `#![forbid(unsafe_code)]` in the core; dependency-light (hand-rolled filters,
  RustCrypto only behind the optional `encrypt` feature).

See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`CLAUDE.md`](CLAUDE.md).

## Workspace

| crate | what |
|---|---|
| `crates/turbo-parsepdf-core` | the pure-Rust engine (crates.io: `turbo-parsepdf-core`) |
| `crates/turbo-parsepdf-napi` | N-API binding (npm: `turbo-parsepdf`) |
| `crates/turbo-parsepdf-py` | PyO3/maturin binding (PyPI: `turbo-parsepdf`) |
| `crates/turbo-parsepdf-wasm` | wasm-bindgen browser build (npm: `turbo-parsepdf-wasm`) |
| `crates/turbo-parsepdf-mcp` | MCP server (stdio JSON-RPC) |

## License

MIT — see [`LICENSE`](LICENSE).
