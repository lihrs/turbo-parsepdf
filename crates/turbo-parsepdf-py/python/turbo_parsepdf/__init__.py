"""turbo-parsepdf — fast native PDF text/table/image extraction.

Thin Python surface over the Rust core. ``parse`` returns a dict; the
``parse_to_*`` helpers return strings. Pass ``password=`` for encrypted PDFs.
"""

from ._turbo_parsepdf import (
    parse,
    parse_to_html,
    parse_to_json,
    parse_to_markdown,
)

__all__ = ["parse", "parse_to_json", "parse_to_html", "parse_to_markdown"]
