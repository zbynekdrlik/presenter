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
  Each spec file's own server binds it too, so run locally with `--workers=1`:
  with two workers the second server fails to bind and its file dies in `beforeAll`
  (180 s timeout), which looks like a test failure (#915).
- **Env:** the `PRESENTER_LIBRARY_ROOT` / `PRESENTER_BIBLE_*` variables of
  pipeline.yml's "Run Playwright tests" step (KJV from `data/bibles`, the others
  from `data/bibles-private`). Leave `CI` unset locally, or a failure retries.
  `refreshDevData` (support.ts) sets the `PRESENTER_BIBLE_*` paths ITSELF from
  `<repo root>/data/bibles-private`, whatever your shell exports: in a worktree
  (no private modules) link them in first, `ln -s <main checkout>/data/bibles-private
  data/bibles-private`, and never `git add` that link (#921).
- **From a worktree (#936):** `npx playwright` also needs `node_modules`; link the main checkout's
  (`ln -s ../../../node_modules node_modules`). Remove both links before you return: they are
  untracked, not ignored, and the worktree guard refuses a shell `rm`/`unlink` of a link that
  resolves into the main checkout, so use `python3 -c "import os; os.unlink('<worktree>/node_modules')"`.
  A cold worktree needs its own release build (~25 min on a shared dev2); start `build-ui.sh` and the
  release build in the background while the unit tests run.
- **Many translations without ingesting them:** `tests/e2e/extra-translations.ts`
  lists SNV's four extra translations and serves their `?translation=` reads from
  the KJV (layout / switching specs only; a verse lookup in them is not served).
- **Reading a CI failure:** `gh run view --log-failed` can come back empty;
  `gh api repos/zbynekdrlik/presenter/actions/jobs/<job id>/logs` always works.
- **A shard cancelled at 30 min has no blob report (#913).** Name the hung test
  from the job log's dot marks (`·` pass, `×` failed try, `F`/`T` final
  failure/timeout, `±` flaky — count every kind) against
  `npx playwright test --shard=N/3 --grep-invert "@synthetic-ndi" --list`. A
  `tap()` with no own timeout waits the whole 180 s test limit when another
  element covers its centre (CI fonts can be a few px wider than local ones).
