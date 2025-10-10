Goal
Split crates/presenter-server/src/state.rs into smaller, focused modules and remove the temporary quality-check allowlist for file size.

Plan
- Extract: state/stage.rs (StageContext, StageResolution, build_stage_snapshot, sanitize_song_title, blank_slide_content)
- Extract: state/timers.rs (format_countdown_text and timer helpers)
- Extract: state/companion_server.rs (CompanionServerManager + handle impl)
- Keep AppState public surface stable; internal items pub(super) with re-exports as needed.
- Move remaining tests under state/tests.rs if any.
- Remove allowlist from scripts/dev/quality-check.sh; restore strict cap.
- Run sudo -E ./scripts/dev/verify-and-refresh.sh and ensure demo/gateway validations pass.

Acceptance Criteria
- state.rs < 800 lines; no function > 60 lines unless justified with #[allow] (with comment)
- cargo test and Playwright pass via verify
- quality-check passes with strict file-size enforcement
- No behavior changes

Links
- Related: Issue #41 (Recurring Quality & Architecture Review)
- Current branch: feature/issue-41-code-organization
