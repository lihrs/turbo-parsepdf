# turbo-parsepdf

Fast native **PDF text / table / image extraction** for Python — a pure-Rust core
(PyO3, stable-ABI wheels). Imports as `turbo_parsepdf`. Output as a `dict`, or
**HTML / Markdown / JSON** strings. **38× faster than pypdf, 62× faster than
PyMuPDF, 307× faster than pdfminer**, with text byte-identical to PyMuPDF.

```sh
pip install turbo-parsepdf
```

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
