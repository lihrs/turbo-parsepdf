# turbo-parsepdf — contributor & agent notes

Native, dependency-light **PDF text/table/image extractor**: a pure-Rust core
shipped to npm (N-API), PyPI (PyO3), the browser (wasm), and an MCP server. The
PDF peer of the sibling `turbo-xlsx` / `turbo-html2pdf` — same Rust-core + thin-
binding blueprint, same `turbo-*` packaging and gates.

## Layout

| Path | What |
| --- | --- |
| `crates/turbo-parsepdf-core` | Rust core: COS parser → resolver → filters → content/font/layout → tables/images → HTML/MD/JSON. **100% coverage gate.** |
| `crates/turbo-parsepdf-napi` | N-API binding, published as **`turbo-parsepdf`** on npm. Excluded from coverage. |
| `crates/turbo-parsepdf-py` | PyO3/maturin binding, **`turbo-parsepdf`** on PyPI (abi3; import name `turbo_parsepdf`). Excluded. |
| `crates/turbo-parsepdf-wasm` | wasm-bindgen browser build (`turbo-parsepdf-wasm`). Excluded. |
| `crates/turbo-parsepdf-mcp` | MCP server (stdio JSON-RPC): `parse_pdf` / `inspect_pdf` / `extract_tables` / `extract_images`. Excluded from cov; host-unit-tested. |
| `tools/cc-check` | cyclomatic-complexity gate (cc < 6), own workspace. |
| `benches/` | competitive perf harnesses + corpus generator. |
| `schema/` | JSON Schema for the `ExtractedDoc` output. |

All four bindings are thin marshaling shims over the same core; every branch
lives in the covered core. Each is a cdylib tarpaulin cannot line-instrument, so
all are excluded (`tarpaulin.toml`) and marked `test = false` in `[lib]` (they
can't host-link Python/Node symbols).

### Core module map

`lex`/`cos` (COS tokenizer + value parser), `object` (typed model + `Dictionary`),
`xref` (classic tables + xref streams + hybrid + `/Prev`), `objstm` (object
streams), `resolver` (lazy cached indirect objects, transparent decryption),
`filter/*` (Flate/LZW/ASCII85/ASCIIHex/RunLength + predictors), `pagetree`
(inherited attrs + content assembly), `content` (op tokenizer; inline `Operator`),
`text` (content interpreter + `Matrix` + positioned runs), `cmap`/`agl`/`font`
(Unicode + widths), `layout` (lines/reading-order/`needs_ocr`), `tables`, `image`,
`serialize` (`ExtractedDoc` → HTML/MD/JSON), `crypt` (behind `encrypt` feature),
`doc` (public `Document` entry point).

## Pre-commit / pre-tag gate (all must pass; CI re-runs them)

```
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="-D warnings" cargo clippy -p turbo-parsepdf-core --all-targets --features encrypt -- -D warnings
cargo test --workspace --features turbo-parsepdf-core/encrypt
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc < 6
cargo tarpaulin                                                         # 100% gate (features=encrypt in tarpaulin.toml)
cargo build -p turbo-parsepdf-napi --release && node crates/turbo-parsepdf-napi/scripts/copy-addon.mjs \
  && node --test crates/turbo-parsepdf-napi/__test__/*.test.mjs        # napi conformance
```

- **Coverage is 100%** (`tarpaulin.toml` `fail-under = 100`, `Llvm` engine pinned
  so host and Linux agree). `tarpaulin.toml` sets `features = "encrypt"` so the
  decrypt glue in the covered core (resolver/doc) is compiled and exercised by
  `tests/encrypt.rs`. New core code needs a test hitting every branch.
- **Excluded from the line gate** (validated functionally): the binding crates,
  the ported DEFLATE decoder `filter/inflate.rs`, and `crypt.rs`. Note that
  tarpaulin counts `#[cfg]`-disabled lines, so avoid `cfg(not(feature))` stub
  *bodies* in covered files — gate the call site instead (see `resolver.rs`).
- **cc-check scans all `crates/` source**, including binding + test code, so keep
  every function (and test helper) under cc 6.

## How to publish (tag-driven)

Two independent tag prefixes (full runbook + version-bump locations in
[`RELEASING.md`](RELEASING.md)):

| Tag | Publishes | Secret |
| --- | --- | --- |
| `vX.Y.Z` | npm `turbo-parsepdf` (5-platform napi) + `turbo-parsepdf-wasm`; crates.io `turbo-parsepdf-core` | `NPM_TOKEN`, `CARGO_REGISTRY_TOKEN` |
| `pyvX.Y.Z` | PyPI `turbo-parsepdf` (maturin abi3 wheels + sdist) | `PYPI_TOKEN` (self-skips if unset) |

There is **no base/parse variant axis** (unlike turbo-xlsx): the parser and the
`encrypt` feature are always on. Bump the same `X.Y.Z` in `Cargo.toml`
`[workspace.package]`, `crates/turbo-parsepdf-napi/package.json`, and
`crates/turbo-parsepdf-py/pyproject.toml` before tagging.

## How to benchmark

```
python3 benches/gen-corpus.py                                   # deterministic corpus (small/medium/large)
cargo run --release --example timeit -- benches/competitive/corpus/large.pdf   # native, in-process, best-of-N
( cd benches/competitive    && npm install && node bench.mjs )  # turbo (napi) vs pdf.js (pdf-parse)
( cd benches/competitive-py && python bench.py )                # turbo (wheel) vs pypdf / pdfminer / PyMuPDF
```

**Interpreting numbers.** Time `parse + extract` best-of-N; competitors re-open
the file each run, as turbo does. The corpus is dense text — turbo's relative
worst case (pdf.js does less per page there); real-world mixed PDFs widen the
gap. **Beware thermal throttling**: after many release builds the laptop throttles
and *all* parsers slow down — measure turbo and the competitors back-to-back and
report the **ratio**, not absolutes. Headline (cool machine, 100-page doc):
turbo **5.6 ms** vs pdf.js 54 (9.7×), pypdf 237 (38×), PyMuPDF 389 (62×),
pdfminer 1920 (307×); text byte-identical to PyMuPDF.

## Design considerations

- **Dependency-light core.** Filters, CMap parsing, the base64-free byte work,
  and the glyph tables are hand-rolled; the only deps are serde/serde_json/
  thiserror, plus the RustCrypto stack **behind the optional `encrypt` feature**.
  A default build links no crypto.
- **Allocation-bound hot path.** Profiling showed the pipeline is dominated by
  small allocations (per-glyph strings, content operands), not arithmetic — the
  napi build (mimalloc) beats the native example (system malloc) on the same
  work. The biggest single win was eliminating the per-glyph `String`/`Glyph`
  allocation in text show (`Font::show_into` / `text::decode_run`): **2.7×**.
  Other wins: a hand-rolled number parser (`cos::fast_numeric`, bit-exact for
  PDF's short decimals, std fallback for the rest) and an inline 4-byte
  `Operator` (no per-op `String`).
- **A fused single-pass engine was tried and reverted.** Streaming ops through one
  reused operand buffer (no `Vec<Op>`) *regressed* the system-malloc native build
  and was neutral under mimalloc — the per-op work, not the AST allocation,
  dominates, and the closure hurt inlining. The codebase keeps the simpler
  tokenize-then-interpret flow with a pre-sized operand buffer.
- **Font/Unicode is the accuracy battleground.** Order of preference: a font's
  `/ToUnicode` CMap (best), then simple-font base encoding + `/Differences` via
  the Adobe Glyph List, then CID. A symbolic font with no `/ToUnicode` may be
  unrecoverable — we emit what we can and never fabricate text.
- **Scanned pages are flagged, not OCR'd.** A page with no text operators and at
  least one image is marked `needs_ocr` (`data-needs-ocr="true"` in HTML, an
  `_[scanned page — …OCR]_` line in Markdown); OCR is out of scope. `.docx` output
  is a future format.
- **`Object` is an owned closed set.** Strings/arrays/dicts own their data so the
  resolver can hand back values without lifetime ties; the cost is some cloning on
  `get`, accepted for API simplicity. `Dictionary` preserves key order for
  reproducible output.

## Status

All phases complete: core (100% cov, cc < 6), four bindings (napi/py/wasm/mcp),
CI + release workflows, benchmarks, and docs. ~194 core tests + binding
conformance suites; `#![forbid(unsafe_code)]` in the core.
