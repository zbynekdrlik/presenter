# CLAUDE.md

> **Version:** 2025.7 | **Last Updated:** 2026-03-16

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

<!-- Global rules applied via airuleset modules (~/devel/airuleset/):
     core/complete-planned-work, core/completion-report, core/autonomous-verification,
     core/ci-monitoring, core/ci-push-discipline, core/pr-merge-policy,
     core/tdd-workflow, core/git-fetch-first, core/version-bumping,
     git/two-branch-workflow, git/commit-conventions,
     ci/test-strictness, ci/no-continue-on-error, ci/e2e-real-user-testing,
     ci/browser-console-zero-errors, quality/rust-web-stack, quality/mvp-philosophy,
     quality/architecture-first, quality/security-basics,
     deploy/post-deploy-verification, deploy/ssh-deployment
-->

---

## Playbook Router

| Topic | Where to look |
|---|---|
| NDI pipeline, WebRTC testing, cleanup | `.claude/skills/ndi/SKILL.md` |
| Companion plugin, /etc/hosts, batching | `.claude/skills/companion/SKILL.md` |
| Companion action option rename ⇒ append-only upgrade script + handler fallback; testing the real `index.js` via `lib/test-support.js` (#805) | `.claude/rules/companion-upgrade-scripts.md` (auto-loads on `ops/companion/presenter/**`) |
| Companion dropdown choices (layouts, stream outputs/scenes/overlays) = the server `catalog` push, never a hardcoded list (#814): `companion/catalog.rs` snapshot + change-gated re-send on `StreamConfigChanged`, `lib/catalog.js` builders, same option ids + `allowCustom` so saved buttons keep working | `.claude/rules/companion-catalog.md` (auto-loads on `ops/companion/presenter/**`, `crates/presenter-server/src/companion/**`) |
| Runner management, GPU wedge, probe cleanup, branch-sync-after-merge | `.claude/skills/ci/SKILL.md` |
| Local build/deploy workflow | `.claude/skills/deploy/SKILL.md` |
| Leptos/WASM frontend gotchas (view! macro, keyed `<For>`) | `.claude/skills/ui/SKILL.md` |
| Database schema, migrations, settings audit, library import | `.claude/skills/database/SKILL.md` |
| Repository refusal → HTTP 404/409/422 typed pattern (RepositoryError) | `.claude/rules/repository-error-pattern.md` (auto-loads on `repository/**`, `router/**`, `state/**`) |
| Stream-graphics subsystem (epic #718): serde tag, i32/i64 ids, config_revision vs activation, error taxonomy, test idiom; playback element (#843) = a keyless Google Drive folder mirror (`drive/` client + `playback/` sync worker, id-addressed Range-served cache, H.264 E2E only in real Chrome `@video-codec`) | `.claude/rules/stream-graphics.md` (auto-loads on `**/*stream*.rs`, `drive/**`, `playback/**`) |
| Stream-graphics WASM operator editor `/ui/stream` (#713): def+events client model, generic ws/api reuse, reorder full-id-set, untracked handler reads, canvas gesture state machine + output-switch def reset (#787), async-list `<select>` = `prop:selected` per option (#827), Scény/Menovky/Písma tabs = mounted panels + merged URL params (#829), output bar = create/rename outputs with the slug derived from the name + the OBS URL (#842), CI clippy-excludes presenter-ui | `.claude/rules/stream-editor-ui.md` (auto-loads on `pages/stream_editor.rs`, `components/stream_editor/**`) |
| Nameplate plate list = ONE shared component (#871): `components/nameplates` (`NameplateStore` = every plate REST call + DTOs + paths, `plate_list.rs` views, pure `numbering.rs` = Stream Deck button 1 song / i+2 persons / 12-button grid), used by the editor's Menovky tab (+ Prehrať, create layer) and the operator's Menovky view (`/ui/operator/menovky`, tab after AI, `stream` only, lazy mount, live via `apply_event`); row numbers = a `Memo` over the position, never captured; per-field drafts saved when the field was LEFT (active element AND `document.hasFocus()`; a window blur or a hidden page saves, a reorder never saves early, a deleted half-typed plate toasts); a row move keeps the focus + caret (`focus::keep_focus`); list responses sequenced (`list_gen`), reorder PUTs and per-plate saves (`SaveQueue`) serialized; drag = pointer events on a `touch-action:none` handle, the drop line read from the current list | `.claude/rules/nameplates-ui.md` (auto-loads on `components/nameplates/**`, `editor_nameplates.rs`, `output_switch.rs`, `styles/nameplates.css`, the specs) |
| Adding a `LiveEvent` variant → the one exhaustive match (`companion/variables.rs`) that breaks (E0004) | `.claude/rules/live-events.md` (auto-loads on `live.rs`, `companion/variables.rs`) |
| Sync/LWW invariants (synthetic tombstones, clock bumps, best-effort recon) | `.claude/rules/sync-lww.md` (auto-loads on `repository/sync*`, `library_sync*`, `state/sync.rs`) |
| AI-eval harness (lib.rs split, Cargo autobins gotcha, minimal pub widening) | `.claude/rules/ai-eval-harness.md` (auto-loads on `bin/ai_eval/**`, `lib.rs`, `scripts/dev/ai-eval/**`) |
| Local quality-gate gotchas (fn-length counts wrapped signature lines, file-size cap) | `.claude/rules/quality-gates.md` (auto-loads on `crates/**/*.rs`) |
| Down-dependency log floods → resolume #484 backoff + power-of-two log gate (AbleSet/Resolume/NDI/ADB pollers); a timer-driven repeat is counted where the outcome is decided — the #858 silent-start streak (`SourceSilent { attempt }`, INFO on attempts 1,2,4,8…, build-time lines DEBUG, ONE INFO `NDI pipeline streaming` on recovery); log tests use the one `tracing_capture` helper | `.claude/rules/log-flood-backoff.md` (auto-loads on `ableset.rs`, `resolume/**`, `manager/**`, `pipeline/build.rs`, `pipeline/encoder_timing.rs`, `state/integrations.rs`, `state/background_tasks.rs`, `android_stage.rs`) |
| Bible page DOM contract + async-effect E2E determinism (#727): collapsed/full book-item attribute parity, settle-signal over expect.poll, live-server repro; chapter/verse inputs bounded to the book with an inline hint, end never collapsed to "all" (#825) | `.claude/rules/bible-page.md` (auto-loads on `pages/bible.rs`, `pages/bible_reference.rs`, `state/bible*.rs`, `wasm-bible.spec.ts`) |
| NdiManager active-map locking: never hold `active` across build/wait/stop; the #741 reservation activation pattern; seam testing without libndi | `.claude/rules/ndi-manager-locking.md` (auto-loads on `manager/**`) |
| NDI shared-encoder timestamp guard (#854): no frame leaves the encoder more than D = 50 ms later than the clock (vah264enc 1.24 frame-counter lead; most SNV pipelines stay untouched, a start-up lead above D (late first frame, NDI burst) is cut to D once, drift is capped), convert through running time + the OUTPUT segment never a raw PTS, commit only after the conversion, identity on PTS-preserving encoders; CI can't see the bug → the x264enc frame-counter emulator test + its unguarded negative control + the SNV start-up burst test; drift self-heal reads only the pipeline lead (never `dropRatio30s`), counted per pipeline generation, recorded only once confirmed, ≥ 5 min apart, ≤ 3/h; `futureTimestamps` verdict | `.claude/rules/ndi-encoder-timestamps.md` (auto-loads on `pipeline/build.rs`, `encoder_timing.rs`, `time_window.rs`, `link_probe.rs`, `manager/supervisor.rs`, `drift_guard.rs`) |
| Presenter Stage APK stable-signing: committed debug keystore + guard, don't delete it (#740) | `.claude/rules/android-stage-signing.md` (auto-loads on `android/presenter-stage/**`) |
| Resolume clip triggers (#807): one `trigger_clips` batch is CONCURRENT, so same-layer clips race (last wins); sequence with two calls. Bible clear (#841) = opacity FADE of every Bible lane clip over its layer's transition, NO trigger, no lane flip; `TrackedOpacities` entered before the first fade PUT and kept after the restore (a fetch at 0 must not decide later fades), synced to each fetch's values, restored before the next connect; every connected clip is remembered so a clear after a deck switch still fades the clip Resolume keeps playing (unmapped 404 = forget, not a stale mapping); the A/B lanes are seeded once from the clips' `connected` state after a start (`lane_seed.rs`); `#bible-clear` (#848) = an optional COVER (not in `destinations()`), never triggered: a successful clear schedules a fade UP to its base, a connected verse a fade DOWN to 0, DEFERRED in `bible_clear_cover.rs` (worker `select!` branch after commands, 50 ms steps, pushes interleave, a new schedule replaces), first step reads `GET /parameter/by-id` (PUTs only a no-transition cover's target), each cover fades over `fade × distance left / base`, best-effort (never fails a push/status/backoff; a failed step gets ONE final step at the target; a 404 leaves only that cover out, mapped = one stale-id refetch per mapping and per id, later ones WARN through the #563h limiter), covers remembered as a set of ≤4 per layer (`bible_clear_cover_memory.rs`: a fetch collapses a layer once, to all the covers it saw PLAYING there, others are added, the oldest not playing goes); tests drive the real worker | `.claude/rules/resolume-clip-triggers.md` (auto-loads on `resolume/handlers.rs`, `bible_clear*.rs`, `lane_seed.rs`, `driver.rs`, `clip_map.rs`, `types.rs`) |
| Resolume port drift (#564/#744/#813): never adopt a port another host on the same machine owns (same host string or a shared resolved IP, re-checked every tick; a disabled host still owns its configured port, an enabled one also its adopted port; `port_claims.rs` table rebuilt by `set_hosts`, a persisted `active_port` is only a seed: kept unless another host owns it, else dropped before the worker is seeded; check-and-claim on adopt; a running host's sibling-owned port dropped + persisted on `SiblingsChanged`); tests use `free_port_pair()` / `free_port_triple()`, never `free_port()+N` | `.claude/rules/resolume-port-drift.md` (auto-loads on `resolume/port_drift*.rs`, `resolume/port_claims.rs`, `resolume/mod.rs`, `resolume/driver.rs`, `resolume/mapping_refresh.rs`) |
| Resolume composition fetches (#808): NEVER read `/composition` (16 MB on SNV) on a timer — the 10 s tick (and the Test button) is a `/product` probe; fetch only on missing / error-invalidated (#563b threshold = recovery) / stale-id (push 404 → typed `StaleIdError`, invalidate + retry once; 404 on a fresh mapping → 60 s pause) / manual ("Refresh mapping" endpoint + settings button); every new PUT/connect maps 404 → `StaleIdError`; keep pushes idempotent until success (flip/dedup after OK); one `resolume_http_client()`, no connection reuse; a fetch can hit Arena MID-LOAD (PP: 27 tag-less clips) and a deck switch changes every clip id, both with NO 404 → `provisional_mapping.rs`: per-deck last-good kinds, suspect → follow-ups 2/5/15/30/60 s (select! deadline, never a timer), deck check `decks/by-id/{id}` in `ensure_mapping_for_push` + on the tick (switch → inline refetch, failed check → push anyway), lane refetch re-armed on every follow-up step, a used one waits only 3 s (owner: no text skipped; only a deck accepted as lacking a kind stops refetching), an unchanged composition settles as the deck's content; mock Arenas listing `decks` must serve the deck route | `.claude/rules/resolume-mapping-refresh.md` (auto-loads on `resolume/driver.rs`, `resolume/mod.rs`, `resolume/mapping_refresh*.rs`, `resolume/provisional_*.rs`, `resolume/keepalive_tests.rs`, `handlers.rs`, `bible_clear.rs`, `port_drift.rs`, the mocks + settings card) |
| NDI `ndi_frames_live` per-session Cell vs shared-signal desync (#757/#732/#500): never write the shared signal `false` without resetting the `FrameStats` Cell; same-source reactivation must not reset the gate; ticker self-heal + pure/clock-wired host-test split | `.claude/rules/ndi-frames-live-desync.md` (auto-loads on `pages/stage.rs`, `pages/stage_events.rs`, `components/stage/ndi_frame_stats.rs`, `ndi_health_ticker.rs`, `ndi_video.rs`) |
| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it); listening mics card (#852 C1b-6) = `MicForm` over the whole-list `GET|PUT /listen/mics` (read-change-PUT per mic, race accepted for ≤ 4 mics, server 422 inline, ids from the role, quiet `GET /listen/mics`; C1b-7: 2 s `/listen/status` poll only while on screen (a hidden panel forgets the status), stale after 2 failures, per-day recorder switch bound to a `Memo<bool>`, locked without a mic, indeterminate while unknown) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
| Stage displays drop-out (#793): launcher adb reads tri-state (never install/uninstall on an adb error), never churn a `device` target (disconnect only after 3 stuck cycles), stage WS applies EVERY event via a handler (no coalescing last-event signal), snapshot-driven layout reconcile + reconnect resync, `routeWebSocket` lost-frame E2E, WS end-reason / real peer IP / layout-switch logs, per-display layout adopted from heartbeat ACK + diag frames (#797) | `.claude/rules/stage-live-sync.md` (auto-loads on `ws/stage.rs`, `pages/stage*.rs`, `pages/camera.rs`, `live.rs`, `stage_connections.rs`, `android_stage*`) |
| AI status on `/healthz` + `/ai/status` (#760): ONE shared `evaluate_ai_status`; backend-agnostic `{connected,error,model}` (no OAuth fields, OpenRouter-ready); `/healthz` served through a per-AppState SWR cache (30s TTL, background refresh, single in-flight) so operator-tab fan-out never storms the backend — `/ai/status` stays live; render code in `router/ai_health.rs` + cache in `ai/health_cache.rs`, not the over-cap `router/ai.rs`; deploy gate `connected:false`→non-blocking `::error::`, `modelValid:false`→hard-fail | `.claude/rules/ai-health-endpoint.md` (auto-loads on `router/ai.rs`, `router/ai_health.rs`, `router.rs`) |
| AI agent loop + bible validator (#784): composer emits the reference the validator sees (`format_verse_range` comma-lists) so `normalize_reference` must accept every composer output; `run_agent_with_guard` seam for race-free budget tests; `##`-marker trick for a deterministic rejection; GaveUp is not an outage (no `AiCallHealth` write); chip fold/probe cross-crate prefix `posledné AI volanie zlyhalo` | `.claude/rules/ai-agent-loop.md` (auto-loads on `ai/agent*.rs`, `ai/bible_validator.rs`, `ai/tools/bible_presentation.rs`, `components/ai_status.rs`) |
| API stage layouts (#799): `api` + `api-ambient` (fullscreen NDI/CG + lyric overlay only while text), optional `currentTranslation`/`nextTranslation`, persisted text mode original/translation/both carried in the api snapshot (`textMode`) + `LiveEvent::StageTextMode`; ONE `is_api_stage_layout` predicate; layout+mode stamped under the layout read lock | `.claude/rules/api-stage-layouts.md` (auto-loads on `state/api_stage.rs`, `state/stage_text_mode.rs`, `router/api_stage.rs`, `components/stage/api_*.rs`) |
| Deploy workflow SSH alias safety (#795): shared runner `~/.ssh/config` — write `Host deploy-target` BEFORE ssh-keyscan (`id: ssh_setup`), always()/failure() remote steps gated on `steps.ssh_setup.outcome == 'success'`; enforced by `scripts/ci/check_deploy_workflows.py` in the quality job | `.claude/rules/deploy-workflow-ssh.md` (auto-loads on the 4 deploy workflows) |
| Bible slide composer (#828, #824): split by the LONGER of main/translation in `chars()` (never `.len()` bytes), lone oversized verse stays whole (`SlideDraft`/`BibleSlideFrame`); per-slide char-count badges share the same rule; the secondary reference names the SECONDARY book on the live AND the legacy `/bible/trigger` path (`BibleUpdate.secondary_book`, `legacy_reference.rs`), legacy lookups by canonical code | `.claude/rules/bible-slide-composer.md` (auto-loads on `slides/compose.rs`, `state/bible.rs`, `resolume/handlers.rs`, `legacy_reference.rs`, `pages/bible_slides.rs`) |
| Operator slide grids (#832): per-browser slides per row 1–8 via the inherited `--operator-slide-columns-choice` on `body` (phone query sets only the default), non-throwing `state/session.rs` (JSON strings kept), stepper follows the phone default, dense cards at 6+, nothing may assume 3 columns (`slide_list_scroll` reads the computed tracks); Edit mode is never moved by stage changes (#853): stage auto-scroll Live-only, AbleSet Follow never switches songs in Edit (`components/stage_follow.rs`, catches up on the switch back to Live, an explicit Follow ON click still syncs; an open waits in `follow_pending` and moves the selection only when its song is shown — never before; songless API-layout frames are not stage changes and the catch-up paths read `follow_stage`), no late answer replaces a newer song and an unshowable one never stays selected (`song_load::show_when_loaded` + `OnFailure`, `is_still_selected`, structural edits via `apply_structural_slides` / `is_shown`), a newly shown song opens at the top unless a Follow open showed it (`follow_landing` → the stage slide), card classes derived from signals (never DOM-poked), song list reads the stage song via a `Memo` | `.claude/rules/operator-slide-grid.md` (auto-loads on `components/slide_columns.rs`, `slide_list.rs`, `slide_list_scroll.rs`, `slide_save.rs`, `slide_reorder.rs`, `slide_selection.rs`, `presentation_list.rs`, `search.rs`, `stage_follow.rs`, `stage_preview.rs`, `pages/operator.rs`, `state/mod.rs`, `state/song_load.rs`, `state/slide_columns.rs`, `state/session.rs`, `styles/operator.css`, the two specs) |
| Remote Bible translation NLT (#826): `eng-nlt` fetched on demand from Tyndale's NLT API (`api.nlt.to`), never stored/bulk-copied/committed; ONE dispatch in `state/bible_source.rs` (never call the repository's Bible reads elsewhere); books/chapters + book names from `eng-kjv`, listed last and only with eng-kjv; ≤50-verse chunks, the API clamps out-of-range requests (filter!), `1Thes`/`1Jn` abbreviations; `RemoteBibleError` → 503/502 "NLT nedostupné" for NLT as main, while an unavailable NLT SECONDARY loads main-only + a `warning` toast (30 s fail-fast after the API was unreachable); `NLT_LONGER_CHAPTERS` extends the KJV structure (3 John 1, Revelation 12) | `.claude/rules/bible-remote-translations.md` (auto-loads on `bible_remote/**`, `state/bible_source*`, `state/bible.rs`) |
| AI backend config (#761, bundled proxy removed #762): OpenRouter via env; `PRESENTER_AI_*` WIN over the DB `ai-settings` row on the EFFECTIVE path only (`resolve_effective_settings` + `router/ai_env.rs`), never the raw persist/display path; effective `apiUrl` = env → DB → default `https://openrouter.ai/api/v1` (`DEFAULT_AI_API_URL`); deploy writes 0600 `/etc/presenter/ai.env` (`EnvironmentFile=-`) from `OPENROUTER_API_KEY`+`AI_MODEL` in all three workflows; `DEFAULT_AI_MODEL`=OpenRouter slug; attribution headers in `ai/client.rs` | `.claude/rules/ai-backend-config.md` (auto-loads on `router/ai.rs`, `router/ai_env.rs`, `ai/mod.rs`, `ai/client.rs`, the 3 deploy workflows, the 2 units) |
| Šepkár tips (#852 S1a/S1b): site-local `interpreter_tips` (no sync), rules + limits once in core `interpreter_tips.rs`, state-layer `TipRefusal` → 422 / 429 + Retry-After, duplicate = pair + its note (or the note alone) within 2 min not dismissed, create = one at a time (`create_lock`), full-list `LiveEvent::InterpreterTips` after every write + the minute boundary task (publishes serialized), injected `TipClock`, retention by `service_date` | `.claude/rules/interpreter-tips.md` (auto-loads on core `interpreter_tips.rs`, the migration, the entity, repository/state/router `interpreter_tips*`, the two pages, `components/interpreter/**`, `ws/passive.rs`, `state/interpreter.rs`, the two stylesheets, the spec) |
| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date, the first pass 10 min after the start (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited; a day after today is cleared at startup AND by the running supervisor), one writer per mic: lazy file, ≤ 50-frame batches in `spawn_blocking`, header patched per batch, `create_new` only, lag counted + gated WARN, a lag ends a batch; a new file when the audio is > 1 s from the wall clock either way (never on the batch gap alone), frames after midnight dropped, restart back-off 1…60 min counted from the failure | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
| Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, quiet poll/asset paths = no span, 5xx = WARN, client strings quoted, client IP = the #793 rule, `sqlx=warn` in every default `RUST_LOG`, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines; deploy.yml/release.yml fail when the running service's `RUST_LOG` is not the unit's), timer-polled GETs quiet (writes on the same paths stay logged), a silent WHEP poll (POST 204) has no response line, the request span prefixes every handler line, `src/tracing_capture.rs` = the ONE log-test helper | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `tracing_capture.rs`, `router/integrations/ndi_whep.rs`, `router.rs`, `state/bible_show_log.rs`, `main.rs`, the units, `deploy.yml`, `release.yml`) |
| Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
| Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr); A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
| Bible import (#851 D1): MySword `<FI>[…]<Fi>` notes go with the space before them, bare `[…]` stays (translation text), `<TS>` headings go, no space before `. , ; : ! ?`; USFM `\f`/`\x` go with their text, leading `¶` goes after the markers, `\s1`/`\d`/`\ms` lines never join a verse; every parse change bumps `PARSER_VERSION`; synthetic archives + the bundled-KJV corpus test; T1 post-deploy SQL; only the public-domain KJV is in `data/bibles/` — SEB/ROH/SEVP/MIL come from the private repo `zbynekdrlik/presenter-bibles`, checked out into the gitignored `data/bibles-private/` (deploy key `BIBLES_DEPLOY_KEY`, never in the public release tarball), guarded by `scripts/ci/check_public_bibles.py` (#859) | `.claude/rules/bible-import.md` (auto-loads on `presenter-bible/src/**`, `bible_digest.rs`, `ingest_bibles.rs`, `repository/bible/import.rs`, `data/bibles/**`, `check_public_bibles.py`) |
| Sermon preparation (#851 P/E): core = deletions + emphasis on token indices, `render_item` is the only truth for wall text, labels mark only run edges (`16b-18a`, `5-7, 9`), one `translation_short_code` (later units add persistence / generation / API) | `.claude/rules/sermon-prep.md` (auto-loads on `bible/sermon/**` and the later sermon files) |
| Operator on a phone (#850 B1): ONE phone-shell query → `body[data-shell]` + `AppContext::phone_shell`; bottom tab bar (left rail in landscape) + shared `view_nav` (Slovak labels), burger gone; 16 px fields under a coarse pointer (`!important`); touch slides trigger on click unless moved > 10 px; no stage-preview iframe on phones | `.claude/rules/phone-shell.md` (auto-loads on the shell files, `header.rs`, `slide_list.rs`, `phone_shell.css`, the two specs) |
| Verse console (#850 B2): the reference resolved in the browser against the MAIN translation's `/bible/books`; pure half in `state/bible_console*.rs` (host tests over the core's structure fixtures); row keys carry label + detail; `Picked` valid for its input + main translation only; preview answers applied only while their key is current; settle markers on `[data-role=bible-console]`; on phones the console replaces `.bible__classic` | `.claude/rules/bible-console.md` (auto-loads on `pages/bible_console/**`, `state/bible_console*.rs`, `bible_console.css`, the spec) |

## Always-Rules

- **Box dimensions** — NEVER change stage layout box sizes or positions without explicit user instruction. Explain the constraint and ask what dimensions they want.
- **Tablet triggers** — Tablet sends only `(presentation_id, slide_id)` to server; server looks up and broadcasts full data. NEVER reconstruct slide data in the tablet. Fixes go in WASM Rust (`tablet.rs`), not CSS.
- **Hardware features** — If a feature depends on external hardware/SDK (NDI, cameras, audio), ask the user about availability BEFORE implementing. Write tests for the real pipeline, not just the fallback path. Never report hardware features as done based only on CI green.
- **External API integration** — Always WebSearch + WebFetch official docs BEFORE writing integration code. Never guess API behavior. (AbleSet `/api/setlist` returns the full session; `internalMeta.skipped: true` marks songs not in the active set.)
- **Drag-drop** — A single middle-position E2E test is insufficient. Before claiming complete, verify in Playwright: empty list, drop above first entry, drop below last entry, middle-position drop.

---

## Project Overview

Presenter is a monolithic Rust application for church worship services, providing lyrics display, Bible passages, timers, and stage displays.

**Key Documentation:**

- Domain specifics: `docs/functional-needs.md`
- System architecture: `docs/architecture.md`
- Configuration reference: `docs/configuration.md`

---

## Versioning

The project uses **Semantic Versioning** with a single `X.Y.Z` format on both branches.

**Version location:** `Cargo.toml` workspace `[workspace.package].version` (always clean semver `X.Y.Z`)

**Build channel:** Dev vs production is distinguished via a compile-time `PRESENTER_BUILD_CHANNEL` env var (defaults to `"dev"`). Deploy workflows set this to `"dev"` or `"release"`.

**Version display:**

- `/healthz` returns `{"status":"ok","version":"0.1.2","channel":"dev"}`
- UI footer shows `v0.1.2 (dev)` or `v0.1.2` (release)

**CI enforcement:** `version-check.yml` validates version is valid semver and greater than the latest GitHub release.

**Release lifecycle:**

1. Dev work: version is `X.Y.Z`, push to dev, CI passes, dev deploy shows `vX.Y.Z (dev)`
2. Release: create PR dev->main, no version change needed, all checks green
3. Merge: Claude merges autonomously once every gate is green (user directive 2026-07-24), main deploys, production shows `vX.Y.Z`
4. Post-release: bump to next version on dev, continue

See `docs/architecture.md` for full versioning and release strategy.

---

## GitHub Actions (Primary CI/CD)

### Runner Architecture

This project uses a **mixed runner strategy**:

- **GitHub-hosted runners** (`ubuntu-latest`): All compilation, linting, testing, and security scans. Free for public repos.
- **One bare-metal local runner** (`self-hosted`): E2E tests (need running binary) and deployments (SSH to LAN hosts).

**Local runner host:** `10.77.8.134` (same machine as dev server)
**Local runner name:** `presenter-local`
**Local runner label:** `self-hosted`
**Config location:** `~/actions-runner/`

The local runner does NOT compile Rust — it only runs pre-built artifacts downloaded from GitHub-hosted build jobs. It needs: Node.js 22, Playwright chromium, rsync, and SSH access to LAN deploy targets.

**Deploy workflows** use SSH from the local runner to application hosts (`10.77.9.205` for production, `10.77.8.134` for dev, `companion-pp.lan` for PP releases).

Runner management commands (status, restart, re-register) live in the ci skill — see the Playbook Router.

### Workflows

| Workflow                | Trigger                    | Purpose                                                   |
| ----------------------- | -------------------------- | --------------------------------------------------------- |
| `pipeline.yml`          | Push to `dev`              | Full pipeline: checks → build → E2E → deploy-dev          |
| `deploy.yml`            | Push to `main`             | Build on GH runner, deploy to production via SSH          |
| `release.yml`           | GitHub Release published   | Build release, upload tarball, deploy to companion-pp.lan |
| `security-schedule.yml` | Weekly (Sunday) + manual   | Scheduled vulnerability scanning                          |
| `import-data.yml`       | Manual (workflow_dispatch) | Re-import ProPresenter/Bible data                         |
| `mutation-full.yml`     | Manual (workflow_dispatch) | On-demand full-tree mutation sweep (`/mutation-sweep`)    |
| `ndi-latency.yml`       | Manual (workflow_dispatch) | On-demand NDI glass-to-glass latency guard on a quiet box (#386) |
| `pr-labeler.yml`        | PR opened/edited           | Auto-label PRs by changed paths                           |

**Pipeline dependency chain:** `branch-sync → [fmt, clippy, companion, version-check, security] → [test, quality] → coverage → build → e2e → deploy-dev`

Deploy-dev **cannot run** unless every check, test, build, and E2E job succeeds.

**NDI E2E split (load-sensitivity, #386):** The per-PR `e2e-ndi` lane runs the load-INSENSITIVE
NDI guards (decode / freeze / console / straggler / reactivate / reload in
`ndi-webrtc-synthetic.spec.ts`) — PRs still fail if NDI video is actually broken. The
load-SENSITIVE glass-to-glass latency assertion (`ndi-latency.spec.ts`, tag `@latency-ndi`) is
NOT in the per-PR pipeline: on the shared dev2 runner, concurrent CPU load (another project's
cargo-mutants / rebuilds) starves the in-browser sampling loop + GPU encoder and false-reds the
timing bound. Its bounds are unchanged — it runs on-demand on a quiet box via `ndi-latency.yml`
(same pattern as the #488 mutation full-sweep). Run it after NDI/WebRTC pipeline changes.

---

## Build & Run

```bash
# Development server
cargo run -p presenter-server

# With environment variables
PRESENTER_PORT=8080 cargo run -p presenter-server

# Release build (what CI produces)
cargo build --release -p presenter-server
./target/release/presenter-server
```

### Deployed Instances

Three instances run on separate hosts:

| Instance   | URL                     | Host             | Port | Service                 | Deploy Dir           | Trigger        |
| ---------- | ----------------------- | ---------------- | ---- | ----------------------- | -------------------- | -------------- |
| Production | http://10.77.9.205      | 10.77.9.205      | 80   | `presenter.service`     | `/opt/presenter`     | push to `main` |
| Dev        | http://10.77.8.134:8080 | 10.77.8.134      | 8080 | `presenter-dev.service` | `/opt/presenter-dev` | push to `dev`  |
| PP         | http://companion-pp.lan | companion-pp.lan | 80   | `presenter.service`     | `/opt/presenter`     | GitHub Release |

```bash
# Check both services
systemctl status presenter presenter-dev

# View dev logs
sudo journalctl -u presenter-dev -f

# Restart dev only
sudo systemctl restart presenter-dev
```

---

## Banned Patterns (Production Code)

| Pattern              | Why Banned                      | Alternative                                   |
| -------------------- | ------------------------------- | --------------------------------------------- |
| `unwrap()`           | Panics in production            | Use `?`, `ok_or()`, or handle `None`/`Err`    |
| `expect()`           | Panics in production            | Use `?` with context via `anyhow`/`thiserror` |
| `panic!`             | Crashes the service             | Return `Result` or `Option`                   |
| `std::thread::sleep` | Blocks async runtime            | Use `tokio::time::sleep`                      |

**Note:** Test code (`#[cfg(test)]` modules) and WASM code (`presenter-ui` crate) are exempt from panic rules. WASM panics become browser-side JavaScript errors rather than server crashes.

---

## File/Function Limits (Enforced by CI)

| Metric         | Warning | Hard Fail | Exempt                                  |
| -------------- | ------- | --------- | --------------------------------------- |
| File lines     | >800    | >1000     | Migrations, tests                       |
| Function lines | >80     | >120      | Migrations, UI renders, router builders |

**Exempt patterns:**

- `m*_create_*.rs` - Migration files (declarative schema definitions)
- `render_*_ui` functions - Leptos component renders (HTML-like DSL)
- `build_router` functions - Route declarations

---

## Testing

### Test Commands

```bash
# Rust unit tests
cargo test

# Single test
cargo test test_name

# Playwright E2E (MUST pass before any merge)
npm run test:playwright
npm run test:playwright:headed  # Browser visible

# View Playwright report
scripts/dev/show-playwright-report.sh
```

### E2E Notes

- Tests must be deterministic: fixed seeds, stable timeouts
- Prefer retry-with-assert poll helpers over arbitrary sleeps
- **E2E timeout = build failure** - Optimize build caching, not extend timeouts

---

## Architecture

### Project Structure

```
data/
├── libraries/             # ProPresenter libraries (single source of truth)
└── bibles/                # Bible translation files
crates/
├── presenter-core/        # Domain logic (no server deps)
├── presenter-server/      # Axum HTTP/WS + Leptos SSR
├── presenter-persistence/ # SeaORM repository layer
├── presenter-migration/   # Schema evolution
├── presenter-importer/    # ProPresenter import
└── presenter-bible/       # Bible translation ingestion
```

### Key HTTP Endpoints

| Endpoint        | Purpose                      |
| --------------- | ---------------------------- |
| `/healthz`      | Readiness probe              |
| `/ui/operator`  | Desktop control surface      |
| `/ui/tablet`    | Touch-optimized controller   |
| `/ui/bible`     | Bible search/trigger UI      |
| `/stage`        | HTML stage display           |
| `/live/ws`      | Live updates (timers, stage) |
| `/companion/ws` | Bitfocus Companion control   |

---

## Environment Variables

| Variable                      | Default                 | Purpose                 |
| ----------------------------- | ----------------------- | ----------------------- |
| `PRESENTER_PORT`              | 80                      | Server port                                                          |
| `PRESENTER_DB_URL`            | `sqlite://presenter.db` | Database connection                                                  |
| `PRESENTER_COMPANION_ENABLED` | 0                       | Enable Companion socket                                              |
| `PRESENTER_COMPANION_PORT`    | 18175                   | Companion listen port                                                |
| `PRESENTER_LOCAL_PUBLIC_IP`   | unset                   | Church's public egress IP for LAN/WAN detection via Cloudflare Tunnel |
| `PRESENTER_ANDROID_STAGE_URL` | unset                   | Stage URL the launcher opens on every Android stage display via `am start -a VIEW -d <url> <package>`. Set per env in deploy units (prod `http://10.77.9.205/stage`, dev `http://10.77.8.134:8080/stage`). Unset → launcher warns + skips. |
| `PRESENTER_STARTUP_MODE`      | `normal`                | `normal` = full boot. `validate` (#771) = deploy schema-validation probe: migrate + open DB + serve `/healthz`, but start NO integration/background task (NDI restore, Companion, OSC, AbleSet, Resolume, sync, Android launcher, tickers) and no AI probe. Used by the "Validate database schema" step in deploy.yml/pipeline.yml/release.yml so the probe never collides with the live service. Unknown value → `normal` (WARN). |
| `PRESENTER_NLT_API_KEY`       | `TEST`                  | Tyndale NLT API key for the remote `eng-nlt` translation (#826). `TEST` = anonymous: ≤50 verses/request, ≤500 requests/day; a registered key ≤500 verses, ≤5000 requests. |
| `PRESENTER_NLT_API_URL`       | `https://api.nlt.to`    | NLT API base URL (tests point it at a mock; the E2E default is a dead loopback). |
| `PRESENTER_DRIVE_URL`         | `https://drive.google.com` | Google Drive folder-listing base URL for the stream playback mirror (#843; tests/E2E point it at a mock, the E2E default is a dead loopback). |
| `PRESENTER_DRIVE_DOWNLOAD_URL` | `https://drive.usercontent.google.com` | Google Drive download base URL for the playback mirror (#843; same test treatment). |
| `PRESENTER_PLAYBACK_MAX_GB`   | `20`                    | Byte budget of the whole playback mirror in GiB (#843); a download that would pass it is refused with a Slovak status error. |
| `PRESENTER_LISTEN_VBAN_ADDR`  | `0.0.0.0:6980`          | Bind address of the VBAN receive socket for the listening microphones (#852 C1a); bound only while `listen.mics` is non-empty (it ships empty). The E2E harness uses `127.0.0.1:0`. |
| `PRESENTER_LISTEN_RECORDINGS_DIR` | `recordings` next to the sqlite file | Folder of the listening recordings (#852 C1b): `<dir>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav`, a day folder is kept 30 days and deleted on the 31st. Prod / PP `/opt/presenter/recordings`, dev `/opt/presenter-dev/recordings`. |
| `RUST_LOG`                    | `info,tower_http=debug,sqlx=warn` | Tracing filter                                                       |

---

## Code Quality Standards

### Format & Lint (CI enforces)

Pre-push quality gate: `scripts/dev/quality-check.sh --strict --against origin/main` (what CI runs) — details in the ci skill.

---

## Documentation Standards

### ADR Process

1. Copy `docs/adr/template.md` to `docs/adr/NNNN-<slug>.md`
2. Fill all sections; set status to "Proposed"
3. Link from PR description
4. Change status to "Accepted" when implemented

---

## Design Philosophy

- **Reliability over breadth**: Offline-ready, sub-100ms latency
- **Church-specific**: Solve exact requirements for our workflows
- **Greenfield redesigns**: Treat redesigns as fresh starts
- **CI-first**: GitHub Actions validates everything, local testing confirms

---

## User Preferences

### LAN IP (NOT localhost)

The user accesses the server from another machine on the same network. **Always provide LAN IP URLs, never localhost.** (IPs are in the Deployed Instances table above.)

**Never use localhost** — always use the LAN IP.
