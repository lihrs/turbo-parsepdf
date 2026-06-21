//! WASM / `wasm-bindgen` binding for turbo-parsepdf.
//!
//! The surface mirrors the N-API binding but is WASM-idiomatic: `parse` returns
//! the structured document as a JS object (via `serde-wasm-bindgen`), and the
//! `parse_to_{json,html,markdown}` helpers return strings. A fatal fault rejects
//! as a string `"<Code>: <message>"`; an optional `password` unlocks encrypted
//! files. All logic lives in the covered core — this is a thin marshaling shim.

#![forbid(unsafe_code)]

use wasm_bindgen::prelude::*;

use turbo_parsepdf_core::{Document, ExtractedDoc, TurboParsePdfError};

/// Map a core fault to a JS error value carrying the stable code.
fn to_js(e: TurboParsePdfError) -> JsValue {
    JsValue::from_str(&format!("{}: {}", e.code.as_str(), e.message))
}

/// Parse + extract a PDF, applying an optional user/owner password.
fn extract(data: &[u8], password: Option<String>) -> Result<ExtractedDoc, JsValue> {
    let password = password.unwrap_or_default();
    let doc = Document::parse_with_password(data, &password).map_err(to_js)?;
    doc.extract().map_err(to_js)
}

/// Parse a PDF and return the extracted document as a JS object.
#[wasm_bindgen]
pub fn parse(data: &[u8], password: Option<String>) -> Result<JsValue, JsValue> {
    let doc = extract(data, password)?;
    serde_wasm_bindgen::to_value(&doc).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Parse a PDF and return its extracted form as pretty JSON.
#[wasm_bindgen]
pub fn parse_to_json(data: &[u8], password: Option<String>) -> Result<String, JsValue> {
    Ok(extract(data, password)?.to_json())
}

/// Parse a PDF and render it to a standalone HTML document.
#[wasm_bindgen]
pub fn parse_to_html(data: &[u8], password: Option<String>) -> Result<String, JsValue> {
    Ok(extract(data, password)?.to_html())
}

/// Parse a PDF and render it to Markdown.
#[wasm_bindgen]
pub fn parse_to_markdown(data: &[u8], password: Option<String>) -> Result<String, JsValue> {
    Ok(extract(data, password)?.to_markdown())
}
