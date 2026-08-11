//! turbo-parsepdf core engine.
//!
//! A native, dependency-light **PDF text/table/image extractor**. It reads a PDF
//! byte buffer through a streaming COS parser ([`lex`]/[`cos`]), resolves the
//! cross-reference chain ([`xref`]/[`resolver`]), decodes stream filters
//! ([`filter`]), and — in later phases — walks the page tree, interprets content
//! streams, decodes fonts to Unicode, reconstructs layout, and serializes the
//! result to HTML / Markdown / JSON.
//!
//! Image-only (scanned) pages are detected and flagged `needs_ocr` rather than
//! OCR'd. The N-API / PyO3 / wasm bindings are thin marshaling shims over this
//! crate; every branch lives here, under the 100% coverage gate.

#![forbid(unsafe_code)]

pub mod agl;
pub mod cmap;
pub mod content;
pub mod cos;
#[cfg(feature = "encrypt")]
pub mod crypt;
pub mod doc;
pub(crate) mod encode;
pub mod error;
pub mod filter;
pub mod font;
pub mod image;
pub mod layout;
pub mod lex;
pub mod object;
pub mod objstm;
pub mod pagetree;
pub mod resolver;
pub mod serialize;
pub mod tables;
pub mod text;
pub mod xref;

pub use content::{parse_content, Op, Operator};
pub use doc::Document;
pub use error::{Diagnostics, ErrorCode, Lint, LintCode, Result, TurboParsePdfError};
pub use font::{Font, FontMap, Glyph};
pub use image::{ImageFormat, ParsedImage};
pub use layout::{layout_page, Line, PageText};
pub use object::{Dictionary, ObjRef, Object, Stream};
pub use pagetree::Page;
pub use serialize::ExtractedDoc;
pub use tables::{table_run_indices, Table};
pub use text::{Matrix, TextRun};
