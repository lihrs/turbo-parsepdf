// Copy the cargo-built cdylib next to index.js as `turbo-parsepdf-napi.node`,
// for the plain `cargo build` path (when @napi-rs/cli is not installed).
// `napi build` does this itself, so this script is only used by
// `npm run build:cargo`.

import { copyFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const repoRoot = join(root, "..", "..");

const name =
  process.platform === "darwin"
    ? "libturbo_parsepdf_napi.dylib"
    : process.platform === "win32"
      ? "turbo_parsepdf_napi.dll"
      : "libturbo_parsepdf_napi.so";

const src = join(repoRoot, "target", "release", name);
if (!existsSync(src)) {
  console.error(
    `copy-addon: ${src} not found — run \`cargo build -p turbo-parsepdf-napi --release\` first`,
  );
  process.exit(1);
}
const dest = join(root, "turbo-parsepdf-napi.node");
copyFileSync(src, dest);
console.log(`copy-addon: ${src} -> ${dest}`);
