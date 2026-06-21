//! turbo-parsepdf PyO3 binding.
//!
//! Exposes the PDF extractor of `turbo-parsepdf-core` to Python. `parse` returns
//! a native dict (pages → lines/tables/images + `needs_ocr`); the
//! `parse_to_{json,html,markdown}` helpers return strings. A fatal fault raises
//! `ValueError` carrying the stable core code; an optional `password` unlocks
//! encrypted files. This crate is a thin marshaling shim — all logic lives in the
//! covered core, so it is excluded from the coverage gate.

#![deny(clippy::all)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pythonize::pythonize;

use turbo_parsepdf_core::{Document, ExtractedDoc, TurboParsePdfError};

/// Map a core fault to a Python `ValueError` carrying the stable code.
fn to_pyerr(e: TurboParsePdfError) -> PyErr {
    PyValueError::new_err(format!("{}: {}", e.code.as_str(), e.message))
}

/// Parse + extract a PDF, applying an optional user/owner password.
fn extract(data: &[u8], password: Option<&str>) -> PyResult<ExtractedDoc> {
    let doc = Document::parse_with_password(data, password.unwrap_or("")).map_err(to_pyerr)?;
    doc.extract().map_err(to_pyerr)
}

/// Parse a PDF and return the extracted document as a native Python dict.
#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn parse(py: Python<'_>, data: &[u8], password: Option<&str>) -> PyResult<PyObject> {
    let doc = extract(data, password)?;
    let value = serde_json::to_value(&doc).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(pythonize(py, &value)
        .map_err(|e| PyValueError::new_err(e.to_string()))?
        .into())
}

/// Parse a PDF and return its extracted form as pretty JSON.
#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn parse_to_json(data: &[u8], password: Option<&str>) -> PyResult<String> {
    Ok(extract(data, password)?.to_json())
}

/// Parse a PDF and render it to a standalone HTML document.
#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn parse_to_html(data: &[u8], password: Option<&str>) -> PyResult<String> {
    Ok(extract(data, password)?.to_html())
}

/// Parse a PDF and render it to Markdown.
#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn parse_to_markdown(data: &[u8], password: Option<&str>) -> PyResult<String> {
    Ok(extract(data, password)?.to_markdown())
}

/// The native extension module (`turbo_parsepdf._turbo_parsepdf`).
#[pymodule]
fn _turbo_parsepdf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_parsers(m)?;
    register_renderers(m)
}

/// Register the structured-output entry points.
fn register_parsers(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(parse_to_json, m)?)
}

/// Register the string-rendering entry points.
fn register_renderers(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_to_html, m)?)?;
    m.add_function(wrap_pyfunction!(parse_to_markdown, m)?)
}
