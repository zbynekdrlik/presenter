---
paths:
  - "tests/e2e/**"
  - "playwright.config.ts"
  - "scripts/build-ui.sh"
---

# Running Playwright specs locally on dev2

CI is the merge gate; a local run only reproduces a CI failure or checks a fix
before the push (one CI cycle is ~25 min).

- **Binary:** `startTestServer` runs a PREBUILT `target/release/presenter-server`
  (or `target/debug`, the newer wins) with the `mock-integrations,test-helpers`
  features. To test exactly what CI built: `gh run download <run> -n
  build-artifacts` and copy ALL of `presenter-server`, `import_propresenter`,
  `ingest_bibles` into `target/release/` (`refreshDevData` runs the two tools;
  without them every spec fails "Command failed (1): … refresh-dev-data.sh").
- **To test a change:** `bash scripts/build-ui.sh` (needs `trunk`, `esbuild`,
  `wasm-opt` from binaryen, `brotli`, `pigz` — the npm `esbuild` / `binaryen`
  packages in a scratch dir on `PATH` work, `brotli pigz` via apt), then
  `cargo build --release -p presenter-server -p presenter-importer --features
  presenter-server/mock-integrations,presenter-server/test-helpers`. The server
  embeds `crates/presenter-ui/dist`: edit UI sources only before the build or
  after it, never while `presenter-ui` compiles.
- **Port 8091:** the mock-integrations build binds the embedded mock Resolume on
  127.0.0.1:8091, and so does the running `presenter-dev` service. Stop it for
  the run (`sudo systemctl stop presenter-dev`), start it again afterwards.
- **Env:** the `PRESENTER_LIBRARY_ROOT` / `PRESENTER_BIBLE_*` variables of
  pipeline.yml's "Run Playwright tests" step (KJV from `data/bibles`, the others
  from `data/bibles-private`). Leave `CI` unset locally, or a failure retries.
- **Reading a CI failure:** `gh run view --log-failed` can come back empty;
  `gh api repos/zbynekdrlik/presenter/actions/jobs/<job id>/logs` always works.
- **A shard cancelled at 30 min has no blob report (#913).** Name the hung test
  from the job log's dot marks (`·` pass, `×` failed try, `F`/`T` final
  failure/timeout, `±` flaky — count every kind) against
  `npx playwright test --shard=N/3 --grep-invert "@synthetic-ndi" --list`. A
  `tap()` with no own timeout waits the whole 180 s test limit when another
  element covers its centre (CI fonts can be a few px wider than local ones).
