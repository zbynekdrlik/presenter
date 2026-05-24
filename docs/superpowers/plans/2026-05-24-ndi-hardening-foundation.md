# NDI Hardening Foundation (Items 1, 2, 6, 7) — Implementation Plan

> **For agentic workers:** Bug-fix PR. RED commit precedes GREEN commits in git log per `regression-test-first.md`. Closes #333 (partial — items 3-5 deferred to follow-up).

**Goal:** Close the two specific failure modes that took prod down on 2026-05-24: stale GStreamer registry boot race (Failure 1) and silent auto-restore re-triggering the wedged state (Failure 2). Make `/healthz` reflect pipeline state machine-readably.

**Architecture:** Four focused changes — env var before `gst::init()`, systemd `After=`/`Wants=` on the DRI render node, encoder-gated auto-restore in `AppState::initialize`, and `/healthz` extension reading a new `NdiManager::pipeline_snapshots()` accessor.

**Tech Stack:** Rust (presenter-ndi, presenter-server), systemd unit files, GStreamer.

---

## File Map

- Modify: `Cargo.toml:15` — version 0.4.94 → 0.4.95
- Modify: `crates/presenter-ndi/src/lib.rs` — env var + tests for items 1, 6 surface
- Modify: `crates/presenter-ndi/src/manager.rs` — add `pipeline_snapshots()` accessor
- Modify: `crates/presenter-server/src/state/mod.rs:393` — encoder-gated auto-restore
- Modify: `crates/presenter-server/src/router.rs:309` — `/healthz` includes `ndi_pipelines`
- Modify: `scripts/deploy/presenter.service` — `After=`/`Wants=` DRI device
- Modify: `scripts/deploy/presenter-dev.service` — same
- Add tests in respective `#[cfg(test)]` blocks.

## TDD discipline

Per `regression-test-first.md`, the RED commit MUST appear before GREEN in git log.

- RED commit: failing tests for items 1, 6, 7 (item 2 is config; tested via deploy verification, not unit)
- GREEN commits: one per item

---

### Task 1: Version bump

- Modify `Cargo.toml:15` workspace `version = "0.4.94"` → `"0.4.95"`
- `cargo update --workspace --offline` to refresh `Cargo.lock`
- Commit: `chore: bump workspace to 0.4.95`

### Task 2: RED test commit

Tests that fail against current code, pass after items 1/6/7 land.

- `crates/presenter-ndi/src/lib.rs` — assert `init()` sets `GST_REGISTRY_UPDATE=yes` BEFORE `gstreamer::init()` runs (use a probe in a fresh process is hard from within `#[test]`; instead the test asserts the env var is set immediately after `init()` returns, which is sufficient because `OnceLock` ensures one-time init within the test process)
- `crates/presenter-server/src/state/tests.rs` — assert `restore_active_source_on_startup` skips when `hw_h264_encoder()` returns None (test via a function that takes the encoder-name as parameter, dependency-injected)
- `crates/presenter-server/src/router/tests.rs` — assert `/healthz` response contains `ndi_pipelines` array
- Commit: `test: add RED regression tests for #333 items 1, 6, 7`

Verify RED:
```bash
cargo test -p presenter-ndi gst_registry_update_set_before_init -- --nocapture  # FAILS
cargo test -p presenter-server health_includes_ndi_pipelines -- --nocapture     # FAILS
cargo test -p presenter-server auto_restore_skipped_when_no_encoder -- --nocapture  # FAILS
```

### Task 3: GREEN item 1 — registry rescan

- `crates/presenter-ndi/src/lib.rs::init()` — add `std::env::set_var("GST_REGISTRY_UPDATE", "yes")` as the first line inside the `get_or_init` closure
- Add `tracing::info!` log line: `"GStreamer registry rescan forced via GST_REGISTRY_UPDATE=yes (#333 hardening)"`
- Commit: `fix(ndi): force GStreamer registry rescan at startup (#333 item 1)`

### Task 4: GREEN item 2 — systemd DRI device ordering

- `scripts/deploy/presenter.service`:
  ```
  [Unit]
  Description=...
  After=network.target dev-dri-renderD128.device
  Wants=dev-dri-renderD128.device
  ```
- `scripts/deploy/presenter-dev.service` — same
- Commit: `fix(deploy): wait for DRI render node before starting presenter (#333 item 2)`

### Task 5: GREEN item 6 — encoder-gated auto-restore

- `crates/presenter-server/src/state/mod.rs:393` — wrap the `state.ndi_manager().is_some()` block with `&& presenter_ndi::hw_h264_encoder().is_some()`. If encoder missing, log structured warning and skip restore.
- Commit: `fix(ndi): gate NDI auto-restore on encoder availability (#333 item 6)`

### Task 6: GREEN item 7 — /healthz reports pipeline state

- `crates/presenter-ndi/src/manager.rs` — add `pub async fn pipeline_snapshots(&self) -> Vec<(String, PipelineState)>` that iterates `self.active` and returns `(source_id, pipeline.state())` per entry
- `crates/presenter-server/src/router.rs::health()` — query `state.ndi_manager().map(|m| m.pipeline_snapshots())` and add `ndi_pipelines: [{source_id, state, last_error}]` field. `state` serialized as `"streaming" | "starting" | "stopped" | "errored"`. `last_error` only present for Errored state.
- Commit: `fix(ndi): expose pipeline state in /healthz (#333 item 7)`

### Task 7: Local gate

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo test --workspace
```

### Task 8: Push, monitor CI, verify on dev

- `git push origin dev` → single CI cycle
- Monitor per `ci-monitoring.md` single-sleep pattern
- After deploy-dev job green: SSH dev, journal check for `GST_REGISTRY_UPDATE=yes` log line + `dev-dri-renderD128.device` ordering. `curl http://10.77.8.134:8080/healthz | jq` confirms `ndi_pipelines` field.

### Task 9: PR + wait for explicit "merge it"

- Open PR dev → main per `pr-merge-policy.md`
- Verify mergeable: true + mergeable_state: clean
- Completion report includes `✅ Regression test:` line citing RED + GREEN SHAs

### Task 10 (post-merge): Verify on PROD

- After main CI deploys, SSH prod, verify new unit file: `grep After= /etc/systemd/system/presenter.service`
- **WITH USER APPROVAL** (per `no-destructive-remote-actions.md`): cold-reboot prod
- After boot:
  - `journalctl -u presenter -b` → presenter started AFTER `dev-dri-renderD128.device`
  - Search log for `GST_REGISTRY_UPDATE=yes` and `vah264enc` probe-success
  - `curl http://10.77.9.205/healthz | jq` confirms `ndi_pipelines` field
- **Do NOT activate NDI** — items 3-5 (CPU/memory limits, encoder fanout) come in follow-up PR; activating now is still risky.
