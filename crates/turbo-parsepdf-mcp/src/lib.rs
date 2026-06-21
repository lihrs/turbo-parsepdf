//! turbo-parsepdf MCP server — the protocol surface.
//!
//! A minimal, synchronous JSON-RPC 2.0 handler exposing one tool, `parse_pdf`,
//! which reads a PDF file from disk and returns its extracted text / HTML /
//! Markdown / JSON (with an optional `password` for encrypted files). All logic
//! lives in `turbo-parsepdf-core`; this crate is a thin protocol shim and is
//! excluded from the coverage gate.

#![forbid(unsafe_code)]

use serde_json::{json, Value};

use turbo_parsepdf_core::{Document, Object};

/// Per-connection session state (none is needed yet; kept for symmetry/forward
/// compatibility with the sibling MCP servers).
#[derive(Debug, Default)]
pub struct Session;

impl Session {
    /// A fresh session.
    pub fn new() -> Self {
        Session
    }
}

/// Handle one JSON-RPC request, returning the response (or `None` for a
/// notification, which gets no reply).
pub fn handle(_session: &mut Session, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str)?;
    let outcome = dispatch(method, req.get("params"));
    id.map(|id| envelope(id, outcome))
}

/// Wrap a method outcome in the JSON-RPC response envelope.
fn envelope(id: Value, outcome: Result<Value, (i64, String)>) -> Value {
    match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    }
}

/// Route a method name to its handler.
fn dispatch(method: &str, params: Option<&Value>) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(server_info()),
        "tools/list" => Ok(tool_list()),
        "tools/call" => call_tool(params),
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

/// The `initialize` result: protocol version + server identity.
fn server_info() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "turbo-parsepdf-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// A common `path` + `password` input schema fragment.
fn path_schema(extra: Value) -> Value {
    let mut props = json!({
        "path": { "type": "string", "description": "Filesystem path to the PDF." },
        "password": { "type": "string", "description": "Password for encrypted PDFs." }
    });
    merge(props.as_object_mut(), extra);
    json!({ "type": "object", "required": ["path"], "properties": props })
}

fn merge(into: Option<&mut serde_json::Map<String, Value>>, extra: Value) {
    if let (Some(into), Value::Object(extra)) = (into, extra) {
        into.extend(extra);
    }
}

/// The `tools/list` result: the full PDF tool surface.
fn tool_list() -> Value {
    json!({ "tools": [
        { "name": "parse_pdf",
          "description": "Extract a PDF's text/tables/images, rendered as text, Markdown, HTML, or JSON.",
          "inputSchema": path_schema(json!({ "format": { "type": "string", "enum": ["text", "markdown", "html", "json"] } })) },
        { "name": "inspect_pdf",
          "description": "Report a PDF's version, page count + geometry, metadata, and encryption status.",
          "inputSchema": path_schema(json!({})) },
        { "name": "extract_tables",
          "description": "Extract ruled tables from a PDF as per-page row/column cell grids (JSON).",
          "inputSchema": path_schema(json!({})) },
        { "name": "extract_images",
          "description": "List a PDF's embedded image XObjects with format and geometry (per page).",
          "inputSchema": path_schema(json!({})) }
    ]})
}

/// Dispatch a `tools/call` request to the named tool.
fn call_tool(params: Option<&Value>) -> Result<Value, (i64, String)> {
    let params = params.ok_or((-32602, "missing params".to_string()))?;
    let name = params.get("name").and_then(Value::as_str);
    let text = run_tool(name, params.get("arguments"))?;
    text_result(text)
}

/// Run a named tool, returning its text output.
fn run_tool(name: Option<&str>, args: Option<&Value>) -> Result<String, (i64, String)> {
    match name {
        Some("parse_pdf") => parse_pdf(args),
        Some("inspect_pdf") => with_doc(args, inspect),
        Some("extract_tables") => with_doc(args, tables_json),
        Some("extract_images") => with_doc(args, images_json),
        other => Err((-32602, format!("unknown tool: {}", other.unwrap_or("")))),
    }
}

/// Wrap a tool's text output in the MCP `content` envelope.
fn text_result(text: String) -> Result<Value, (i64, String)> {
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

/// Read + parse the file named by `args`, then run `f` over the document.
fn with_doc<F>(args: Option<&Value>, f: F) -> Result<String, (i64, String)>
where
    F: FnOnce(&Document) -> Result<String, (i64, String)>,
{
    let args = args.ok_or((-32602, "missing arguments".to_string()))?;
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing path".to_string()))?;
    let password = args.get("password").and_then(Value::as_str).unwrap_or("");
    let bytes = std::fs::read(path).map_err(|e| (-32000, format!("read {path}: {e}")))?;
    let doc = Document::parse_with_password(&bytes, password).map_err(tool_error)?;
    f(&doc)
}

/// `parse_pdf`: extract + render in the requested format (`text` default).
fn parse_pdf(args: Option<&Value>) -> Result<String, (i64, String)> {
    let format = args
        .and_then(|a| a.get("format"))
        .and_then(Value::as_str)
        .unwrap_or("text")
        .to_string();
    with_doc(args, move |doc| {
        let extracted = doc.extract().map_err(tool_error)?;
        Ok(format_doc(&extracted, &format))
    })
}

/// `inspect_pdf`: version, page geometry, metadata, and encryption status.
fn inspect(doc: &Document) -> Result<String, (i64, String)> {
    let pages = doc.pages().map_err(tool_error)?;
    let geometry: Vec<Value> = pages.iter().map(page_geometry).collect();
    let report = json!({
        "version": doc.version(),
        "page_count": pages.len(),
        "encrypted": doc.trailer().get("Encrypt").is_some(),
        "metadata": metadata(doc),
        "pages": geometry,
    });
    Ok(pretty(&report))
}

fn page_geometry(page: &turbo_parsepdf_core::Page) -> Value {
    let [x0, y0, x1, y1] = page.media_box;
    json!({ "width": (x1 - x0).abs(), "height": (y1 - y0).abs(), "rotate": page.rotate })
}

/// The document `/Info` metadata (title/author/subject/creator), if present.
fn metadata(doc: &Document) -> Value {
    let info = doc.trailer().get("Info").and_then(|i| doc.resolve(i).ok());
    let dict = info.as_ref().and_then(Object::as_dict);
    json!({
        "title": info_field(dict, "Title"),
        "author": info_field(dict, "Author"),
        "subject": info_field(dict, "Subject"),
        "creator": info_field(dict, "Creator"),
    })
}

fn info_field(dict: Option<&turbo_parsepdf_core::Dictionary>, key: &str) -> Value {
    let bytes = dict.and_then(|d| d.get(key)).and_then(Object::as_string);
    bytes.map_or(Value::Null, |b| {
        Value::String(String::from_utf8_lossy(b).into_owned())
    })
}

/// `extract_tables`: per-page ruled tables.
fn tables_json(doc: &Document) -> Result<String, (i64, String)> {
    let extracted = doc.extract().map_err(tool_error)?;
    let pages: Vec<Value> = extracted
        .pages
        .iter()
        .map(|p| json!({ "tables": p.tables }))
        .collect();
    Ok(pretty(&json!({ "pages": pages })))
}

/// `extract_images`: per-page embedded image XObject metadata.
fn images_json(doc: &Document) -> Result<String, (i64, String)> {
    let extracted = doc.extract().map_err(tool_error)?;
    let pages: Vec<Value> = extracted
        .pages
        .iter()
        .map(|p| json!({ "images": p.images }))
        .collect();
    Ok(pretty(&json!({ "pages": pages })))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Render an extracted document in the named format (`text` is the default).
fn format_doc(doc: &turbo_parsepdf_core::ExtractedDoc, format: &str) -> String {
    match format {
        "markdown" => doc.to_markdown(),
        "html" => doc.to_html(),
        "json" => doc.to_json(),
        _ => doc
            .pages
            .iter()
            .map(|p| p.text())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn tool_error(e: turbo_parsepdf_core::TurboParsePdfError) -> (i64, String) {
    (-32000, format!("{}: {}", e.code.as_str(), e.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        let mut s = Session::new();
        handle(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .unwrap()
    }

    #[test]
    fn initialize_and_list() {
        assert_eq!(
            call("initialize", json!({}))["result"]["serverInfo"]["name"],
            "turbo-parsepdf-mcp"
        );
        let tools = call("tools/list", json!({}));
        assert_eq!(tools["result"]["tools"][0]["name"], "parse_pdf");
    }

    #[test]
    fn unknown_method_errors() {
        let r = call("bogus", json!({}));
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn notification_has_no_reply() {
        let mut s = Session::new();
        let req = json!({ "jsonrpc": "2.0", "method": "initialize" });
        assert!(handle(&mut s, &req).is_none());
        // A request without a method is also ignored.
        assert!(handle(&mut s, &json!({ "id": 1 })).is_none());
    }

    fn fixture() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../turbo-parsepdf-core/tests/fixtures/real.pdf"
        )
    }

    fn tool(name: &str) -> Value {
        call(
            "tools/call",
            json!({ "name": name, "arguments": { "path": fixture() } }),
        )
    }

    fn tool_text(name: &str) -> String {
        tool(name)["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn lists_all_tools() {
        let names: Vec<String> = call("tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "parse_pdf",
                "inspect_pdf",
                "extract_tables",
                "extract_images"
            ]
        );
    }

    #[test]
    fn inspect_reports_version_and_geometry() {
        let report: Value = serde_json::from_str(&tool_text("inspect_pdf")).unwrap();
        assert_eq!(report["version"], "1.5");
        assert_eq!(report["page_count"], 1);
        assert_eq!(report["encrypted"], false);
        assert_eq!(report["pages"][0]["width"], 612.0);
        assert!(report["metadata"].is_object());
    }

    #[test]
    fn tables_and_images_tools_return_per_page_arrays() {
        let tables: Value = serde_json::from_str(&tool_text("extract_tables")).unwrap();
        assert!(tables["pages"][0]["tables"].is_array());
        let images: Value = serde_json::from_str(&tool_text("extract_images")).unwrap();
        assert!(images["pages"][0]["images"].is_array());
    }

    #[test]
    fn parse_pdf_reads_a_file() {
        let pdf = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../turbo-parsepdf-core/tests/fixtures/real.pdf"
        );
        let r = call(
            "tools/call",
            json!({ "name": "parse_pdf", "arguments": { "path": pdf } }),
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("turbo-parsepdf"));
        // Markdown format also works.
        let md = call(
            "tools/call",
            json!({ "name": "parse_pdf", "arguments": { "path": pdf, "format": "markdown" } }),
        );
        assert!(md["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("turbo-parsepdf"));
    }

    #[test]
    fn tool_errors() {
        // Unknown tool.
        let r = call("tools/call", json!({ "name": "nope" }));
        assert_eq!(r["error"]["code"], -32602);
        // Missing params.
        let mut s = Session::new();
        let no_params = handle(&mut s, &json!({ "id": 1, "method": "tools/call" })).unwrap();
        assert_eq!(no_params["error"]["code"], -32602);
        // Missing path.
        let no_path = call(
            "tools/call",
            json!({ "name": "parse_pdf", "arguments": {} }),
        );
        assert_eq!(no_path["error"]["code"], -32602);
        // Unreadable file.
        let bad = call(
            "tools/call",
            json!({ "name": "parse_pdf", "arguments": { "path": "/no/such.pdf" } }),
        );
        assert_eq!(bad["error"]["code"], -32000);
    }
}
