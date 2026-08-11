# Contributing to turbo-parsepdf

Thanks for helping! The project keeps a tight quality bar; the gate below runs in
CI and must pass.

## The gate

```sh
cargo fmt --all -- --check                                              # formatting
cargo clippy --workspace --all-targets -- -D warnings                   # lints (default)
cargo clippy -p turbo-parsepdf-core --all-targets --features encrypt -- -D warnings
cargo test --workspace --features turbo-parsepdf-core/encrypt           # tests
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cyclomatic complexity < 6
cargo tarpaulin                                                         # 100% line coverage
```

## Rules

- **100% line coverage** on the covered surface. New core code needs a test that
  exercises every branch — prefer inline `#[cfg(test)]` modules for private
  helpers, and `crates/turbo-parsepdf-core/tests/` for the public surface. The
  binding crates and the ported DEFLATE (`filter/inflate.rs`) and crypto
  (`crypt.rs`) modules are validated **functionally** and excluded from the line
  gate (see `tarpaulin.toml`).
- **Cyclomatic complexity < 6** per function (`cc-check`). Decompose into small
  named helpers rather than nesting branches; a `match` counts as one decision.
- **`#![forbid(unsafe_code)]`** in the core. Keep it dependency-light: filters and
  CMaps are hand-rolled; crypto lives behind the optional `encrypt` feature
  (RustCrypto).
- **All extraction logic lives in the core.** The N-API / PyO3 / wasm / MCP crates
  are thin, mechanical marshaling shims — push every branch down into the covered
  core.

## Layout

See [`README.md`](README.md#workspace) for the crate map and [`CLAUDE.md`](CLAUDE.md)
for the architecture, the benchmark runbook, and design rationale.

## Running tests

```sh
# 运行所有markdown提取测试
cargo test -p turbo-parsepdf-core --test markdown_extraction

# 运行单个测试
cargo test -p turbo-parsepdf-core --test markdown_extraction extract_markdown_from_123_pdf

# 运行所有测试并显示输出
cargo test -p turbo-parsepdf-core --test markdown_extraction -- --nocapture

# 运行所有工作区测试
cargo test --workspace


# 运行测试，自动提取并保存markdown到output目录
cargo test -p turbo-parsepdf-core --test markdown_extraction extract_markdown_from_123_pdf -- --nocapture
```

New core code needs a test that exercises every branch — see **100% line coverage**
under **Rules** above.

## Benchmarks

```sh
python3 benches/gen-corpus.py                 # generate the corpus
cargo run --release --example timeit -- benches/competitive/corpus/large.pdf
( cd benches/competitive    && npm install && node bench.mjs )  # vs pdf.js
( cd benches/competitive-py && python bench.py )                # vs pypdf/pdfminer/PyMuPDF
( cd benches/parse-native   && cargo run --release )            # vs pdf-extract / lopdf (Rust)
```
