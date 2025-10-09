# Issue #41 — Recurring Quality & Architecture Review (2025 Baseline)

Purpose
- Keep architecture, code quality, and operational workflows aligned with our 2025 standards, continuously — not a one‑off.
- Use this issue before every merge to main, on bigger refactors, and on a regular cadence (e.g., weekly) to spot regressions early.

How To Use
- For any branch that meaningfully changes server routes, UI surfaces, state, or runtime scripts:
  1) Run `scripts/dev/quality-check.sh --strict` and fix all failures.
  2) Run `sudo -E ./scripts/dev/verify-and-refresh.sh` and confirm demo + gateway validations pass.
  3) Paste the “Checklist” block below into your PR description and check off each item.
  4) If the review finds architectural drift, add/update an ADR under `docs/adr/` and link it in the PR.

Scope
- Monolithic app only; feature modules inside `crates/presenter-server` (no microservices).
- Operator UX, stage/gateway behavior, Bible/resolume/OSC/AbleSet integrations, data model, and test harness.

Automated Pre‑Flight (run locally)
- `scripts/dev/quality-check.sh --strict`
  - Router layout sanity (feature modules present; no legacy route impls in `router.rs`).
  - UI layout checks (expected pages exist; shared components/tokens under `ui/`).
  - File size limits (warn >800 lines; fail >1000 lines) and function length limits (fail >60 lines).
  - No direct `playwright show-report` calls; enforce `scripts/dev/show-playwright-report.sh`.
  - No focused/skipped Playwright tests.
  - Optional: emits warnings for `cargo check` warnings; use Clippy locally to tighten further when feasible.
  - Format & Lint: `cargo fmt --check` and `cargo clippy` (presenter-server at minimum) must pass with `-D warnings`.
  - Dependency/security: if installed, `cargo deny check` and `cargo audit` must pass.
  - Hygiene: no `unwrap`/`expect`/`panic!` in production sources (allowed in tests/examples/benches).
  - Async: no `std::thread::sleep` or `block_on(` in server async paths.
  - Toolchain: repository has `rust-toolchain.toml` pinned.
- `sudo -E ./scripts/dev/verify-and-refresh.sh`
  - Must complete successfully (cargo + Playwright + Docker demo + gateway validations).

Manual Review Rubric
- Architecture & Boundaries
  - Clear feature modules: `router/{bible,libraries,playlists,presentations}.rs` with minimal cross‑module coupling.
  - `router.rs` orchestrates routes only; no primary implementations.
  - State operations are mediated by `state/*` and avoid side effects in handlers.
  - Error handling uses `AppError`; every externally visible failure is actionable and logged.
- Code Organization
  - Files <=800 lines (target), hard cap 1000. Functions <=60 lines.
  - UI pages under `ui/{operator,tablet,bible,settings,home,timer_overlay}.rs`; shared bits in `ui/components/` and `ui/styles/`.
  - No large CSS inside router/state; reusable styles live in `ui/styles/` or static assets.
  - `mod.rs` used only for orchestration/re‑exports.
- Tests & E2E
  - New behavior ships with E2E covering both UI flow and side effects (DB/gateway/stage/integration triggers).
  - Tests are deterministic (fixed seeds, retry‑with‑assert, no arbitrary sleeps, animations disabled where needed).
  - No `.only`/`.skip` in tests.
- Ops & Demos
  - Branch demo refreshed; gateway card shows fresh `data-updated-at`.
  - Only `scripts/dev/show-playwright-report.sh` is used to view Playwright reports (no blocking foreground viewers).
- Data Model (Pre‑release)
  - Adjust initial migration/schema directly when model changes; regenerate SQLite; refresh dev data scripts.
- Documentation
  - ADRs added/updated for significant decisions.
  - README/AGENTS/docs remain consistent with behavior and layout.

Definition of Done (copy into PR)
- [ ] Ran `scripts/dev/quality-check.sh --strict` — result: PASS
- [ ] Ran `sudo -E ./scripts/dev/verify-and-refresh.sh` — result: PASS
- [ ] E2E includes user interaction + side‑effects for changed surfaces
- [ ] No `.only`/`.skip` in tests
- [ ] File/function sizes within limits or refactor justified in PR notes
- [ ] ADR updated/added for non‑obvious architectural choices
- [ ] Gateway card fresh; operator UI reachable; Stage link present
- [ ] No direct `playwright show-report` usage (policy enforced)
 - [ ] Format & Lint pass (`cargo fmt --check`, `cargo clippy`)
 - [ ] No `unwrap`/`expect`/`panic!` in production sources

References
- Handbook: `AGENTS.md` (Design Iteration Policy, E2E Policy, Runtime, Prompts)
- Merge checklist: `prompts/merge-prepare.md`
- Quality script: `scripts/dev/quality-check.sh`
