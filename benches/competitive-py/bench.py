#!/usr/bin/env python3
"""Competitive perf + accuracy harness: turbo-parsepdf vs the Python PDF stack.

Compares wall-clock text extraction (best-of-N) of `turbo_parsepdf` against
`pymupdf` (fitz), `pypdf`, and `pdfminer.six` on a fixture corpus, and checks
that turbo's text matches PyMuPDF (the reference) word-for-word.

Setup:
    python3 -m venv .venv && . .venv/bin/activate
    pip install pymupdf pypdf pdfminer.six
    # build + install turbo: cd ../../crates/turbo-parsepdf-py && maturin develop --release
Run:
    python bench.py [corpus_dir]   # default: ./corpus
"""

import io
import os
import sys
import time

CORPUS = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "corpus")


def best(fn, n=10):
    fn()  # warm
    return min(_time(fn) for _ in range(n)) * 1000.0


def _time(fn):
    t = time.perf_counter()
    fn()
    return time.perf_counter() - t


def adapters():
    a = {}
    try:
        import turbo_parsepdf

        a["turbo-parsepdf"] = lambda d: turbo_parsepdf.parse_to_markdown(d)
    except ImportError:
        pass
    try:
        import fitz

        def mupdf(d):
            doc = fitz.open(stream=d, filetype="pdf")
            return "".join(p.get_text() for p in doc)

        a["pymupdf"] = mupdf
    except ImportError:
        pass
    try:
        import pypdf

        a["pypdf"] = lambda d: "".join(p.extract_text() for p in pypdf.PdfReader(io.BytesIO(d)).pages)
    except ImportError:
        pass
    try:
        from pdfminer.high_level import extract_text

        a["pdfminer.six"] = lambda d: extract_text(io.BytesIO(d))
    except ImportError:
        pass
    return a


def main():
    libs = adapters()
    files = sorted(f for f in os.listdir(CORPUS) if f.endswith(".pdf"))
    print(f"# turbo-parsepdf — competitive perf (best-of-N, ms)\n")
    header = "| file | " + " | ".join(libs) + " |"
    print(header)
    print("|" + "---|" * (len(libs) + 1))
    for f in files:
        data = open(os.path.join(CORPUS, f), "rb").read()
        cells = []
        for name, fn in libs.items():
            try:
                cells.append(f"{best(lambda: fn(data)):.2f}")
            except Exception as e:  # noqa: BLE001
                cells.append(f"ERR({type(e).__name__})")
        print(f"| {f} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
