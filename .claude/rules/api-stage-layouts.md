---
paths:
  - "crates/presenter-server/src/state/api_stage.rs"
  - "crates/presenter-server/src/state/stage_text_mode.rs"
  - "crates/presenter-server/src/router/api_stage.rs"
  - "crates/presenter-core/src/stage_display.rs"
  - "crates/presenter-core/src/stage_text_mode.rs"
  - "crates/presenter-ui/src/components/stage/api_stage.rs"
  - "crates/presenter-ui/src/components/stage/api_ambient.rs"
  - "crates/presenter-ui/src/components/stage/api_text.rs"
  - "crates/presenter-ui/src/components/stage_text_mode_picker.rs"
  - "crates/presenter-ui/styles/stage_ambient.css"
  - "tests/e2e/api-stage.spec.ts"
  - "tests/e2e/api-ambient-text-mode.spec.ts"
---

# API stage layouts: `api` + `api-ambient`, translation, text mode (#799)

`PUT /api/stage` (songplayer) feeds ONE stored `ApiStageState` that two layouts render:
`api` (WorshipSnv boxes over optional NDI) and `api-ambient` (fullscreen NDI/CG video, lyric
overlay only while text is present, the bottom StatusBar kept, no header chrome). The payload carries optional
`currentTranslation`/`nextTranslation` (serde default "" — old clients unchanged), mapped into
`StageDisplaySlide.translation`.

## Every "is this an API layout?" check uses `is_api_stage_layout(code)`

`presenter_core::is_api_stage_layout` (api OR api-ambient) is the ONE predicate — snapshot routing
(`stage_display.rs::stage_display_snapshot` / `selected_stage_display_snapshot`), the resolution
broadcast skip (`broadcasting.rs::publish_stage_context`), the switch probe/publish
(`context_for_pending_switch`, `persist_and_broadcast_switch`), persisted-layout validation. Never
compare to `"api"` / `API_STAGE_LAYOUT_CODE` directly — a new API-fed layout would silently get the
presentation snapshot and overwrite the API text. Adding a third API layout = extend the predicate
+ `StageDisplayLayout::api_layout_for`.

## The api snapshot's layout + text mode are stamped UNDER the layout read lock

Displays ADOPT a snapshot's layout (`stage-live-sync.md` §3). The api snapshot is built async
(group colors, timers) with the `api` layout as a placeholder, then `publish_api_snapshot` takes the
`stage_layout` read lock and `stamp_api_snapshot` sets `layout = api_layout_for(selected)` and
`text_mode` — sync, no await — before publishing. So a switch `api` <-> `api-ambient` between build
and publish can never publish the other layout's code (it would flip every display back). Every api
snapshot publisher goes through `publish_api_snapshot` (`update_api_stage`, `republish_api_snapshot`
used by the switch and by a text-mode change). `republish_api_snapshot` holds the `api_stage` READ
guard across build + publish, so a concurrent `PUT /api/stage` (which must take the WRITE lock
first) can never publish its new text and then be overwritten by the older republish. Lock order:
`api_stage` → group-color cache / timers → `stage_layout`; never acquire `api_stage` while holding
one of the others.

## Text mode = persisted setting, atomic cell, carried INSIDE the api snapshot

`StageTextMode` (`original` | `translation` | `both`, default `both`) lives in
`state/stage_text_mode.rs`: an `Arc<AtomicU8>` (`StageTextModeCell`) so it can be read under the
layout lock without a second lock, plus a setter `tokio::sync::Mutex` that serializes a whole
change (swap → persist → event → republish) so two operators can't leave DB / memory / displays
out of step; persisted in `app_settings` key `feature.stage.text_mode` (same
no-audit k/v as `feature.stage.layout`); restored in `from_config` (pure read). A change publishes
`LiveEvent::StageTextMode` (operator pickers) AND re-publishes the api snapshot, whose
`textMode` field is what DISPLAYS read — so the existing reconnect resync (`GET /stage/snapshot`)
covers the mode with no extra fetch. Non-api snapshots have `textMode: None` (omitted on the wire).
`GET`/`PUT /stage/text-mode` `{"mode": …}`; an unknown mode is a typed-`Json` 422.

## Rendering: one pure helper

`components/stage/api_text.rs::select_api_lines(main, translation, mode)` decides the lines
(host-tested): `translation` falls back to the original when none was sent; `both` with no/identical
translation == `original`; `original` with only a translation sent shows nothing. `api` uses it via
`WorshipSnv api_text_mode=true` (both → joined on a new line inside the SAME boxes — never change
box sizes, project Always-Rule); `api-ambient` renders primary (large) + secondary (smaller, below).
`WorshipSnv` tail-breaks each line BEFORE joining (`api_box_text` → `ApiLines::map_lines`) —
`break_if_long` skips any text that already contains a newline, so breaking the joined string would
silently disable it in `both` mode. The ambient overlay keeps the last non-empty lines while fading
out and ends `visibility: hidden` (Playwright `toBeHidden`, opacity alone reads as visible). Its
text boxes MUST stay `display: block` (top-down flow): `autofit_text` detects overflow via
`scrollHeight > clientHeight`, which never counts overflow ABOVE a box — a `flex-end`/bottom-aligned
text box overflows upward, autofit never shrinks it, and long lines get clipped. The primary fit
re-runs when the secondary line shows/hides (the box height changes on a both ↔ original switch). `api-ambient` reuses the ndi_fullscreen
`Memo` dedup for `<NdiVideo>` (NVENC session leak otherwise) and shows NO NDI status overlays —
black while no source is live.

## `api-ambient` KEEPS the bottom StatusBar (owner ruling on #799)

"Ambient" never means "strip everything": the owner reversed the original "no clock/status
chrome" design — `ApiAmbient` takes `ws_state`/`latency_ms` from `pages/stage.rs` and renders
`<StatusBar … hide_live=true hide_song_number=true />` with EXACTLY the `ndi-fullscreen` flags
(clock + connection + video-latency + version; no live pill, no song number). The StatusBar's
boxes are the shared `stage.css` ones (bottom 7% of the container) — never restyle them per
layout. The lyric overlay is anchored `bottom: calc(7% + 1vh)` (1vh gap above the bar top — an exact `7%` overlapped by 1px from sub-pixel rounding in CI) so
it sits ABOVE the bar and never overlaps it; the E2E asserts overlay bottom <= clock/connection
top. If the StatusBar height ever changes, move the overlay's `bottom` with it.
