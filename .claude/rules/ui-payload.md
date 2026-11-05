---
paths:
  - "scripts/build-ui.sh"
  - "crates/presenter-server/src/router/wasm_ui.rs"
  - "crates/presenter-ui/Cargo.toml"
  - "crates/presenter-ui/.cargo/config.toml"
  - "crates/presenter-ui/index.html"
  - "scripts/ci/check_ui_size_budget.py"
  - "scripts/ci/ui_size_budget.json"
  - ".github/workflows/ui-perf.yml"
  - "tests/e2e/phone-payload.spec.ts"
  - "tests/e2e/perf-phone.spec.ts"
---

# What a phone downloads (#850 B1c)

- **`scripts/build-ui.sh` shrinks and precompresses.** `wasm-opt -Oz --mvp-features
  --enable-mutable-globals` (binaryen from apt): its feature validation is the Safari 12 gate,
  so never add a feature flag to it, and never move it into Trunk (`data-wasm-opt` stays `0`).
  Then `brotli -q 11` + `gzip -9 -n` siblings for every `.wasm` / `.js` / `.css`. `binaryen` and
  `brotli` are in the "Install system dependencies" step of the build job in pipeline, deploy and
  release (no second `apt-get update`, a stalled mirror has hung that step several times).
- **The size profile is in `crates/presenter-ui/.cargo/config.toml`, not the crate's `Cargo.toml`.**
  A config `[profile]` beats a manifest `[profile]`, and the root `.cargo/config.toml` merges into
  this build by directory ancestry, so `opt-level = "z"` in the manifest is silently ignored
  (probed: `opt-level=3`, `lto=thin`). The nearer config wins.
- **`router/wasm_ui.rs` only picks**: `.br` → `.gz` → the file by Accept-Encoding (an explicit
  entry beats `*`, `q=0` refuses), `Content-Encoding` + `Vary: accept-encoding`. Never a runtime
  `CompressionLayer` (6 MB per cold load on the N100).
- **Chrome offers brotli only on HTTPS and on loopback.** A phone on the LAN (plain http) gets the
  gzip sibling, the public host gets brotli, the E2E server (127.0.0.1) gets brotli. A LAN answer
  of `gzip` is correct, not a bug; `curl -H 'Accept-Encoding: br'` proves the `.br` branch.
- **Size budget:** `check_ui_size_budget.py` in the build job counts raw, brotli and gzip;
  `ui_size_budget.json` is the first measurement + 5 %. Raising it is a reviewed edit with the
  measured numbers in the commit body.
- **Timing:** `@perf-phone` never runs per PR: `chromium` excludes it and its own `perf-phone`
  project exists only with `PERF_BASE_URL` set. `--retries=0`, a retry could pass a missed budget.
  `ui-perf.yml` measures a running instance on demand: after a payload or console release, SNV
  LAN at 3500 ms and the public host at 3000 ms (spec S8).
