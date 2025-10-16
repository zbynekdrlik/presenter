## Implementation Tasks
- [x] Audit `crates/presenter-core/src/bible.rs` to catalogue exported types, functions, constants, and tests that must remain available.
- [x] Create `crates/presenter-core/src/bible/` directory with `mod.rs` facade plus `reference.rs`, `canonical.rs`, `translation.rs`, `search.rs` modules; move code into the appropriate files while preserving behaviour.
- [x] Relocate existing unit tests into focused modules under `crates/presenter-core/src/bible/tests/`, updating fixtures/helpers as needed.
- [x] Update workspace imports (including other crates) to use the new module paths or rely on re-exports; run `cargo fmt`.
- [x] Execute `cargo test -p presenter-core` and relevant integration suites; ensure Playwright/E2E suites still pass as part of `sudo -E ./scripts/dev/verify-and-refresh.sh`.
- [x] Run `scripts/dev/quality-check.sh --strict --against origin/main` to confirm the Bible module now complies with Issue #41 thresholds and no new warnings appear.
