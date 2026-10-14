---
paths:
  - "crates/presenter-server/src/router/request_log.rs"
  - "crates/presenter-server/src/tracing_capture.rs"
  - "crates/presenter-server/src/router/integrations/ndi_whep.rs"
  - "crates/presenter-server/src/router.rs"
  - "crates/presenter-server/src/state/bible_show_log.rs"
  - "crates/presenter-server/src/main.rs"
  - "scripts/deploy/presenter.service"
  - "scripts/deploy/presenter-dev.service"
  - ".github/workflows/deploy.yml"
  - ".github/workflows/release.yml"
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
- **The span prefixes every INFO/WARN line logged inside a non-quiet handler.** The fmt layer
  prints the enclosing span's fields before the message, so under the units' filter
  (`tower_http=debug` enables the span) a handler's own line reads
  `request{method=POST uri="/bible/trigger-slide" user_agent="Mozilla/5.0 …" client_ip="203.0.113.9"}:
  presenter::bible::show: …`. Consequences: a `grep` for the target or message still matches, but a
  `^`-anchored pattern does not; every such line carries the user agent (long); work moved onto a
  `tokio::spawn`ed task does NOT inherit the span (no prefix there); a quiet path has no prefix
  (`Span::none()`); and where `tower_http` is below debug (dev2's `gst-debug.conf`) the span is
  disabled, so no prefix either.
- **Quiet requests (`is_quiet_request`) get `Span::none()` and no response line.** Two lists, both
  pure fns with unit tests: `is_quiet_path` (any method) — `/healthz`, `/ai/status`,
  `/integrations/resolume/status`, `/ndi/time`, `/ndi/client-stats`,
  `/ndi/sessions/{id}/client-stats` and `/ui-pkg/*`, polls and assets every open operator tab or
  stage display repeats every 2–15 s — and `is_quiet_get_path` (GET ONLY) — the list / status GETs a
  page re-fetches on a timer: `/bible/presentations` (tablet, 10 s), `/integrations/resolume/hosts`,
  `/integrations/android-stage/displays`, `/integrations/ableset/status`, `/integrations/osc/status`,
  `/integrations/video-sources/status` (Settings tab, 5 s). The GET split matters: the same paths take
  a person's POST / PUT / DELETE (create, edit, remove a host / display / presentation), and those stay
  logged. `RequestOnResponse` skips its line when `span.is_disabled()`. A new timer-polled route
  belongs in one of those two fns (+ the unit test); a route a person triggers never does.
- **A silent WHEP poll has a span but no response line (`is_silent_whep_poll`).** With an NDI
  broadcaster silent, every NDI stage display's reconnect loop POSTs `/ndi/whep/{source_id}` every 5 s
  and gets 204 (~720 lines/h per display). The status is unknown when the span opens, so the decision
  is in `RequestOnResponse`: `tag_request_line` (a `from_fn` middleware installed INSIDE the trace
  layer by `with_request_log`) stamps method + URI on the response extensions, and a POST to exactly
  `/ndi/whep/{one segment}` answered 204 logs no response line. A 201 session start, every error and
  every session-scoped WHEP request (POST / PATCH / DELETE `/ndi/whep/{id}/{session}`) stay logged.
  Install the layers ONLY through `with_request_log` — the trace layer without the tag fails open
  (logs the poll again); the whep test in `request_log.rs` pins the order.
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
- **Log tests use the ONE helper, `src/tracing_capture.rs`** (`#[cfg(test)] mod tracing_capture` in
  `lib.rs`; `crate::tracing_capture::{Capture, quoted}`). `Capture::install()` records spans + events of
  the test's thread, field values as the fmt layer prints them (`field("attempt")` is the text `"3"`,
  a `&str` field is quoted). `events(target)` / `spans(target)` filter by TARGET;
  `events_with_message(needle)` filters events by message text across targets (the #858 silent-streak
  tests in `state/integrations.rs`). Never write a second capture subscriber: extend this one. It keeps
  a SECOND live dispatcher: tracing-core caches callsite interest, and with one dispatcher a callsite
  first hit by a sibling test's thread is cached "never" and silently drops this test's records (same
  trap as `ai/client.rs` `GlobalCapture`). Bind the value for the whole test; never count ALL events.
- **Drop-ins override the unit's `RUST_LOG`.** SNV carried `ableset-debug.conf` (`tower_http=info`)
  until #850 O1; deploy.yml / release.yml now `rm -f` it and `noresolume.conf`. Dev2 still carries
  `/etc/systemd/system/presenter-dev.service.d/gst-debug.conf` with
  `RUST_LOG=presenter_server=info,…,tower_http=warn`, so dev shows NO request lines and NO
  `presenter::bible::show` lines (that target is not under `presenter_server`). A drop-in can no
  longer sneak back onto prod silently: deploy.yml / release.yml run the step `Verify effective
  RUST_LOG (#850 O1)` after `Verify deployment` — the running service's `systemctl show -p Environment`
  must carry `RUST_LOG=<the committed unit's value>` entry for entry (the expected value is read from
  `scripts/deploy/presenter.service`, no second copy), else the deploy fails and prints the
  Environment line. pipeline.yml (dev) has no such step: dev2 keeps `gst-debug.conf` on purpose.
  Check S10 on SNV:
  `ssh newlevel@presenter.lan "journalctl -u presenter --since '-10 min' --no-pager | grep -m3 on_response"`.
