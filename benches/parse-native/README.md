# Rust-native perf harness

Compares **turbo-parsepdf** against the Rust PDF stack on text extraction:

- [`pdf-extract`](https://crates.io/crates/pdf-extract) — the common Rust text
  extractor (built on `lopdf`); the apples-to-apples competitor.
- [`lopdf`](https://crates.io/crates/lopdf) — a low-level library; we time a raw
  structural `load_mem` + `get_pages` (it has **no** high-level text extraction),
  so this column is a *floor* (almost no work done), not a same-task rival.

A **separate workspace** so these competitor crates never enter the product
workspace or its lockfile.

## Run

```sh
python3 ../../benches/gen-corpus.py     # generate the corpus first
cargo run --release                     # → a Markdown perf table (best-of-20)
```

## Sample result (Apple M-series, release build)

```
| file   | turbo-parsepdf | pdf-extract | lopdf (parse only) |
|--------|----------------|-------------|--------------------|
| small  |           0.05 |        3.97 |               0.03 |
| medium |           0.87 |       51.56 |               0.10 |
| large  |           4.67 |      274.67 |               0.23 |
```

turbo-parsepdf is **~59× faster than `pdf-extract`** on the 100-page document
(4.67 ms vs 274.67 ms) while producing the same text. `lopdf`'s bare load is
faster only because it does no extraction at all — it parses object structure and
stops.
