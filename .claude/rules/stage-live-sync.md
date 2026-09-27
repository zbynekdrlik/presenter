---
paths:
  - "crates/presenter-ui/src/ws/stage.rs"
  - "crates/presenter-ui/src/pages/stage.rs"
  - "crates/presenter-ui/src/pages/stage_events.rs"
  - "crates/presenter-ui/src/pages/camera.rs"
  - "crates/presenter-server/src/live.rs"
  - "crates/presenter-server/src/stage_connections.rs"
  - "crates/presenter-server/src/android_stage.rs"
  - "crates/presenter-server/src/android_stage/**"
  - "tests/e2e/stage-layout-sync.spec.ts"
---

# Stage displays: lossless live events, adb hygiene, WS diagnostics (#793)

SNV 2026-09-27: sd2–sd4 "dropped out as if restarting" mid-event. Three separate causes; each has
a rule here so it does not come back.

## 1. The launcher must NEVER install/uninstall on an unreadable adb state

`pm path` is read tri-state (`adb.rs::parse_package_state` → `Installed` / `NotInstalled` /
`Unknown`). Only a silent exit 0/1 is "absent"; ANY adb error text (`error: device offline`,
`error: device still authorizing`, `error: device '<ip>:5555' not found`), exit 255, or odd stdout
is `Unknown` → `ensure_app_installed` WARNs and skips the cycle. The old bool collapsed every adb
failure into "not installed" and the #734 uninstall fallback KILLED the running stage app
(logcat `Killing … due to installPackageLI`). Never add a new adb read that maps an error to a
destructive default — make it tri-state and treat Unknown as "do nothing this cycle".

## 2. Never churn a connected adb target

`adb_connect` reads `adb devices` once per cycle: a target in state `device` is left alone; a
missing/stuck target gets a plain `adb connect`; `adb disconnect` only after
`STALE_RECONNECT_CYCLES` (3) consecutive stuck cycles (`decide_connect` + the per-display
`AdbLinkState` owned by `run_device_worker`). Only `offline`/unexpected states count as stuck —
`unauthorized`/`authorizing` never get disconnected (that re-shows the TV's RSA prompt while
someone may be accepting it). The old unconditional disconnect+connect every 20 s itself produced
the offline/authorizing states. Repeating adb WARN/INFO lines (unknown package state, stale
reconnects) go through `should_log_adb_streak` (1st + powers of two) + one recovery line. Also: SNV had a legacy, repo-unmanaged
`stage-watchdog.timer` (`/opt/presenter/stage-watchdog.sh`) fighting the same adb server — it was
disabled on 2026-09-27 (rollback `sudo systemctl enable --now stage-watchdog.timer`); if adb flaps
again, check `systemctl list-timers` for a second adb client first.

## 3. Stage live events are applied directly, never through one latest-value signal

`use_stage_websocket(client_id, layout_code, on_event)` calls the `StageEventHandler` synchronously
for every event in arrival order. A `ReadSignal<Option<LiveEvent>>` "last event" slot COALESCES a
burst (`StageLayout` immediately followed by the `Stage` snapshot the switch publishes) and the
earlier event is lost before the page effect runs. The operator/tablet `use_live_websocket` still
uses that pattern — do not copy it into any display that must not miss an event.

Layout reconciliation (`pages/stage_events.rs::stage_snapshot_action`): the server publishes ONLY
the selected layout's snapshot plus the always-on `camera-crew` one (api snapshot only while api
is selected — `broadcasting.rs::publish_stage_context`, `api_stage.rs`). So on `/stage` a snapshot
for another layout means "this is the active layout" → adopt it; `camera-crew` → ignore. If the
server ever starts publishing snapshots for non-selected layouts, this invariant breaks — update
`stage_snapshot_action` in the same PR. `/ui/camera` stays pinned to `camera-crew`.

**Server ordering invariant (load-bearing for adoption):** `switch_stage_layout` publishes
`StageLayout` INSIDE the `stage_layout` write-lock block; every `LiveEvent::Stage` publisher reads
the selected code and publishes under the READ lock with no await in between (build/enrich FIRST):
`publish_stage_context`, `update_api_stage`, and the switch's own api-snapshot publish. So layout
events are totally ordered and a snapshot for the previous layout can never follow a switch's
`StageLayout`. Any NEW `Stage` publisher must follow the same rule, or a late stale snapshot flips
every display back. Never await while holding the guard (tokio RwLock is write-preferring — a
nested read behind a queued writer deadlocks).

The live hub does NOT replay: anything published while a display's socket is resetting is lost.
`pages/stage.rs::resync_stage_state` re-reads layout + snapshot + broadcast + Bible overlay on page
load and on every `Connected` transition — the same pattern as `sync_ndi_source_state`. Its
layout/snapshot answers are discarded when a live `StageLayout` or an APPLIED `Stage` event arrived
while the fetch was in flight (`StageSyncGeneration`) — the live event is newer. The ignored
camera-crew snapshot must NOT count (it is published on every broadcast; with api selected it is
the only one) or every resync would be voided.

**E2E technique:** `page.routeWebSocket(/\/live\/ws/, ws => { const server = ws.connectToServer();
server.onMessage(m => { …filter…; ws.send(m); }); })` reproduces lost frames deterministically
(drop a frame type, or black out one specific route object and then `route.close({ code: 4000 })`
to force a reconnect). Filter per-ROUTE object, not with a time window — a frame published before
the flag flips can still be in flight. See `tests/e2e/stage-layout-sync.spec.ts`.

## 4. Live WS diagnostics

`main.rs` serves with `into_make_service_with_connect_info::<SocketAddr>()`; `/live/ws` extracts
the peer as `Option<Extension<ConnectInfo<SocketAddr>>>` (axum 0.8's `ConnectInfo` has no optional
extractor, and tests serving `axum::serve(listener, app)` have no connect-info — a bare
`ConnectInfo<_>` extractor would 500 them). `extract_client_ip`: a non-loopback peer IS the client
(its forwarding headers are ignored — no spoofing); only from a loopback peer (cloudflared) or no
peer is X-Forwarded-For → X-Real-IP used; else `anonymous`. `serve_websocket` ends when EITHER
side ends (the write-side task is raced only against the cancel-safe `receiver.next()`, never
against an in-progress `dispatch_inbound`) and logs `reason=` (`WsEndReason`: client close code/reason, read error, stream ended,
send failed, hub closed) + `stage_client` + `connected_ms`. Layout switches log
`presenter::stage::layout` INFO `from`/`to`. Read them with
`journalctl -u presenter | grep -E 'live ws client|stage::layout'`.

**Per-display layout (#797):** the `StagePresence` frame is sent on socket open, BEFORE the layout
resync, so it carries the `worship-snv` default. `StageConnectionTracker::record_diag` therefore
adopts every non-empty `NdiVideoDiag.layout_code` (the DISPLAYED layout) into the connection, and
the (non-empty) layout is part of `DiagLogKey` so a switch logs `presenter::stage::diag` on the
first diag frame that reports it (next diag push / heartbeat ack — the client `DiagChangeKey` has
no layout). Limit: the client only sends a diag while an NDI `<video>` is mounted
(`ws/stage_diag.rs::collect_ndi_video_diag`), so the tracker holds "the last layout reported while
an NDI video was mounted" — a switch to a non-NDI layout is NOT reflected. Never read the
register-time layout as the truth for `/stage/connections` or the diag log.
