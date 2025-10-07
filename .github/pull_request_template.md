Title: <concise summary>

Linked Issues
- Closes #<id> (or) Part of #<id>

Checklist — Readiness to Review/Merge

Core Validation
- [ ] Follows TDD: specs/tests added first or alongside code
- [ ] `cargo test` passes locally
- [ ] `npm run test:playwright` passes (no retries)
- [ ] No focused/skipped tests: `rg -n "\.only\(|describe\.only|it\.only|test\.only|\.skip\(" tests/e2e` returns nothing
- [ ] `sudo -E ./scripts/dev/verify-and-refresh.sh` completed successfully (demo + gateway rebuilt)
- [ ] Gateway landing page validated (one Stage link per demo; no legacy Stage variants)

E2E Coverage (required for user‑visible/integration changes)
- [ ] E2E simulates the real UI flow (forms, toggles, nav) and asserts observable side effects
- [ ] For integrations (if applicable):
  - [ ] AbleSet mock `/api/setlist` + OSC `/note` assertions
  - [ ] Resolume status/clip mapping
  - [ ] Stage snapshot/latency
  - [ ] Companion/Android where applicable

Code Organization & Limits (2025 guidelines)
- [ ] File size within guidance (~800 lines target; hard ceiling 1,000). If exceeded, justify or split
- [ ] Functions are focused (≈60 lines target) with clear responsibilities
- [ ] UI pages live under `crates/presenter-server/src/ui/` (operator/tablet/bible/settings/stage)
- [ ] Shared UI pieces under `ui/components/`; avoid monolithic `ui.rs`
- [ ] Router split by feature under `router/`; imports re‑exported from `router/mod.rs`
- [ ] `state.rs` heavy impls split to `state/<feature>.rs` (or justified)
- [ ] CSS kept in small constants or `ui/styles.rs`/static file; no large inline CSS with router/state logic

Docs
- [ ] Markdown updated for any behaviour/structure changes (runbook, AGENTS.md, ADRs)
- [ ] PR body includes brief rationale and testing evidence (screenshots/logs where useful)

Operational Notes
- [ ] Demo/dev environment instructions (if needed) included
- [ ] No breaking changes without explicit migration notes (pre‑release: keep single initial migration policy)

Risks & Rollback
- [ ] Risks identified and mitigations noted
- [ ] Safe to rollback by reverting the merge if needed

Screenshots / Evidence
<optional>
