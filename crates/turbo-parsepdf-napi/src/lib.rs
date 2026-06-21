//! turbo-parsepdf N-API binding.
//!
//! Exposes the PDF extractor of `turbo-parsepdf-core` to Node/JS. `parse` returns
//! the structured document (pages → lines/tables/images + `needsOcr`); the
//! `parseTo{Json,Html,Markdown}` helpers return ready-to-write strings. A fatal
//! fault is thrown as an `Error` carrying the stable core code; an optional
//! `password` unlocks encrypted files.
//!
//! The product surface is this thin marshaling layer — all parsing logic lives in
//! the core crate, which carries the 100% coverage gate. This crate is a cdylib
//! addon tarpaulin cannot line-instrument, so it is excluded from that gate and
//! kept deliberately minimal and mechanical.

#![deny(clippy::all)]

/// Route allocations through mimalloc (the parse path churns many short
/// String/Vec allocations). Skipped on musl — a statically-linked mimalloc
/// segfaults when the addon is dlopen'd under musl Node.
#[cfg(not(target_env = "musl"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod errors;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use serde_json::Value;

use turbo_parsepdf_core::{Document, ExtractedDoc};

/// Parse + extract a PDF buffer, applying an optional user/owner password.
fn extract(data: &[u8], password: Option<String>) -> napi::Result<ExtractedDoc> {
    let password = password.unwrap_or_default();
    let doc = Document::parse_with_password(data, &password).map_err(errors::to_napi)?;
    doc.extract().map_err(errors::to_napi)
}

/// Parse a PDF and return the extracted document as a plain JS object.
#[napi]
pub fn parse(data: Buffer, password: Option<String>) -> napi::Result<Value> {
    let doc = extract(data.as_ref(), password)?;
    serde_json::to_value(&doc).map_err(|e| napi::Error::from_reason(e.to_string()))
}

/// Parse a PDF and return its extracted form as pretty JSON.
#[napi]
pub fn parse_to_json(data: Buffer, password: Option<String>) -> napi::Result<String> {
    Ok(extract(data.as_ref(), password)?.to_json())
}

/// Parse a PDF and render it to a standalone HTML document.
#[napi]
pub fn parse_to_html(data: Buffer, password: Option<String>) -> napi::Result<String> {
    Ok(extract(data.as_ref(), password)?.to_html())
}

/// Parse a PDF and render it to Markdown.
#[napi]
pub fn parse_to_markdown(data: Buffer, password: Option<String>) -> napi::Result<String> {
    Ok(extract(data.as_ref(), password)?.to_markdown())
}
