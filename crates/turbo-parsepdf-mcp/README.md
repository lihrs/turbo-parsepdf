# turbo-parsepdf-mcp

A native, synchronous **MCP server** (stdio JSON-RPC 2.0) over the
`turbo-parsepdf` PDF extractor. No async runtime — the tools are CPU/file work.

## Build & run

```sh
cargo build -p turbo-parsepdf-mcp --release
./target/release/turbo-parsepdf-mcp     # reads JSON-RPC from stdin, replies on stdout
```

Register it with an MCP client by pointing the client at that binary as a stdio
server.

## Tools

| tool | description |
|---|---|
| `parse_pdf` | Extract text/tables/images, rendered as `text`, `markdown`, `html`, or `json`. |
| `inspect_pdf` | Version, page count + geometry, `/Info` metadata, encryption status. |
| `extract_tables` | Per-page ruled tables as row/column cell grids (JSON). |
| `extract_images` | Per-page embedded image XObjects with format + geometry. |

Every tool takes `{ "path": "<file.pdf>", "password"?: "<pw>" }`; `parse_pdf`
also takes an optional `"format"`. Encrypted files are unlocked with `password`
(user or owner).

## Example

```jsonc
// → stdin
{"jsonrpc":"2.0","id":1,"method":"tools/call",
 "params":{"name":"parse_pdf","arguments":{"path":"doc.pdf","format":"markdown"}}}
// ← stdout
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"# …markdown…"}]}}
```

All extraction logic lives in `turbo-parsepdf-core` (100% line-covered); this
crate is a thin, host-unit-tested protocol shim.
