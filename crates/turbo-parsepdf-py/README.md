# turbo-parsepdf

Fast native **PDF text / table / image extraction** for Python — a pure-Rust core
(PyO3, stable-ABI wheels). Imports as `turbo_parsepdf`. Output as a `dict`, or
**HTML / Markdown / JSON** strings.

```sh
pip install turbo-parsepdf
```

## Benchmark vs the Python PDF stack

Wall-clock to extract every page's text, best-of-N (Apple M-series, release).
Reproduce: `python benches/competitive-py/bench.py` (after `python3 benches/gen-corpus.py`).

| document | **turbo-parsepdf** | pypdf | PyMuPDF (MuPDF, C) | pdfminer.six |
|---|---|---|---|---|
| 100 pages | **6.2 ms** | 237 ms · **38×** | 389 ms · **62×** | 1920 ms · **307×** |
| 20 pages | **1.1 ms** | 80 ms | 103 ms | 419 ms |
| 2 pages | **0.06 ms** | 2.6 ms | 4.0 ms | 18 ms |

Even *including* the Python FFI + dict-marshaling overhead, turbo is 38–307×
faster — and its text is **byte-identical to PyMuPDF** (100% word recall).

```python
import turbo_parsepdf

data = open("doc.pdf", "rb").read()

doc = turbo_parsepdf.parse(data)
# {"version": "1.7", "pages": [{"width": ..., "height": ..., "needs_ocr": False,
#   "lines": [{"text": ..., "x": ..., "y": ...}],
#   "tables": [{"rows": ..., "cols": ..., "cells": [[...]]}],
#   "images": [{"name": ..., "format": "Jpeg", "width": ..., ...}]}]}

turbo_parsepdf.parse_to_markdown(data)  # str
turbo_parsepdf.parse_to_html(data)      # str
turbo_parsepdf.parse_to_json(data)      # str

# Encrypted PDFs: pass the user or owner password.
turbo_parsepdf.parse(open("locked.pdf", "rb").read(), password="secret")
```

A fatal parse fault raises `ValueError` with a stable code
(`InvalidHeader`, `BadStream`, …). Scanned/image-only pages come back with
`needs_ocr=True` (OCR is out of scope).

Supports cross-reference streams + object streams (PDF 1.5+), all standard
stream filters + predictors, `/ToUnicode` & encoding/AGL & CID font decoding,
ruled tables, image XObject extraction, and standard-handler decryption
(RC4 + AES-128/256, R2–R6).

Part of the [turbo-parsepdf](https://github.com/miaskiewicz/turbo-parsepdf)
workspace. MIT.
