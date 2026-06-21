# turbo-parsepdf-wasm

Fast native **PDF text / table / image extraction** in the browser, via
WebAssembly (`wasm-bindgen`) over the pure-Rust turbo-parsepdf core. Output as a
JS object, or **HTML / Markdown / JSON** strings.

```sh
npm install turbo-parsepdf-wasm
```

## Benchmark

Same engine as the native Node binding — **~9.7× faster than pdf.js** on a
100-page document (5.6 ms vs 54 ms, best-of-N), text byte-identical to PyMuPDF.
Browser numbers track native within WebAssembly overhead. Full tables + harness:
the workspace [`benches/`](https://github.com/miaskiewicz/turbo-parsepdf/tree/main/benches).

```js
import init, { parse, parseToMarkdown, parseToHtml, parseToJson } from "turbo-parsepdf-wasm";

await init(); // load the .wasm

const bytes = new Uint8Array(await file.arrayBuffer());
const doc = parse(bytes);            // { version, pages: [...] }
const md = parseToMarkdown(bytes);   // string

// Encrypted PDFs: pass the user or owner password.
parse(bytes, "secret");
```

A fatal parse fault rejects with a `"<Code>: <message>"` string. Same engine and
feature set as the Node and Python bindings (cross-reference streams, all stream
filters, font/Unicode decoding, tables, images, RC4/AES decryption); scanned
pages are flagged `needs_ocr`.

Part of the [turbo-parsepdf](https://github.com/miaskiewicz/turbo-parsepdf)
workspace. MIT.
