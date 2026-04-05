# Mutation Testing & Test-Integrity CI Gates

> **Date:** 2026-04-06 | **Status:** Proposed

## Problem

The CI pipeline enforces code formatting, linting, security audits, and line coverage — but none of these prove that tests actually verify behavior. A test suite can achieve 80% coverage while catching almost no real bugs if assertions are weak or missing.

**Mutation testing** fills this gap: it makes small changes to production code (e.g., `>` to `>=`, removing a `return`) and re-runs tests. If tests still pass after a mutation, the test is weak.

Additionally, the pipeline has no checks for degenerate test patterns like empty test bodies or `assert!(true)`.

## Design

### 1. Mutation Testing Job in `pipeline.yml`

A new `mutation` job runs after `test` completes, in parallel with `coverage` and `quality`.

**Updated dependency chain:**

```
branch-sync → fmt, clippy → test → mutation ─┐
                                   coverage   ├→ build → e2e → deploy-dev
                             quality ─────────┘
```

**Job configuration:**

- **Runner:** `ubuntu-latest` (GitHub-hosted)
- **Tool:** `cargo-mutants` (installed via `cargo install cargo-mutants --locked`, cached)
- **Scope:** Full workspace (`--workspace`)
- **Per-mutant timeout:** 120 seconds (`--timeout 120`) — kills mutations that cause infinite loops or extreme slowdowns
- **Job timeout:** 45 minutes (`timeout-minutes: 45`)
- **Rust cache:** Shares the `ci` cache key with clippy/test jobs
- **Tool cache:** Caches `~/.cargo/bin/cargo-mutants` binary
- **System deps:** Same as test job (protobuf-compiler, cmake, nasm)
- **Failure mode:** `cargo mutants` exits non-zero if any mutant survives — job fails, pipeline stops

**Why full workspace:** The user chose full-workspace mutation testing over diff-only. This catches weak tests across the entire codebase, not just in changed files. The per-mutant timeout prevents any single mutation from consuming excessive time.

### 2. Test-Integrity Checks in `quality-check.sh`

Three new checks added to the existing quality script, using the same `rg` + bash pattern as existing checks:

#### a) Empty test bodies

Detects `#[test]` functions that contain no `assert`, `assert_eq`, `assert_ne`, `assert_matches`, `should_panic`, or `expect` calls. A test without assertions is a no-op that provides false coverage.

Pattern: scan `#[test]` function bodies for assertion keywords. Flag any that have zero matches.

#### b) False-positive assertions

Detects trivially-true assertions that never fail:

- `assert!(true)`
- `assert_eq!(1, 1)` and similar literal-vs-literal comparisons

Pattern: `rg` for `assert!(true)` and `assert_eq!(<literal>, <literal>)` in test code.

#### c) Assertion density floor

Warn when a `#[test]` function has only one assertion for a body longer than 20 lines. Long tests with few assertions likely test setup but not behavior.

### 3. Build Job Dependency Update

The `build` job's `needs` list changes from `[test, quality]` to `[test, quality, mutation]`. This gates all downstream jobs (E2E, deploy) on mutation testing passing.

## Crates Affected

- No crate code changes — this is purely CI/tooling
- `.github/workflows/pipeline.yml` — new mutation job, updated build dependencies
- `scripts/dev/quality-check.sh` — new test-integrity checks

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Mutation testing takes too long | Per-mutant timeout (120s) + job timeout (45 min) |
| False positives (mutants that are semantically equivalent) | Review surviving mutants; add `#[mutants::skip]` for justified cases (requires `mutants` crate as dev-dependency) |
| Slows down pipeline | Runs in parallel with coverage/quality — not on critical path until build step |
| cargo-mutants not available in cache | Falls back to `cargo install` (adds ~2 min first run) |

## Success Criteria

1. `cargo mutants --workspace --timeout 120` runs in CI on every push to dev
2. Build job is gated on mutation testing passing
3. `quality-check.sh` detects empty tests, false-positive assertions, and low-assertion tests
4. No surviving mutants in the initial run (or justified `#[mutants::skip]` annotations)
5. Pipeline wall-clock time increases by no more than 10 minutes (mutation runs in parallel)
