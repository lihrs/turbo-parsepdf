// Hand-maintained loader for the turbo-parsepdf N-API addon. Loads the platform
// `.node` binary and re-exports the `#[napi]` functions. A `napi build` writes
// platform packages; the `build:cargo` path copies a local cdylib next to this
// file as `turbo-parsepdf-napi.node`.

"use strict";

const { existsSync } = require("node:fs");
const { join } = require("node:path");
const { platform, arch } = process;

function candidates() {
  const local = join(__dirname, "turbo-parsepdf-napi.node");
  const triple = `turbo-parsepdf.${platform}-${arch}.node`;
  return [local, join(__dirname, triple), `turbo-parsepdf-${platform}-${arch}`];
}

function load() {
  for (const candidate of candidates()) {
    try {
      if (candidate.startsWith("turbo-parsepdf-") || existsSync(candidate)) {
        return require(candidate);
      }
    } catch (_err) {
      // Try the next candidate.
    }
  }
  throw new Error(
    "turbo-parsepdf: native addon not found. Run `npm run build` (napi) or `npm run build:cargo`.",
  );
}

const native = load();

module.exports.parse = native.parse;
module.exports.parseToJson = native.parseToJson;
module.exports.parseToHtml = native.parseToHtml;
module.exports.parseToMarkdown = native.parseToMarkdown;
