// Competitive perf harness: turbo-parsepdf (N-API) vs pdf-parse (pdf.js) on a
// fixture corpus. Build the addon first:
//   cargo build -p turbo-parsepdf-napi --release
//   node ../../crates/turbo-parsepdf-napi/scripts/copy-addon.mjs
// then: npm install && npm run bench

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const corpus = join(here, "corpus");

async function adapters() {
  const a = {};
  try {
    const turbo = await import("../../crates/turbo-parsepdf-napi/index.js");
    a["turbo-parsepdf"] = (d) => turbo.parseToMarkdown(d);
  } catch {
    // addon not built; skip
  }
  try {
    const { default: pdfParse } = await import("pdf-parse");
    a["pdf-parse"] = async (d) => (await pdfParse(d)).text;
  } catch {
    // not installed; skip
  }
  return a;
}

async function best(fn, n = 10) {
  await fn();
  let ms = Infinity;
  for (let i = 0; i < n; i++) {
    const t = performance.now();
    await fn();
    ms = Math.min(ms, performance.now() - t);
  }
  return ms;
}

const libs = await adapters();
const names = Object.keys(libs);
const files = readdirSync(corpus).filter((f) => f.endsWith(".pdf")).sort();

console.log(`# turbo-parsepdf — competitive perf (best-of-N, ms)\n`);
console.log(`| file | ${names.join(" | ")} |`);
console.log(`|${"---|".repeat(names.length + 1)}`);
for (const f of files) {
  const data = readFileSync(join(corpus, f));
  const cells = [];
  for (const name of names) {
    try {
      cells.push((await best(() => libs[name](data))).toFixed(2));
    } catch (e) {
      cells.push(`ERR(${e.name})`);
    }
  }
  console.log(`| ${f} | ${cells.join(" | ")} |`);
}
