---
paths:
  - "crates/presenter-ui/src/pages/**"
  - "crates/presenter-ui/src/components/**"
  - "crates/presenter-ui/styles/**"
  - "crates/presenter-ui/src/state/ui_usage.rs"
  - "crates/presenter-server/src/router/ui_usage.rs"
  - "crates/presenter-persistence/src/repository/ui_usage.rs"
  - "crates/presenter-server/src/ui/home.rs"
  - "crates/presenter-server/src/ui/styles/home.css"
---

# GUI usage: read it before deciding what gets a place (#913)

Owner, 2026-11-07: "mas mat inmplementovany zber pouzivania gui aby si sa spravne rozhodol co je dolezite a comu si dal dolezite misto no nikdy sa to nepouziva!!"

- **Before any change of what is shown, moved, hidden or given room, read the counters from SNV and cite them in the ticket's design comment:**
  `curl -s 'http://10.77.9.205/ui/usage?days=90'` (`days` 1–400, default 30). Rows `{page, shell, role, count}`, most used first; `shell` = `phone` or `desktop`.
- **The phone comes first** (owner, same day): the phone rows decide, then the desktop follows.
- **Few counts decide nothing.** Say how many there are. The owner's own words beat a thin count. The SNV request log (`journalctl -u presenter`, user agent + URI) adds what a request shows: which translations a chapter was read in, for example.
- **What is counted:** one document click listener (capture) in `state/ui_usage.rs` takes the nearest `[data-role]`, so a control needs a `data-role` to count. Page, shell, role and a count leave the browser, never text. So a `data-role` never holds user text, and the server's `is_name` drops a row that does. Scrolling and typing are not counted.
- **Sent by `sendBeacon`** after 10 s, on `pagehide` and when the page goes hidden, so the body is `text/plain`. The leave listeners come with the first press: a stage display nobody presses holds none, and `stage-ndi-*` specs count its listeners exactly. `/ui/usage` is a quiet path in the request log.
- **The home page `/` (#938, `presenter-server/src/ui/home.rs`)** orders its "Najčastejšie" tiles by
  these counters (90 days, 2026-11-12: operator 440, study 302, tablet 69; the operator's AI tab, 19
  presses (15 from a phone), sits above the tablet because the owner asked for it on his phone). The order is
  fixed in the code on purpose (tiles never move under the thumb); re-read the counters before
  changing it. The page is SSR, not WASM: it has no `data-role` and counts nothing itself.
- **Stored per day** in `ui_usage`, site-local (no sync, PP keeps its own), kept 400 days.
