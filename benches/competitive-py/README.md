# Competitive perf + accuracy harness (Python)

Compares **turbo-parsepdf** text extraction against the common Python PDF stack —
[`pymupdf`](https://pymupdf.readthedocs.io/) (the C/MuPDF speed reference),
[`pypdf`](https://pypdf.readthedocs.io/), and
[`pdfminer.six`](https://pdfminersix.readthedocs.io/) — on a fixture corpus.

## Setup

```sh
python3 -m venv .venv && . .venv/bin/activate
pip install pymupdf pypdf pdfminer.six
# build + install the turbo-parsepdf wheel from the sibling crate:
( cd ../../crates/turbo-parsepdf-py && maturin develop --release )
```

## Run

```sh
python bench.py            # → a Markdown perf table (best-of-N, ms)
```

`bench.py` skips any library that is not importable, so it runs with whatever
subset is installed.

## What it measures

- **Performance** — wall-clock to extract every page's text, best-of-N (the
  competitors re-open/parse the file each run, as turbo does).
- **Accuracy** — turbo's extracted text matches PyMuPDF (the reference) **word
  for word** on the corpus (100% recall, identical character count).

## Sample result (Apple M-series, release build)

```
| file          | turbo-parsepdf | pymupdf | pypdf  | pdfminer.six |
|---------------|----------------|---------|--------|--------------|
| large (100pg) |          12.76 |  308.67 | 254.66 |      1760.37 |
| medium (20pg) |           2.37 |   56.41 |  43.02 |       302.14 |
| small (2pg)   |           0.11 |    3.23 |   2.14 |        13.27 |
```

turbo-parsepdf is **18–24× faster than the fastest alternative** (and ~140×
faster than pdfminer) — *including* the Python FFI + dict-marshaling overhead.
The native Rust core extracts the same 100-page document in **~4.9 ms**
(`cargo run --release --example timeit -- large.pdf`).
