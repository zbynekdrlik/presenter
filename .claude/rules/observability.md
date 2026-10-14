---
paths:
  - "crates/presenter-server/src/router/request_log.rs"
  - "crates/presenter-server/src/router/tracing_capture.rs"
  - "crates/presenter-server/src/router.rs"
  - "crates/presenter-server/src/state/bible_show_log.rs"
  - "crates/presenter-server/src/main.rs"
  - "scripts/deploy/presenter.service"
  - "scripts/deploy/presenter-dev.service"
---

# Request log + Bible show lines (#850 O1)

- **The request log is a `TraceLayer` in `build_router` (`router/request_log.rs`).** One DEBUG span
  per request (`method`, `uri` = path + query, `user_agent`, `client_ip`) and one DEBUG
  `tower_http::trace::on_response` line (status, latency in ms). No "started processing" line
  (`on_request(())`), no "end of stream" line (`on_eos(())` — a drained body used to add a second
  line per response).
- **The span's target is `tower_http::trace::make_span`, on purpose.** The units run
  `RUST_LOG=info,tower_http=debug,sqlx=warn`; a span under our own target would be DEBUG under
  `info` = disabled, and the response line would print WITHOUT the method/path fields.
- **`sqlx=warn` is part of every default filter** (both units, `main.rs`'s fallback, the docs). The
  old SNV drop-in `ableset-debug.conf` carried it, so removing the drop-in alone un-silenced sqlx's
  INFO `sqlx::query` line per SQL statement (8,045 of PP's 9,293 journal lines/h). Change the three
  places together.
- **Quiet paths (`is_quiet_path`) get `Span::none()` and no response line:** `/healthz`,
  `/ai/status`, `/integrations/resolume/status`, `/ndi/time`, `/ndi/client-stats`,
  `/ndi/sessions/{id}/client-stats` and `/ui-pkg/*` — polls and assets every open operator tab or
  stage display repeats every 2–15 s. `RequestOnResponse` skips its line when `span.is_disabled()`.
  A new poll route that fires every few seconds belongs in that one pure fn (+ its unit test).
- **A 5xx is never hidden, quiet path or not:** `on_failure` is `DefaultOnFailure` at **WARN** (routine
  NDI 503 back-pressure must not be ERROR). On a quiet path that line has no request fields (no span).
- **Client-controlled strings are recorded as `&str`** (`uri`, `user_agent`, `client_ip` — the last
  comes from X-Forwarded-For), so they print quoted and a hostile header cannot forge a field. Only
  `method` (a validated token) uses `%`. The tests pin the quoted form.
- **Client IP = `integrations::extract_client_ip`** (the #793 live-WS rule): X-Forwarded-For only
  from a loopback peer (cloudflared), else the socket peer. Never a second copy of that rule.
- **`presenter::bible::show`** (INFO) is emitted at the top of `AppState::trigger_bible_slide_output`
  from the `BibleShowOrigin` the caller passes (source endpoint, device, client IP). A new caller adds
  its `BibleShowSource` variant in the same PR that constructs it — `state::bible_show_log` is
  `pub(crate)`, so an unconstructed variant fails `dead_code` under `-D warnings`.
- **Log tests use `router/tracing_capture.rs`** (`Capture::install()` — spans + events of the test's
  thread, field values as the fmt layer prints them). It keeps a SECOND live dispatcher: tracing-core
  caches callsite interest, and with one dispatcher a callsite first hit by a sibling test's thread
  is cached "never" and silently drops this test's records (same trap as `ai/client.rs`
  `GlobalCapture`). Bind the value for the whole test; filter by TARGET, never count all events.
- **Drop-ins override the unit's `RUST_LOG`.** SNV carried `ableset-debug.conf` (`tower_http=info`)
  until #850 O1; deploy.yml / release.yml now `rm -f` it and `noresolume.conf`. Dev2 still carries
  `/etc/systemd/system/presenter-dev.service.d/gst-debug.conf` with
  `RUST_LOG=presenter_server=info,…,tower_http=warn`, so dev shows NO request lines and NO
  `presenter::bible::show` lines (that target is not under `presenter_server`). Check S10 on SNV:
  `ssh newlevel@presenter.lan "journalctl -u presenter --since '-10 min' --no-pager | grep -m3 on_response"`.
