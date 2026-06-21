//! turbo-parsepdf MCP server — the stdio pump.
//!
//! Reads newline-delimited JSON-RPC requests from stdin, dispatches each through
//! [`turbo_parsepdf_mcp::handle`], and writes one-line responses to stdout. Blank
//! and unparseable lines are skipped; notifications (no `id`) get no reply.

#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

use turbo_parsepdf_mcp::{handle, Session};

fn main() {
    let mut session = Session::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        if pump(&mut session, line, &mut stdout).is_none() {
            break;
        }
    }
}

/// Handle one input line. Returns `None` only when stdin is closed/errored.
fn pump(session: &mut Session, line: std::io::Result<String>, out: &mut impl Write) -> Option<()> {
    let line = line.ok()?;
    let trimmed = line.trim();
    if !trimmed.is_empty() {
        respond(session, trimmed, out);
    }
    Some(())
}

/// Parse, dispatch, and write the response line (if any).
fn respond(session: &mut Session, line: &str, out: &mut impl Write) {
    if let Ok(req) = serde_json::from_str::<serde_json::Value>(line) {
        if let Some(reply) = handle(session, &req) {
            let _ = writeln!(out, "{reply}");
        }
    }
}
