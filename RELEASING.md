# Releasing turbo-parsepdf

Releases are **tag-driven**, mirroring the sibling turbo-xlsx. Two independent
tag prefixes drive the publish workflows in `.github/workflows/`.

| tag | publishes | secret |
|---|---|---|
| `vX.Y.Z` | **npm** `turbo-parsepdf` (5-platform N-API matrix, bundled) + **npm** `turbo-parsepdf-wasm` (`release.yml`); **crates.io** `turbo-parsepdf-core` (`release-crates.yml`) | `NPM_TOKEN`, `CARGO_REGISTRY_TOKEN` |
| `pyvX.Y.Z` | **PyPI** `turbo-parsepdf` — maturin abi3 wheels + sdist, import name `turbo_parsepdf` (`release-py.yml`) | `PYPI_TOKEN` (publish self-skips if unset) |

The parser (and the `encrypt` feature) is always on — there is **no** base/parse
variant axis.

## Before tagging

1. Run the full gate locally (CI re-runs it):

   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo clippy -p turbo-parsepdf-core --all-targets --features encrypt -- -D warnings
   cargo test --workspace --features turbo-parsepdf-core/encrypt
   cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates
   cargo tarpaulin
   cargo build -p turbo-parsepdf-napi --release && node crates/turbo-parsepdf-napi/scripts/copy-addon.mjs \
     && node --test crates/turbo-parsepdf-napi/__test__/*.test.mjs
   ```

2. Bump the same `X.Y.Z` in **all** of these (not auto-synced):

   - `Cargo.toml` → `[workspace.package] version` (all crates inherit it)
   - `crates/turbo-parsepdf-napi/package.json` → `version`
   - `crates/turbo-parsepdf-py/pyproject.toml` → `version`

   Sweep for stragglers:

   ```sh
   grep -rn "OLD.VERSION" --include="*.json" --include="*.toml" --include="*.md" . \
     | grep -vE "node_modules|/target/|Cargo.lock"
   ```

3. Update `CHANGELOG.md`.

## Tagging

```sh
git push origin main
# npm + wasm + crates.io
git tag -a vX.Y.Z -m "turbo-parsepdf vX.Y.Z" && git push origin vX.Y.Z
# PyPI
git tag -a pyvX.Y.Z -m "turbo-parsepdf vX.Y.Z" && git push origin pyvX.Y.Z
```

## Verify after

```sh
npm view turbo-parsepdf@X.Y.Z version
pip download turbo-parsepdf==X.Y.Z --no-deps -d /tmp/verify
cargo search turbo-parsepdf-core
```
