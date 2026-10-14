---
paths:
  - "crates/presenter-server/src/router/request_log.rs"
  - "crates/presenter-server/src/router.rs"
  - "crates/presenter-server/src/state/bible_show_log.rs"
  - "scripts/deploy/presenter.service"
  - "scripts/deploy/presenter-dev.service"
---

# Request log + Bible show lines (#850 O1)

- **The request log is a `TraceLayer` in `build_router` (`router/request_log.rs`).** One DEBUG span
  per request (`method`, `uri` = path + query, `user_agent`, `client_ip`) and one DEBUG
  `tower_http::trace::on_response` line (status, latency in ms). No "started processing" line
  (`on_request(())`); a 5xx still gets tower-http's default ERROR line.
- **The span's target is `tower_http::trace::make_span`, on purpose.** The units run
  `RUST_LOG=info,tower_http=debug`; a span under our own target would be DEBUG under `info` =
  disabled, and the response line would print WITHOUT the method/path fields.
- **Client IP = `integrations::extract_client_ip`** (the #793 live-WS rule): X-Forwarded-For only
  from a loopback peer (cloudflared), else the socket peer. Never a second copy of that rule.
- **`presenter::bible::show`** (INFO) is emitted at the top of `AppState::trigger_bible_slide_output`
  from the `BibleShowOrigin` the caller passes (source endpoint, device, client IP). A new caller adds
  its `BibleShowSource` variant in the same PR that constructs it — `state::bible_show_log` is
  `pub(crate)`, so an unconstructed variant fails `dead_code` under `-D warnings`.
- **Drop-ins override the unit's `RUST_LOG`.** SNV carried `ableset-debug.conf` (`tower_http=info`)
  until #850 O1; deploy.yml / release.yml now `rm -f` it and `noresolume.conf`. Dev2 still carries
  `/etc/systemd/system/presenter-dev.service.d/gst-debug.conf` with
  `RUST_LOG=presenter_server=info,…,tower_http=warn`, so dev shows NO request lines and NO
  `presenter::bible::show` lines (that target is not under `presenter_server`). Check S10 on SNV:
  `ssh newlevel@presenter.lan "journalctl -u presenter --since '-10 min' --no-pager | grep -m3 on_response"`.
