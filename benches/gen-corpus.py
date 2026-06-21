#!/usr/bin/env python3
"""Generate the benchmark corpus (deterministic, no deps).

Writes small/medium/large multi-page text PDFs (FlateDecode content, WinAnsi
Type1 font) into each harness's `corpus/` directory. Run from the repo root:

    python3 benches/gen-corpus.py
"""

import os
import zlib


def build(npages, lines_per_page):
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>"]
    kids = " ".join(f"{3 + i} 0 R" for i in range(npages))
    font_obj = 3 + 2 * npages
    objs.append(
        (
            "<< /Type /Pages /Kids [" + kids + f"] /Count {npages} "
            f"/MediaBox [0 0 612 792] /Resources << /Font << /F1 {font_obj} 0 R >> >> >>"
        ).encode()
    )
    contents = []
    for p in range(npages):
        parts = ["BT /F1 11 Tf"]
        y = 760
        for line in range(lines_per_page):
            parts.append(
                f"1 0 0 1 50 {y} Tm (Page {p + 1} line {line + 1}: "
                "the quick brown fox jumps over the lazy dog 0123456789) Tj"
            )
            y -= 14
        parts.append("ET")
        comp = zlib.compress("\n".join(parts).encode(), 6)
        objs.append(f"<< /Type /Page /Parent 2 0 R /Contents {3 + npages + p} 0 R >>".encode())
        contents.append(b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(comp) + comp + b"\nendstream")
    objs.extend(contents)
    objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    pdf = b"%PDF-1.5\n"
    offsets = []
    for i, obj in enumerate(objs):
        offsets.append(len(pdf))
        pdf += b"%d 0 obj\n" % (i + 1) + obj + b"\nendobj\n"
    xref = len(pdf)
    pdf += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for off in offsets:
        pdf += b"%010d 00000 n \n" % off
    pdf += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF" % (len(objs) + 1, xref)
    return pdf


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    sizes = {"small": (2, 20), "medium": (20, 45), "large": (100, 50)}
    for harness in ("competitive", "competitive-py"):
        out = os.path.join(here, harness, "corpus")
        os.makedirs(out, exist_ok=True)
        for name, (pages, lines) in sizes.items():
            path = os.path.join(out, f"{name}.pdf")
            with open(path, "wb") as fh:
                fh.write(build(pages, lines))
            print(f"wrote {path} ({os.path.getsize(path)} bytes)")


if __name__ == "__main__":
    main()
