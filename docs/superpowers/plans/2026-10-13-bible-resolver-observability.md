# Bible Resolver + Observability (#850 O1, A1a, A1b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first three PR units of #850: O1 (HTTP request log, stale systemd drop-ins removed, one `presenter::bible::show` INFO line per show, a reproducible Cloudflare Access reconcile script), A1a (the committed book-name sources, their Python generator with a CI `--check`, the generated `names_table.rs`, the Rust fold, `Ab` ≠ Obadiah) and A1b (the reference resolver in `presenter-core`: grammar, ranking, versification, hints, public API, 100/100 corpus).

**Architecture:** O1 is server-only: a `TraceLayer` in `build_router` (new `router/request_log.rs`), a `BibleShowOrigin` threaded from the two trigger endpoints into `AppState::trigger_bible_slide_output` (new `state/bible_show_log.rs`), two `rm -f` lines in the SNV/PP deploy steps, and `ops/cloudflare/presenter_access.py` (pure `desired_apps`/`plan` + a thin HTTP shell). A1/A1b live entirely in the new `crates/presenter-core/src/bible/resolver/` module, compiled into the server and the WASM: `parse` (text → ranked book candidates + a raw chapter/verse part) and `resolve` (bind to the MAIN translation's `Versification`) — two phases, the structure filter last. Nothing outside the module calls the resolver yet (A2a/B2a do); A1b only moves `counted()` out of `presenter-ui`.

**Tech Stack:** Rust 2021 (msrv 1.85), axum 0.8, tower-http 0.6.8 (`trace`), tracing, `unicode-normalization`, serde/serde_json (tests), Python 3.12 + PyYAML (generator, CI `quality` job installs `python3-yaml`), stdlib-only Python (Access script), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-13-bible-verses-any-device-design.md` (commit 5f8ebb85), §1–§11; design inputs in the session scratchpad (`850-resolver-input.md`, `850-book-names.json`, `850-resolver-corpus.json`, `resolver-scripts/`, `cf_presenter_access.py`).

## Global Constraints

- File cap ≤ 1000 production lines (warn > 800) via `bash scripts/dev/count_prod_lines.sh <file>`; function cap ≤ 120 lines (warn > 80) via `QC_TARGETS="<files>" python3 scripts/dev/fn_length_check.py .` — generic fns are invisible to the checker, size them by hand.
- No `unwrap()` / `expect()` / `panic!` in server or core production code (tests and the WASM `presenter-ui` crate are exempt). Enforced in CI since 2026-10-14 by `scripts/dev/no_panic_check.py` (quality-check section 10; the old rg pattern never parsed).
- Tier-0: NO local compiling cargo (`build`/`check`/`clippy`/`test`/`run`); allowed locally: `cargo fmt --all -- --check`, `(cd crates/presenter-ui && cargo fmt --check)`, `cargo metadata --offline`, `rustfmt`, Python. Every Rust test below is verified by CI after the unit's push.
- Public repo: no e-mails, egress IPs or Cloudflare account ids committed — tests use RFC 5737 addresses (`192.0.2.x`, `203.0.113.x`); the real Access config lives in `~/.config/presenter/presenter-access.json` (0600), never in the repo.
- Version bump = first commit of a PR unit whenever dev ≤ main (`[workspace.package].version` in `Cargo.toml` + `Cargo.lock`, computed by the Task 6 recipe): on 2026-10-13 dev 0.4.316 > main 0.4.315, so O1 needs none and A1a / A1b expect 0.4.317 / 0.4.318 — if another release lands first, the recipe picks the next patch and every `vX.Y.Z` below follows it.
- This checkout is shared with an active lane (#853 committed d7b24d12..2f8b2315 and had uncommitted presenter-ui / `CLAUDE.md` edits at 15:00): never stage, stash or revert files you did not change; edit only your own `CLAUDE.md` rows; a dev→main release PR carries everything on dev, so open it only when dev is quiet (the supervisor's integration call, `.claude/rules/quality-gates.md` "Shared dev under a multi-worker fleet").
- These are features: bug-fix RED-before-GREEN commits do not apply, but every unit ships its tests in the same PR.
- Commits: `git commit -F <file>`, conventional subject `type(scope): … (#850 <unit>)`, `[no-test: <reason>]` on commits without tests, message ends with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- Never rewrite history (no amend / `reset --hard` / force push); stage explicit paths only — the untracked `tests/e2e/operator-stage-churn-scroll.spec.ts` belongs to another lane, never `git add -A`.
- Two-branch flow: commit on `dev`, PR dev→main, merge commit (no squash/rebase), merge yourself once every gate is green (CLAUDE.md "Release lifecycle"); PP gets the release via `gh release create vX.Y.Z --target main`.
- CI clippy = `-D warnings -W clippy::all`, msrv 1.85: inline format args in presenter-core (`format!("{n} {noun}")`), no std API newer than 1.85, `///` list continuations indented, no identical `if` branches.
- presenter-core consumers import `presenter_core::bible::resolver::…` (spec §5.1 — no root `pub use`); a `pub(crate)` item without a caller fails `dead_code` at the unit's head (intermediate A1b commits may warn until Task 17 wires the API; only the pushed head is compiled).
- `crates/presenter-ui` is workspace-excluded: format it with `(cd crates/presenter-ui && cargo fmt --check)`.
- Before a push: load `ci-push-discipline`; after it: monitor the run to a terminal state per `ci-monitoring` (never `gh run watch`); after a deploy: `post-deploy-verification` (liveness, version from the live DOM, functional).

Shell variables used below:

```bash
REPO=/home/newlevel/devel/presenter/presenter-dev2
SCRATCH=/tmp/claude-1000/-home-newlevel-devel-presenter-presenter-dev2/d306e5ce-ff0c-4d69-8407-7cf7c1af755a/scratchpad
```

Commit recipe (every "Commit" step): write the message to `$SCRATCH/msg.txt` with the Write tool (subject, blank line, body, blank line, the `Co-Authored-By:` line), then `git -C "$REPO" add <the listed paths> && git -C "$REPO" commit -F "$SCRATCH/msg.txt"`.

## Review Focus

The prototype (`resolver-scripts/verify_corpus.py`) passed 100/100 but never checked alternatives order, hints, end chapters or later list items. A Python mirror of THIS plan's rules (`$SCRATCH/plan-probe/mirror.py`) passes the corpus under the stricter Rust checks; the five input classes most likely to bite anyway, each pinned by a test in its owning task:

1. **Typing in progress and garbage after the reference** (`zid 11:`, `zid 11:1-`, `zid 11:1 až`, `zid 11:1 viera`, `zid 11,1:3`, `zid 99999999`, a 10,000-char paste). The prototype returned "unparsed" for all of them; the console resolves on every keystroke. Tests: `trailing_separators_are_ignored_and_unused_text_is_leftover`, `a_long_paste_is_bounded` (Task 14).
2. **Every segment validated, not only the last verse** (`mt 5:3.99.7`, `gn 50:1-51:2`, `ž 150-151`, `zid 11:39-45`). The prototype checked only the final verse and never an end chapter. Tests: `every_segment_is_checked_not_only_the_last`, `out_of_range_offers_the_last_chapter_or_verse_as_fix` (Task 16).
3. **Single-chapter books meeting chapter syntax** (`jud 1-5`, `jud 30`, `jud 2:5` → Judges 2:5 because Jude has no chapter 2). The prototype ignored chapter ranges. Tests: `single_chapter_books_read_a_lone_number_as_a_verse` (Task 16), `the_structure_filter_picks_the_book_that_has_the_verse` (Task 17).
4. **The main translation is not what the table assumes** (KJV vs SEB Malachi, SEVP's stored names "Evanjelium podľa Jána", a partial main translation such as slk-mil without Hebrews, a future translation's own names). Tests: `versification_differs_per_translation` (Task 16), `main_translation_decides_the_versification_and_the_names`, `a_book_missing_from_a_partial_main_translation_says_so`, `stored_names_of_the_main_translation_match_at_runtime` (Task 17).
5. **Python fold ≠ Rust fold** (NFD vs NFC input, U+2010–U+2015 dashes, glued `Prvýlist`, the `k.` filler, roman `I.` vs the book `Iz`). The keys come from Python, the typed text is folded in Rust. Tests: `test_shared_fold_cases` (Python, Task 9) and `fold_matches_the_cases_shared_with_the_generator` + `every_generated_row_folds_back_to_its_key` (Rust, Task 10) over the same `testdata/fold-cases.json`.

---

## PR unit O1 — observability + Access script (v0.4.316, no bump)

### Task 1: HTTP request log layer

**Files:**
- Create: `crates/presenter-server/src/router/request_log.rs`
- Modify: `crates/presenter-server/src/router.rs:11-12` (module list: `mod request_log;` between `mod presentations;` and `mod search;`)
- Modify: `crates/presenter-server/src/router.rs:441-442` (inside `build_router` 41-443: `.layer(…)` between `.merge(stream_playback::routes())` and `.with_state(state)`)

**Interfaces:**
- Consumes: `fn extract_client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String` (`router/integrations/mod.rs:56-63`, `pub(super)` = visible in `router` and below); `axum::extract::ConnectInfo<SocketAddr>` (inserted by `into_make_service_with_connect_info`, `main.rs:130`); tower-http 0.6.8 `TraceLayer`, `DefaultOnResponse`, `LatencyUnit` (feature `trace`, `Cargo.toml:41`).
- Produces: `pub(super) type RequestTraceLayer = TraceLayer<SharedClassifier<ServerErrorsAsFailures>, fn(&Request<Body>) -> Span, ()>;`, `pub(super) fn request_trace_layer() -> RequestTraceLayer`, `pub(super) fn request_fields<B>(request: &Request<B>) -> RequestFields`, `pub(super) struct RequestFields { pub(super) method: String, pub(super) uri: String, pub(super) user_agent: String, pub(super) client_ip: String }`.

- [ ] **Step 1: Write the failing tests** — the `#[cfg(test)]` block that ends `router/request_log.rs` (it calls `request_fields`, which Step 3 creates):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str, peer: Option<&str>, forwarded: Option<&str>) -> Request<()> {
        let mut builder = Request::builder()
            .uri(uri)
            .header(USER_AGENT, "Mozilla/5.0 (iPhone)");
        if let Some(value) = forwarded {
            builder = builder.header("x-forwarded-for", value);
        }
        let mut request = builder.body(()).expect("request");
        if let Some(addr) = peer {
            let addr: SocketAddr = addr.parse().expect("socket address");
            request.extensions_mut().insert(ConnectInfo(addr));
        }
        request
    }

    #[test]
    fn fields_keep_the_query_and_the_user_agent() {
        let request = request(
            "/bible/books?translation=slk-seb",
            Some("192.0.2.7:51000"),
            None,
        );
        assert_eq!(
            request_fields(&request),
            RequestFields {
                method: "GET".to_string(),
                uri: "/bible/books?translation=slk-seb".to_string(),
                user_agent: "Mozilla/5.0 (iPhone)".to_string(),
                client_ip: "192.0.2.7".to_string(),
            }
        );
    }

    #[test]
    fn forwarded_for_counts_only_from_a_loopback_peer() {
        let tunnel = request("/healthz", Some("127.0.0.1:40000"), Some("203.0.113.9"));
        assert_eq!(request_fields(&tunnel).client_ip, "203.0.113.9");
        let lan = request("/healthz", Some("192.0.2.7:40000"), Some("203.0.113.9"));
        assert_eq!(request_fields(&lan).client_ip, "192.0.2.7");
    }

    #[test]
    fn a_bare_request_logs_dashes_and_anonymous() {
        let bare = Request::builder().uri("/").body(()).expect("request");
        let fields = request_fields(&bare);
        assert_eq!(fields.user_agent, "-");
        assert_eq!(fields.client_ip, "anonymous");
    }
}
```

- [ ] **Step 2: How it is verified.** Rust tests run in CI only (Tier-0); the RED state is not compiled locally. Every router test in `router/tests.rs` (e.g. `bible_passage_endpoint_returns_reference`, 1136) now also runs through the new layer, which proves it composes with `Router<AppState>`.

- [ ] **Step 3: Implement** — the top of `router/request_log.rs` (above the test block):

```rust
//! HTTP request log (#850 spec §8): a DEBUG span per request (method, path +
//! query, user agent, client IP) and a DEBUG line per response (status,
//! latency in ms). The units' `RUST_LOG=info,tower_http=debug` enables both —
//! the span uses tower-http's own `make_span` target so that filter covers it.
//! Client IP follows the live WS rule (#793): X-Forwarded-For counts only from
//! a loopback peer (cloudflared); a LAN peer is the client itself.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header::USER_AGENT, Request};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::{Level, Span};

/// The layer `build_router` installs: our span, no "started" line, one
/// DEBUG response line, tower-http's default ERROR line for a 5xx.
pub(super) type RequestTraceLayer =
    TraceLayer<SharedClassifier<ServerErrorsAsFailures>, fn(&Request<Body>) -> Span, ()>;

/// What the request span carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestFields {
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) user_agent: String,
    pub(super) client_ip: String,
}

pub(super) fn request_trace_layer() -> RequestTraceLayer {
    TraceLayer::new_for_http()
        .make_span_with(request_span as fn(&Request<Body>) -> Span)
        .on_request(())
        .on_response(
            DefaultOnResponse::new()
                .level(Level::DEBUG)
                .latency_unit(LatencyUnit::Millis),
        )
}

pub(super) fn request_fields<B>(request: &Request<B>) -> RequestFields {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let uri = request.uri();
    RequestFields {
        method: request.method().to_string(),
        uri: uri
            .path_and_query()
            .map_or_else(|| uri.path().to_string(), ToString::to_string),
        user_agent: request
            .headers()
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("-")
            .to_string(),
        client_ip: super::integrations::extract_client_ip(request.headers(), peer),
    }
}

fn request_span(request: &Request<Body>) -> Span {
    let fields = request_fields(request);
    tracing::span!(
        target: "tower_http::trace::make_span",
        Level::DEBUG,
        "request",
        method = %fields.method,
        uri = %fields.uri,
        user_agent = %fields.user_agent,
        client_ip = %fields.client_ip
    )
}
```

Then in `router.rs` add the module (after line 11):

```rust
mod presentations;
mod request_log;
mod search;
```

and the layer at the end of `build_router` (lines 441-442 become):

```rust
        .merge(stream_playback::routes())
        // #850 O1: one DEBUG request span + response line per request.
        .layer(request_log::request_trace_layer())
        .with_state(state)
```

The span's target is `tower_http::trace::make_span` on purpose: the units run `RUST_LOG=info,tower_http=debug` (`scripts/deploy/presenter.service:67`), so a span under `presenter_server::…` would be DEBUG under `info` = disabled and the `on_response` line would print without method/path. `on_request(())` drops the "started processing request" line; a 5xx keeps tower-http's default ERROR line.

- [ ] **Step 4: Local gates**

```bash
cd "$REPO"
cargo fmt --all -- --check
QC_TARGETS=$'crates/presenter-server/src/router/request_log.rs\ncrates/presenter-server/src/router.rs' python3 scripts/dev/fn_length_check.py .
bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/router.rs   # 763 (< 800)
bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/router/request_log.rs
```

Expected: fmt clean, `{"violations": [], "warnings": []}`.

- [ ] **Step 5: Commit** — paths `crates/presenter-server/src/router/request_log.rs crates/presenter-server/src/router.rs`, message:

```text
feat(server): log every HTTP request through a TraceLayer (#850 O1)

build_router installed no request logging, so the prod journal showed no
request at all (spec §4). One DEBUG span per request (method, path + query,
user agent, client IP by the #793 rule) and one DEBUG response line (status,
latency in ms). The span uses tower-http's make_span target so the units'
RUST_LOG=info,tower_http=debug enables it.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 2: One `presenter::bible::show` INFO line per show

**Files:**
- Create: `crates/presenter-server/src/state/bible_show_log.rs`
- Modify: `crates/presenter-server/src/state/mod.rs:37-38` (`pub(crate) mod bible_show_log;` between `mod bible_manager;` and `mod bible_source;`)
- Modify: `crates/presenter-server/src/state/bible.rs:3` (import) and `:629-633` (signature of `trigger_bible_slide_output`, 629-696) + its first body line
- Modify: `crates/presenter-server/src/router/bible/broadcast.rs:6-15` (imports), `:139-143` + `:161-163` (`trigger_bible_slide`, 139-168), `:171-174` + `:217-219` (`trigger_presentation_slide`, 171-224), new `show_origin` before `clear_bible_broadcast` (`:226`)
- Modify: `crates/presenter-server/src/state/tests.rs:1419-1424` (the only other caller of `trigger_bible_slide_output`)
- Modify: `crates/presenter-server/src/router/tests.rs` (append after line 3377)

**Interfaces:**
- Consumes: `presenter_core::BibleSlideOutput { main_reference, secondary_reference, … }` (`presenter-core/src/bible/translation.rs:170-185`); `extract_client_ip` (Task 1).
- Produces: `pub enum BibleShowSource { Api, Prepared }` + `pub const fn as_str(self) -> &'static str`; `pub struct BibleShowOrigin { pub source: BibleShowSource, pub device: Option<String>, pub client_ip: String }`; `pub(crate) struct BibleShowLine<'a>` + `pub(crate) fn new(origin: &'a BibleShowOrigin, output: &'a BibleSlideOutput) -> Self`; `pub(crate) fn log_bible_show(origin: &BibleShowOrigin, output: &BibleSlideOutput)`; changed `pub async fn trigger_bible_slide_output(&self, output: BibleSlideOutput, reference_metadata: BibleSlideReferenceMetadata, origin: BibleShowOrigin)`.
- `BibleShowOrigin`/`BibleShowSource` are declared `pub` inside the `pub(crate)` module, the same pattern as `BibleSlideReferenceMetadata` (`state/bible.rs:76-85`), which already appears in this `pub` AppState method's signature. Only the two variants that have a caller exist; B2a adds `Console`, A2b `Companion`/`Ai` together with their callers (an unconstructed variant fails `dead_code` because the module is crate-private).

- [ ] **Step 1: Write the failing tests.** The test block that ends `state/bible_show_log.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn output(secondary_reference: &str) -> BibleSlideOutput {
        BibleSlideOutput::new(
            "Viera je podstatou toho, v čo dúfame",
            "Hebrejom 11:1 (SEB)",
            "",
            secondary_reference,
            Utc::now(),
        )
    }

    #[test]
    fn the_line_carries_source_device_ip_and_both_references() {
        let origin = BibleShowOrigin {
            source: BibleShowSource::Prepared,
            device: Some("touch".to_string()),
            client_ip: "192.0.2.7".to_string(),
        };
        let output = output("Hebrews 11:1 (NLT)");
        assert_eq!(
            BibleShowLine::new(&origin, &output),
            BibleShowLine {
                source: "prepared",
                device: "touch",
                client_ip: "192.0.2.7",
                reference: "Hebrejom 11:1 (SEB)",
                secondary: "Hebrews 11:1 (NLT)",
            }
        );
    }

    #[test]
    fn a_missing_device_and_secondary_are_dashes() {
        let origin = BibleShowOrigin {
            source: BibleShowSource::Api,
            device: None,
            client_ip: "anonymous".to_string(),
        };
        let output = output("");
        let line = BibleShowLine::new(&origin, &output);
        assert_eq!(
            (line.source, line.device, line.secondary),
            ("api", "-", "-")
        );
    }
}
```

Append to `router/tests.rs` (after line 3377):

```rust

// #850 O1: the trigger handlers now extract the headers and the optional
// ConnectInfo for the `presenter::bible::show` line. A router served without
// connect-info (this test, `tower::ServiceExt::oneshot`) must still accept a show.
#[tokio::test]
async fn trigger_slide_without_connect_info_still_shows_the_verse() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bible/trigger-slide")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "203.0.113.9")
                .body(Body::from(
                    json!({
                        "mainText": "Viera je podstatou toho, v čo dúfame",
                        "mainReference": "Hebrejom 11:1 (SEB)"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let active = state.active_bible_slide_output().await.unwrap();
    assert_eq!(active.main_reference, "Hebrejom 11:1 (SEB)");
}
```

and update the existing caller in `state/tests.rs:1419-1424` to:

```rust
    state
        .trigger_bible_slide_output(
            output.clone(),
            super::bible::BibleSlideReferenceMetadata::default(),
            super::bible_show_log::BibleShowOrigin {
                source: super::bible_show_log::BibleShowSource::Api,
                device: None,
                client_ip: "anonymous".to_string(),
            },
        )
        .await;
```

- [ ] **Step 2: How it is verified.** CI `test` job. The router test proves the new `HeaderMap` + `Option<Extension<ConnectInfo<SocketAddr>>>` extractors do not reject a request served without connect-info (the pattern `router.rs:559` already uses). The journal line itself is checked on SNV in Task 5 (dev's `gst-debug.conf` drop-in filters it out, see Task 5).

- [ ] **Step 3: Implement.** The top of `state/bible_show_log.rs`:

```rust
//! One INFO line per Bible show (#850 spec §8): target
//! `presenter::bible::show` with the source endpoint, the device, the client
//! IP and the references that went to the wall.

use presenter_core::BibleSlideOutput;

/// The endpoint a Bible show came from. B2a adds the console, A2b Companion
/// and the AI tools when the legacy trigger is rerouted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BibleShowSource {
    /// `POST /bible/trigger-slide`.
    Api,
    /// `POST /bible/presentations/{id}/slides/{slide_id}/trigger`.
    Prepared,
}

impl BibleShowSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Prepared => "prepared",
        }
    }
}

/// Who asked for a show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleShowOrigin {
    pub source: BibleShowSource,
    /// `touch` / `desktop` once the console sends it (#850 B2a).
    pub device: Option<String>,
    pub client_ip: String,
}

/// The fields of one `presenter::bible::show` line; `-` for a missing value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BibleShowLine<'a> {
    pub(crate) source: &'static str,
    pub(crate) device: &'a str,
    pub(crate) client_ip: &'a str,
    pub(crate) reference: &'a str,
    pub(crate) secondary: &'a str,
}

impl<'a> BibleShowLine<'a> {
    pub(crate) fn new(origin: &'a BibleShowOrigin, output: &'a BibleSlideOutput) -> Self {
        Self {
            source: origin.source.as_str(),
            device: origin.device.as_deref().unwrap_or("-"),
            client_ip: &origin.client_ip,
            reference: &output.main_reference,
            secondary: if output.secondary_reference.is_empty() {
                "-"
            } else {
                &output.secondary_reference
            },
        }
    }
}

pub(crate) fn log_bible_show(origin: &BibleShowOrigin, output: &BibleSlideOutput) {
    let line = BibleShowLine::new(origin, output);
    tracing::info!(
        target: "presenter::bible::show",
        source = line.source,
        device = line.device,
        client_ip = line.client_ip,
        reference = line.reference,
        secondary = line.secondary,
        "bible show"
    );
}
```

`state/mod.rs` (after line 37):

```rust
mod bible_manager;
pub(crate) mod bible_show_log;
mod bible_source;
```

`state/bible.rs` line 3 becomes two lines, and the method gains the parameter and the log call:

```rust
use super::bible_show_log::{log_bible_show, BibleShowOrigin};
use super::AppState;
```

```rust
    pub async fn trigger_bible_slide_output(
        &self,
        output: BibleSlideOutput,
        reference_metadata: BibleSlideReferenceMetadata,
        origin: BibleShowOrigin,
    ) {
        log_bible_show(&origin, &output);
        // Store as the new active output
```

`router/bible/broadcast.rs` imports (lines 6-15) become:

```rust
use anyhow::Context;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tracing::instrument;

use super::super::AppError;
use crate::state::bible_show_log::{BibleShowOrigin, BibleShowSource};
use crate::state::AppState;
```

`trigger_bible_slide` (139) gains two extractors before the body extractor, and its call (161-163) passes the origin:

```rust
pub(crate) async fn trigger_bible_slide(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(payload): Json<BibleTriggerSlideRequest>,
) -> Result<Json<BibleTriggerSlideResponse>, AppError> {
```

```rust
    let origin = show_origin(BibleShowSource::Api, &headers, connect_info);
    state
        .trigger_bible_slide_output(output.clone(), reference_metadata, origin)
        .await;
```

`trigger_presentation_slide` (171) likewise, with `BibleShowSource::Prepared` at its call (217-219):

```rust
pub(crate) async fn trigger_presentation_slide(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    axum::extract::Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<BibleTriggerSlideResponse>, AppError> {
```

```rust
    let origin = show_origin(BibleShowSource::Prepared, &headers, connect_info);
    state
        .trigger_bible_slide_output(output.clone(), reference_metadata, origin)
        .await;
```

and before `#[instrument(skip_all)] pub(crate) async fn clear_bible_broadcast` (226):

```rust
/// Who asked for a show (#850 §8): the endpoint's source and the client IP by
/// the live WS rule (X-Forwarded-For only from a loopback peer).
fn show_origin(
    source: BibleShowSource,
    headers: &HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> BibleShowOrigin {
    let peer = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    BibleShowOrigin {
        source,
        device: None,
        client_ip: super::super::integrations::extract_client_ip(headers, peer),
    }
}
```

- [ ] **Step 4: Local gates**

```bash
cd "$REPO"
cargo fmt --all -- --check
grep -rn 'trigger_bible_slide_output(' crates/ | grep -v 'pub async fn'   # exactly: broadcast.rs ×2, state/tests.rs ×1
QC_TARGETS=$'crates/presenter-server/src/state/bible_show_log.rs\ncrates/presenter-server/src/state/bible.rs\ncrates/presenter-server/src/router/bible/broadcast.rs' python3 scripts/dev/fn_length_check.py .
bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/state/bible.rs          # 763 (< 800)
bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/router/bible/broadcast.rs  # 274
```

- [ ] **Step 5: Commit** — paths `crates/presenter-server/src/state/bible_show_log.rs crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/bible.rs crates/presenter-server/src/router/bible/broadcast.rs crates/presenter-server/src/state/tests.rs crates/presenter-server/src/router/tests.rs`, message:

```text
feat(server): one presenter::bible::show INFO line per Bible show (#850 O1)

trigger_bible_slide_output now takes a BibleShowOrigin (source endpoint,
device, client IP) and logs source, device, client_ip, reference and
secondary before it publishes. /bible/trigger-slide reports "api", the
prepared-slide trigger "prepared"; the console (B2a) and the rerouted legacy
path (A2b) add their sources with their callers.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 3: Remove the stale drop-ins on SNV and PP

**Files:**
- Modify: `.github/workflows/deploy.yml:435-441` ("Deploy systemd service" step: the #544 comment + the `ssh deploy-target` block)
- Modify: `.github/workflows/release.yml:410-415` (same step)

**Interfaces:** none (shell inside existing remote steps; the `ssh_setup` gating of `check_deploy_workflows.py` is unaffected — these steps carry no status function).

- [ ] **Step 1: Confirm the current state (read-only).** Verified 2026-10-13: SNV `presenter.service.d/` = `ableset-debug.conf noresolume.conf stage-url.conf sync-peer.conf`, and `ableset-debug.conf` sets `RUST_LOG=info,…,tower_http=info,…`; PP has only `stage-url.conf sync-peer.conf` (the `rm -f` is a no-op there). Re-check:

```bash
ssh -o BatchMode=yes newlevel@presenter.lan "ls /etc/systemd/system/presenter.service.d/"
ssh -o BatchMode=yes newlevel@companion-pp.lan "ls /etc/systemd/system/presenter.service.d/"
```

- [ ] **Step 2: Edit `deploy.yml`.** Lines 435-441 become:

```yaml
          # #544: the service's GStreamer registry must not survive this deploy — a driver
          # or permission change would otherwise keep being judged by the old cache.
          # #850 O1: drop two leftover drop-ins — ableset-debug.conf (closed #600) overrides
          # the unit's RUST_LOG with tower_http=info, which hides the request log; noresolume.conf
          # sets an env var no code reads. rm -f keeps this idempotent.
          ssh deploy-target "sudo cp /tmp/presenter.service /etc/systemd/system/ && \
            sudo install -m 0755 /tmp/wait-for-h264-encoder.sh ${{ env.DEPLOY_DIR }}/wait-for-h264-encoder.sh && \
            sudo rm -f ${{ env.DEPLOY_DIR }}/gstreamer-registry.bin && \
            sudo mkdir -p /etc/systemd/system/presenter.service.d && \
            sudo rm -f /etc/systemd/system/presenter.service.d/ableset-debug.conf /etc/systemd/system/presenter.service.d/noresolume.conf && \
            sudo systemctl daemon-reload"
```

- [ ] **Step 3: Edit `release.yml`.** Lines 410-415 become:

```yaml
          # #544: the registry must not survive the deploy (see deploy.yml).
          # #850 O1: drop the leftover ableset-debug.conf / noresolume.conf drop-ins (see deploy.yml).
          ssh deploy-target "sudo cp /tmp/presenter.service /etc/systemd/system/ && \
            sudo install -m 0755 /tmp/wait-for-h264-encoder.sh ${{ env.DEPLOY_DIR }}/wait-for-h264-encoder.sh && \
            sudo rm -f ${{ env.DEPLOY_DIR }}/gstreamer-registry.bin && \
            sudo mkdir -p /etc/systemd/system/presenter.service.d && \
            sudo rm -f /etc/systemd/system/presenter.service.d/ableset-debug.conf /etc/systemd/system/presenter.service.d/noresolume.conf && \
            sudo systemctl daemon-reload"
```

- [ ] **Step 4: Verify locally** (the `quality` job's own checks):

```bash
cd "$REPO"
for w in .github/workflows/deploy.yml .github/workflows/release.yml; do python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" "$w"; done
python3 -m unittest discover -s scripts/ci -p 'test_check_deploy_workflows.py'
python3 scripts/ci/check_deploy_workflows.py      # "deploy-workflow SSH safety OK"
```

The real verification is the post-deploy check in Task 5 (both files gone on SNV, `RUST_LOG=info,tower_http=debug` effective).

- [ ] **Step 5: Commit** — paths `.github/workflows/deploy.yml .github/workflows/release.yml`, message:

```text
ci(deploy): remove the stale ableset-debug / noresolume drop-ins on SNV and PP (#850 O1) [no-test: deploy workflow config, verified by the post-deploy journal check]

ableset-debug.conf (closed #600) overrides the unit's RUST_LOG with
tower_http=info, so the new request log would stay invisible on SNV;
noresolume.conf sets an env var no code reads. rm -f before daemon-reload,
idempotent on both hosts (PP has neither file).

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 4: Cloudflare Access reconcile script

**Files:**
- Create: `ops/cloudflare/presenter_access.py`, `ops/cloudflare/test_presenter_access.py`, `ops/cloudflare/presenter-access.example.json`
- Modify: `.gitignore` (append after line 51)
- Modify: `.github/workflows/pipeline.yml:452-455` (new step right after "Check E2E harness has no silent cargo fallback (#802)", before "Install Rust toolchain" at 457)

**Interfaces:**
- Produces (Python): `validate_config(config: dict) -> None` (raises `ConfigError`), `desired_apps(config: dict) -> list[dict]`, `normalize_policy(policy: dict) -> dict`, `changed_fields(have: dict, want: dict) -> list[str]`, `plan(existing: list[dict], desired: list[dict]) -> list[dict]` (actions `create` / `update_app` / `create_policy` / `update_policy` / `extra_policy` / `ok`), `describe(action: dict) -> str`, `apply_action(action: dict, account: str, call) -> bool`, `main(argv=None, call=http_call) -> int`.
- Consumes: the session script `$SCRATCH/cf_presenter_access.py` (its `ACCOUNT`, `ALLOWED_EMAILS`, `SITES` seed the real config) and the vault token file `cloudflare-presenter-access` (persisted 0600 at paste, 2026-10-13) through `airuleset.py secret exec --file … --env CF_TOKEN`.

- [ ] **Step 1: Write the failing tests** — `ops/cloudflare/test_presenter_access.py`:

```python
#!/usr/bin/env python3
"""Self-tests for presenter_access.py (#850 spec §9) — pure functions, no network.

Run: python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py' -v
"""

from __future__ import annotations

import copy
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import presenter_access as pa  # noqa: E402

EXAMPLE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "presenter-access.example.json")


def example_config() -> dict:
    with open(EXAMPLE, encoding="utf-8") as handle:
        return json.load(handle)


def live_apps(config: dict) -> list[dict]:
    """What GET /access/apps returns once the desired state is applied: ids added."""
    apps = copy.deepcopy(pa.desired_apps(config))
    for n, app in enumerate(apps):
        app["id"] = f"app-{n}"
        app["aud"] = f"aud-{n}"
        for m, policy in enumerate(app["policies"]):
            policy["id"] = f"pol-{n}-{m}"
            policy["created_at"] = "2026-10-13T12:00:00Z"
    return apps


class DesiredAppsTests(unittest.TestCase):
    def test_two_apps_per_host_with_bypass_then_allow(self):
        apps = pa.desired_apps(example_config())
        self.assertEqual([a["domain"] for a in apps], [
            "presenter-site-a.example.com", "presenter-site-a.example.com/healthz",
            "presenter-site-b.example.com", "presenter-site-b.example.com/healthz"])
        main = apps[0]
        self.assertEqual(main["session_duration"], "720h")
        self.assertEqual([(p["name"], p["decision"], p["precedence"]) for p in main["policies"]],
                         [(pa.BYPASS_NAME, "bypass", 1), (pa.ALLOW_NAME, "allow", 2)])
        self.assertEqual(main["policies"][0]["include"], [{"ip": {"ip": "192.0.2.10/32"}}])
        self.assertEqual(main["policies"][1]["include"], [{"email": {"email": "person@example.com"}}])
        self.assertEqual(apps[1]["session_duration"], "24h")
        self.assertEqual(apps[1]["policies"][0]["include"], [{"everyone": {}}])

    def test_a_site_without_bypass_ips_gets_only_the_allow_policy(self):
        config = example_config()
        config["sites"][0]["bypass_ips"] = []
        policies = pa.desired_apps(config)[0]["policies"]
        self.assertEqual([(p["name"], p["precedence"]) for p in policies], [(pa.ALLOW_NAME, 1)])

    def test_invalid_configs_are_refused(self):
        for broken in ({"allowed_emails": ["a@example.com"], "sites": [{"host": "h"}]},
                       {**example_config(), "allowed_emails": []},
                       {**example_config(), "sites": [{"host": "h", "bypass_ips": ["192.0.2.1"]}]}):
            with self.assertRaises(pa.ConfigError):
                pa.desired_apps(broken)


class PlanTests(unittest.TestCase):
    def test_applied_state_plans_only_ok(self):
        config = example_config()
        actions = pa.plan(live_apps(config), pa.desired_apps(config))
        self.assertEqual([a["op"] for a in actions], ["ok"] * 4)

    def test_missing_apps_are_created(self):
        config = example_config()
        actions = pa.plan(live_apps(config)[:2], pa.desired_apps(config))
        self.assertEqual([a["op"] for a in actions], ["ok", "ok", "create", "create"])

    def test_a_new_email_updates_the_allow_policy_in_place(self):
        config = example_config()
        existing = live_apps(config)
        config["allowed_emails"].append("second@example.com")
        actions = pa.plan(existing, pa.desired_apps(config))
        update = [a for a in actions if a["op"] == "update_policy"]
        self.assertEqual(len(update), 2)
        self.assertEqual(update[0]["policy_id"], "pol-0-1")
        self.assertEqual(update[0]["fields"], ["include"])

    def test_include_order_does_not_count_as_a_change(self):
        config = example_config()
        config["allowed_emails"] = ["a@example.com", "b@example.com"]
        existing = live_apps(config)
        existing[0]["policies"][1]["include"].reverse()
        self.assertEqual(pa.plan(existing, pa.desired_apps(config))[0]["op"], "ok")

    def test_session_change_keeps_the_existing_policies(self):
        config = example_config()
        existing = live_apps(config)
        config["session_duration"] = "336h"
        actions = pa.plan(existing, pa.desired_apps(config))
        update = [a for a in actions if a["op"] == "update_app"]
        self.assertEqual(len(update), 2)
        self.assertEqual(update[0]["body"]["session_duration"], "336h")
        self.assertEqual(update[0]["body"]["policies"], [{"id": "pol-0-0", "precedence": 1}, {"id": "pol-0-1", "precedence": 2}])
        self.assertNotIn("aud", update[0]["body"])

    def test_an_extra_policy_is_reported_never_deleted(self):
        config = example_config()
        existing = live_apps(config)
        existing[0]["policies"].append({"id": "pol-x", "name": "manual", "decision": "allow", "precedence": 3, "include": []})
        actions = pa.plan(existing, pa.desired_apps(config))
        self.assertEqual(actions[0]["op"], "extra_policy")
        self.assertTrue(pa.apply_action(actions[0], "acct", lambda *a: self.fail("extra_policy must not call the API")))


class OutputTests(unittest.TestCase):
    def test_describe_never_prints_emails_or_ips(self):
        config = example_config()
        existing = live_apps(config)
        config["allowed_emails"].append("second@example.com")
        for action in pa.plan(existing, pa.desired_apps(config)):
            line = pa.describe(action)
            self.assertNotIn("@", line)
            self.assertNotIn("192.0.2", line)

    def test_dry_run_writes_nothing(self):
        config = example_config()
        calls = []

        def fake(method, path, body):
            calls.append((method, path))
            return 200, {"success": True, "result": live_apps(config)[:2]}

        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(config, handle)
        try:
            out = io.StringIO()
            with redirect_stdout(out):
                code = pa.main(["--config", handle.name], call=fake)
        finally:
            os.unlink(handle.name)
        self.assertEqual(code, 0)
        self.assertEqual([m for m, _ in calls], ["GET"])
        self.assertIn("would create", out.getvalue())

    def test_apply_posts_the_missing_apps(self):
        config = example_config()
        calls = []

        def fake(method, path, body):
            calls.append((method, path))
            if method == "GET":
                return 200, {"success": True, "result": live_apps(config)[:2]}
            return 200, {"success": True, "result": {"id": "new"}}

        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(config, handle)
        try:
            with redirect_stdout(io.StringIO()):
                code = pa.main(["--config", handle.name, "--apply"], call=fake)
        finally:
            os.unlink(handle.name)
        self.assertEqual(code, 0)
        self.assertEqual(calls[1:], [("POST", "/accounts/<cloudflare-account-id>/access/apps")] * 2)


if __name__ == "__main__":
    unittest.main()
```

and the placeholder config `ops/cloudflare/presenter-access.example.json` (RFC 5737 addresses, `example.com`):

```json
{
  "account_id": "<cloudflare-account-id>",
  "allowed_emails": ["person@example.com"],
  "sites": [
    {"host": "presenter-site-a.example.com", "bypass_ips": ["192.0.2.10/32"]},
    {"host": "presenter-site-b.example.com", "bypass_ips": ["198.51.100.20/32"]}
  ],
  "session_duration": "720h",
  "healthz_session_duration": "24h"
}
```

- [ ] **Step 2: Run them — RED**

```bash
cd "$REPO" && python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py' -v
```

Expected: `ModuleNotFoundError: No module named 'presenter_access'`.

- [ ] **Step 3: Implement** — `ops/cloudflare/presenter_access.py` (the session script reworked into reconcile mode; apps are matched by domain, policies by name, the existing policy id is kept, nothing is ever deleted):

```python
#!/usr/bin/env python3
"""Reconcile the Cloudflare Access apps of the public Presenter hosts (#850 spec §9).

Desired state = the JSON file named by --config (or PRESENTER_ACCESS_CONFIG): account
id, allowed e-mails, hosts and the on-site bypass IPs. Only
``presenter-access.example.json`` (placeholders) is committed — the repo is public; the
real file lives outside it (dev2: ``~/.config/presenter/presenter-access.json``, 0600).

Dry run by default; ``--apply`` writes. Creates missing apps, updates the session
duration and the policies of existing apps, never deletes anything (an extra policy
is reported). Prints app ids, domains and policy names only — never the token,
e-mails or IPs. The token reaches the script from the vault as CF_TOKEN:

    python3 ~/devel/airuleset/airuleset.py secret exec \\
      --file <vault>/cloudflare-presenter-access --env CF_TOKEN -- \\
      python3 ops/cloudflare/presenter_access.py \\
      --config ~/.config/presenter/presenter-access.json [--apply]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from typing import Any, Callable

API = "https://api.cloudflare.com/client/v4"
APP_DEFAULTS = {
    "type": "self_hosted",
    "auto_redirect_to_identity": False,
    "allowed_idps": [],
    "app_launcher_visible": False,
    "http_only_cookie_attribute": True,
}
APP_KEYS = ("name", "domain", "type", "session_duration", *APP_DEFAULTS)
BYPASS_NAME = "presenter on-site network (no login)"
ALLOW_NAME = "presenter allowed users"
HEALTHZ_NAME = "presenter healthz probe bypass"

Call = Callable[[str, str, "dict[str, Any] | None"], "tuple[int, dict[str, Any]]"]


class ConfigError(ValueError):
    """The Access config is missing a key or holds an unusable value."""


def validate_config(config: dict[str, Any]) -> None:
    for key in ("account_id", "allowed_emails", "sites"):
        if not config.get(key):
            raise ConfigError(f"config needs a non-empty {key!r}")
    for site in config["sites"]:
        if not site.get("host"):
            raise ConfigError("every site needs a 'host'")
        for cidr in site.get("bypass_ips", []):
            if "/" not in cidr:
                raise ConfigError(f"bypass IP of {site['host']} needs a CIDR suffix such as /32")


def desired_apps(config: dict[str, Any]) -> list[dict[str, Any]]:
    """The Access apps the config asks for: per host one app + one /healthz bypass app."""
    validate_config(config)
    apps = []
    for site in config["sites"]:
        host = site["host"]
        policies = []
        if site.get("bypass_ips"):
            policies.append({"name": BYPASS_NAME, "decision": "bypass",
                             "include": [{"ip": {"ip": cidr}} for cidr in site["bypass_ips"]]})
        policies.append({"name": ALLOW_NAME, "decision": "allow",
                         "include": [{"email": {"email": e}} for e in config["allowed_emails"]]})
        for precedence, policy in enumerate(policies, start=1):
            policy["precedence"] = precedence
        apps.append({**APP_DEFAULTS, "name": f"presenter — {host}", "domain": host,
                     "session_duration": config.get("session_duration", "720h"), "policies": policies})
        apps.append({**APP_DEFAULTS, "name": f"presenter healthz — {host}", "domain": f"{host}/healthz",
                     "session_duration": config.get("healthz_session_duration", "24h"),
                     "policies": [{"name": HEALTHZ_NAME, "decision": "bypass", "precedence": 1,
                                   "include": [{"everyone": {}}]}]})
    return apps


def normalize_policy(policy: dict[str, Any]) -> dict[str, Any]:
    include = sorted(json.dumps(rule, sort_keys=True) for rule in policy.get("include") or [])
    return {"decision": policy.get("decision"), "precedence": policy.get("precedence"), "include": include}


def changed_fields(have: dict[str, Any], want: dict[str, Any]) -> list[str]:
    a, b = normalize_policy(have), normalize_policy(want)
    return [field for field in ("decision", "precedence", "include") if a[field] != b[field]]


def plan(existing: list[dict[str, Any]], desired: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Actions that bring ``existing`` (GET /access/apps) to ``desired``; pure, no network."""
    by_domain = {app.get("domain"): app for app in existing}
    actions: list[dict[str, Any]] = []
    for want in desired:
        have = by_domain.get(want["domain"])
        if have is None:
            actions.append({"op": "create", "domain": want["domain"], "app": want})
            continue
        app_actions = []
        if have.get("session_duration") != want["session_duration"]:
            body = {key: have[key] for key in APP_KEYS if key in have}
            body["session_duration"] = want["session_duration"]
            body["policies"] = [{"id": p["id"], "precedence": p.get("precedence")} for p in have.get("policies") or []]
            app_actions.append({"op": "update_app", "domain": want["domain"], "app_id": have["id"], "body": body})
        have_policies = {p.get("name"): p for p in have.get("policies") or []}
        for policy in want["policies"]:
            current = have_policies.get(policy["name"])
            if current is None:
                app_actions.append({"op": "create_policy", "domain": want["domain"], "app_id": have["id"],
                                    "name": policy["name"], "policy": policy})
            elif changed_fields(current, policy):
                app_actions.append({"op": "update_policy", "domain": want["domain"], "app_id": have["id"],
                                    "policy_id": current["id"], "name": policy["name"], "policy": policy,
                                    "fields": changed_fields(current, policy)})
        wanted_names = {p["name"] for p in want["policies"]}
        for name in sorted(n for n in have_policies if n not in wanted_names):
            app_actions.append({"op": "extra_policy", "domain": want["domain"], "app_id": have["id"], "name": name})
        actions.extend(app_actions or [{"op": "ok", "domain": want["domain"], "app_id": have["id"]}])
    return actions


def describe(action: dict[str, Any]) -> str:
    """One output line: op, domain, ids, policy name and changed field names — no values."""
    parts = [f"{action['op']:<14}", action["domain"]]
    if "app_id" in action:
        parts.append(f"app={action['app_id']}")
    if "name" in action:
        parts.append(f"policy={action['name']!r}")
    if action.get("fields"):
        parts.append("changed=" + ",".join(action["fields"]))
    if action["op"] == "extra_policy":
        parts.append("(kept — delete it in the dashboard if it is unwanted)")
    return "  ".join(parts)


def apply_action(action: dict[str, Any], account: str, call: Call) -> bool:
    base = f"/accounts/{account}/access/apps"
    requests = {
        "create": ("POST", base, action.get("app")),
        "update_app": ("PUT", f"{base}/{action.get('app_id')}", action.get("body")),
        "create_policy": ("POST", f"{base}/{action.get('app_id')}/policies", action.get("policy")),
        "update_policy": ("PUT", f"{base}/{action.get('app_id')}/policies/{action.get('policy_id')}", action.get("policy")),
    }
    if action["op"] not in requests:
        return True
    method, path, body = requests[action["op"]]
    code, result = call(method, path, body)
    return code in (200, 201) and bool(result.get("success"))


def http_call(method: str, path: str, body: dict[str, Any] | None = None) -> tuple[int, dict[str, Any]]:
    token = os.environ["CF_TOKEN"].strip()
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(API + path, data=data, method=method,
                                     headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as err:
        try:
            return err.code, json.load(err)
        except ValueError:
            return err.code, {}


def main(argv: list[str] | None = None, call: Call = http_call) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--apply", action="store_true", help="write the changes (default: dry run)")
    parser.add_argument("--config", default=os.environ.get("PRESENTER_ACCESS_CONFIG"))
    args = parser.parse_args(argv)
    if not args.config:
        print("::error::set PRESENTER_ACCESS_CONFIG or pass --config", file=sys.stderr)
        return 2
    try:
        with open(os.path.expanduser(args.config), encoding="utf-8") as handle:
            config = json.load(handle)
        desired = desired_apps(config)
    except (OSError, ValueError) as err:
        print(f"::error::access config: {err}", file=sys.stderr)
        return 2
    code, body = call("GET", f"/accounts/{config['account_id']}/access/apps?per_page=100", None)
    if code != 200:
        print(f"::error::listing Access apps failed: HTTP {code} {[e.get('message') for e in body.get('errors', [])]}",
              file=sys.stderr)
        return 1
    failures = 0
    for action in plan(body.get("result") or [], desired):
        line = describe(action)
        if action["op"] in ("ok", "extra_policy"):
            print(line)
            continue
        if not args.apply:
            print("would " + line)
            continue
        ok = apply_action(action, config["account_id"], call)
        failures += 0 if ok else 1
        print(("done   " if ok else "FAILED ") + line)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Run them — GREEN**

```bash
cd "$REPO" && python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py' -v
```

Expected: `Ran 12 tests … OK`.

- [ ] **Step 5: Keep the real config out of the public repo.** Append to `.gitignore`:

```gitignore

# #850 O1: the real Cloudflare Access config (account id, e-mails, IPs) never enters this public repo
ops/cloudflare/*.json
!ops/cloudflare/presenter-access.example.json
```

and add the CI step to `pipeline.yml` after line 455:

```yaml

      # #850 O1: the Cloudflare Access reconcile planner (pure functions, no network).
      - name: Test Cloudflare Access reconcile script (#850)
        run: |
          set -euo pipefail
          python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py' -v
```

Check: `python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" .github/workflows/pipeline.yml && python3 scripts/ci/check_deploy_workflows.py`.

- [ ] **Step 6: Write the real config outside the repo** (0600; values from the session script that created the live apps):

```bash
mkdir -p ~/.config/presenter && umask 077 && python3 -I - "$SCRATCH/cf_presenter_access.py" ~/.config/presenter/presenter-access.json <<'EOF'
import json, runpy, sys
g = runpy.run_path(sys.argv[1])  # defines ACCOUNT, ALLOWED_EMAILS, SITES; main() runs only under __main__
config = {"account_id": g["ACCOUNT"], "allowed_emails": g["ALLOWED_EMAILS"],
          "sites": [{"host": host, "bypass_ips": [f"{ip}/32"]} for host, ip in g["SITES"]],
          "session_duration": "720h", "healthz_session_duration": "24h"}
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    handle.write(json.dumps(config, indent=1) + "\n")
print("hosts:", [site["host"] for site in config["sites"]])
EOF
stat -c '%a %n' ~/.config/presenter/presenter-access.json     # 600
```

- [ ] **Step 7: Live dry run (read-only GET).** The apps applied on 2026-10-13 must plan as four `ok` lines:

```bash
cd "$REPO" && python3 ~/devel/airuleset/airuleset.py secret exec --file ~/.secrets/cloudflare-presenter-access --env CF_TOKEN -- \
  python3 ops/cloudflare/presenter_access.py --config ~/.config/presenter/presenter-access.json
```

Expected: `ok  prsnv… app=…`, `ok  prsnv…/healthz …`, `ok  prpp…`, `ok  prpp…/healthz …`. Any `update_policy … changed=<field>` means the API returns a policy shape `normalize_policy` does not expect: fix the normalisation, add a unittest with the observed field names (never values), re-run. Do NOT pass `--apply` in this plan — nothing needs to change. Post the result line count (not the lines) as a comment on #850.

- [ ] **Step 8: Commit** — paths `ops/cloudflare/presenter_access.py ops/cloudflare/test_presenter_access.py ops/cloudflare/presenter-access.example.json .gitignore .github/workflows/pipeline.yml`, message:

```text
feat(ops): reconcile script for the Presenter Cloudflare Access apps (#850 O1)

The session's create-only apply script becomes a reconcile tool: desired
apps from a config file kept outside the public repo, a pure plan() against
GET /access/apps (create / update_app / create_policy / update_policy /
extra_policy / ok), dry run by default, --apply writes, never deletes,
prints ids and names only. The CI quality job runs its unittest; the live
dry run against the applied apps plans four "ok" lines.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 5: O1 playbook rules, then ship O1 to SNV and PP

**Files:**
- Create: `.claude/rules/observability.md`, `.claude/rules/cloudflare-access.md`
- Modify: `CLAUDE.md:60` (two rows appended to the Playbook Router table after the "AI backend config" row, before the blank line 61)

**Interfaces:** none.

- [ ] **Step 1: Write the rules.** `.claude/rules/observability.md`:

```markdown
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
```

`.claude/rules/cloudflare-access.md`:

```markdown
---
paths:
  - "ops/cloudflare/**"
---

# Cloudflare Access for the public Presenter hosts (#850 §9)

- **The repo is PUBLIC.** Only `ops/cloudflare/presenter-access.example.json` (placeholders) is
  committed; `.gitignore` refuses any other `ops/cloudflare/*.json`. The real config (account id,
  e-mails, hosts, on-site bypass IPs) is `~/.config/presenter/presenter-access.json` (0600) on dev2.
  The Cloudflare token stays in the vault file `cloudflare-presenter-access` and reaches the script
  only through `secret exec` (the vault hook refuses hand reads/writes under the vault dir, so the
  non-secret config does not live there).
- **Run — dry run first, always:**
  `python3 ~/devel/airuleset/airuleset.py secret exec --file <vault>/cloudflare-presenter-access --env CF_TOKEN -- python3 ops/cloudflare/presenter_access.py --config ~/.config/presenter/presenter-access.json`
  The live state is applied, so a clean run prints four `ok` lines. Add `--apply` only when the dry
  run shows exactly the intended `create` / `update_*` lines, then dry-run again (4× `ok`).
- **Adding a person** = one more e-mail in `allowed_emails` → the dry run shows
  `update_policy … changed=include` for both hosts → `--apply`.
- The script never deletes: a policy the config does not name is printed as `extra_policy` and kept.
- `update_app` (a session-duration change) PUTs the existing app with its policy ids kept —
  **[unverified]** against the live API (the 2026-10-13 apps were created, never updated). The first
  real `update_app` must be followed by a dry run that prints `ok`.
- `desired_apps()` / `plan()` are pure and unit-tested in `test_presenter_access.py` (CI `quality` job).
```

CLAUDE.md, after line 60:

```markdown
| Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, client IP = the #793 rule, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines) | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `router.rs`, `state/bible_show_log.rs`, the units) |
| Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
```

Commit — paths `.claude/rules/observability.md .claude/rules/cloudflare-access.md CLAUDE.md`, message `docs(playbook): request log, Bible show line and Access reconcile rules (#850 O1) [no-test: docs only]` + body one line + the Co-Authored-By line.

- [ ] **Step 2: Local gates for the whole unit**

```bash
cd "$REPO"
cargo fmt --all -- --check
bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .
python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py'
python3 -m unittest discover -s scripts/ci -p 'test_check_deploy_workflows.py' && python3 scripts/ci/check_deploy_workflows.py
for w in .github/workflows/*.yml; do python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" "$w"; done
git status --short    # only the untracked operator-stage-churn-scroll.spec.ts may remain
```

- [ ] **Step 3: Push dev and watch CI.** `git fetch origin && git merge --ff-only origin/dev` (abort and merge normally if dev moved), `git push origin dev`, then monitor the `pipeline.yml` run to a terminal state (all jobs incl. e2e + deploy-dev). A red job: `gh run view <id> --log-failed`, fix the root cause in one commit.

- [ ] **Step 4: Open and merge the release PR** (dev quiet — no other lane mid-flight; otherwise ask the supervisor first).

```bash
gh pr create --base main --head dev --title "Release v0.4.316: request log, Bible show line, Access reconcile script (#850 O1)" --body-file "$SCRATCH/pr-o1.md"
```

`pr-o1.md`: summary of Tasks 1-4, the spec docs commits it carries (5f8ebb85 and the two mockup commits), the test list, and the closing line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. When every check is green and the PR is `mergeable: true` + `mergeable_state: clean`: `gh pr merge <N> --merge` (never `--admin`, never squash).

- [ ] **Step 5: Verify SNV after `deploy.yml`** (load `post-deploy-verification`): `/healthz` version `0.4.316`, the operator footer shows `v0.4.316` in the live DOM, then S10:

```bash
ssh -o BatchMode=yes newlevel@presenter.lan "ls /etc/systemd/system/presenter.service.d/; \
  systemctl show presenter -p Environment --no-pager | tr ' ' '\n' | grep RUST_LOG; \
  journalctl -u presenter --since '-10 min' --no-pager | grep -m3 'tower_http::trace::on_response'"
```

Expected: only `stage-url.conf sync-peer.conf`; `RUST_LOG=info,tower_http=debug`; lines like `DEBUG request{method=GET uri=/healthz user_agent=… client_ip=…}: tower_http::trace::on_response: finished processing request latency=0 ms status=200`. Then one show + clear from dev2 (the deploy's functional check; prod testing is standing-approved):

```bash
curl -fsS -X POST http://10.77.9.205/bible/trigger-slide -H 'content-type: application/json' \
  -d '{"mainText":"Lebo Boh tak miloval svet, že dal svojho jednorodeného Syna.","mainReference":"Ján 3:16 (SEB)"}' >/dev/null
curl -fsS -X POST http://10.77.9.205/bible/clear -o /dev/null
ssh -o BatchMode=yes newlevel@presenter.lan "journalctl -u presenter --since '-2 min' --no-pager | grep 'presenter::bible::show'"
```

Expected: `INFO presenter::bible::show: bible show source="api" device="-" client_ip="10.77.8.134" reference="Ján 3:16 (SEB)" secondary="-"`.

- [ ] **Step 6: Release to PP and verify.** `gh release create v0.4.316 --target main --title v0.4.316 --generate-notes`; monitor `release.yml` to terminal; PP `/healthz` → `0.4.316`, footer `v0.4.316`, `ls /etc/systemd/system/presenter.service.d/` unchanged.

- [ ] **Step 7: Measure the journal volume** (spec §8 [unverified]): one hour after the SNV deploy, `ssh newlevel@presenter.lan "journalctl -u presenter --since '-1 hour' --no-pager | grep -c on_response"`. Post the number on #850 as a FINDING. Above 2,000 lines/h, the exclusion of `/ui-pkg/*` and the settings poll route becomes the first feature commit of A1a's PR (after its bump); at or below, record "kept as is".

- [ ] **Step 8: Close the unit.** Run the `playbook-review` skill (emit the `📔 Playbook:` line), update #850 with the shipped O1 items.

---

## PR unit A1a — the book-name table (v0.4.317)

Starts after the O1 release: main = 0.4.316 = dev, so the first commit bumps.

### Task 6: Post-release version bump to 0.4.317

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (this recipe is reused by Task 12)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"     # expected after the O1 release: main=0.4.316 dev=0.4.316
```

If main is still below the O1 version (O1 not released yet), STOP: A1a starts after the O1 release. If `DEV` is already above `MAIN`, skip Steps 2-3 (someone else bumped).

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.317 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to <NEXT> [no-test: version bump only]` (0.4.317 expected) + blank line + the Co-Authored-By line.

### Task 7: Commit the name sources and the notices

**Files:**
- Create: `data/bible-book-names/bprp-slk.yaml`, `bprp-ces.yaml`, `bprp-eng.yaml`, `bprp-eng-preferred.yaml`, `LICENSE-BPRP.md`, `biblia-sk.json`, `presenter-stored.json`, `curated.yaml`
- Create: `THIRD_PARTY_NOTICES.md` (repo root; none exists today)

**Interfaces:**
- Produces the generator's inputs (Task 9): BPRP `book_names/all/<lang>.yaml` = YAML list of `{osis: <id or list>, texts: [..]}`; `book_names/preferred/eng.yaml` = `{default: {<osis>: {long|short|shorter: str|list}}}`; `biblia-sk.json` = `{"seb"|"sep"|"ssv"|"roh": [[slug, abbreviation, title], …]}`; `presenter-stored.json` = `{"<translation>": {"<CODE>": "<stored name>"}}` for slk-seb, slk-roh, slk-sevp, slk-mil (4 books), eng-kjv, eng-nlt; `curated.yaml` keys `short_defaults`, `no_fold`, `remove`, `add`, `frequency_rank`.

- [ ] **Step 1: Fetch BPRP at the pinned commit** (MIT; byte-identical to the files the prototype used, checked 2026-10-13):

```bash
cd "$REPO" && mkdir -p data/bible-book-names && D=data/bible-book-names
SHA=5735787d970a87483eac580e5c1d1b5192c3ed48
BASE=https://raw.githubusercontent.com/openbibleinfo/Bible-Passage-Reference-Parser-Languages/$SHA
for lang in slk ces eng; do
  { echo "# Source: https://github.com/openbibleinfo/Bible-Passage-Reference-Parser-Languages/blob/$SHA/book_names/all/$lang.yaml (MIT, see LICENSE-BPRP.md)"
    curl -fsSL "$BASE/book_names/all/$lang.yaml"; } > "$D/bprp-$lang.yaml"
done
{ echo "# Source: https://github.com/openbibleinfo/Bible-Passage-Reference-Parser-Languages/blob/$SHA/book_names/preferred/eng.yaml (MIT, see LICENSE-BPRP.md)"
  curl -fsSL "$BASE/book_names/preferred/eng.yaml"; } > "$D/bprp-eng-preferred.yaml"
curl -fsSL "$BASE/LICENSE.md" -o "$D/LICENSE-BPRP.md"
wc -l $D/bprp-*.yaml    # 1284 / 1400 / 7701 / 617
```

The spec lists three BPRP files; the fourth (`preferred/eng`) keeps the prototype's tier-2 English forms (Task 9, tier rule).

- [ ] **Step 2: biblia.sk menus and the stored names**

```bash
cp "$SCRATCH/resolver-in/bibliask-menus.json" "$D/biblia-sk.json"
python3 - <<'EOF'
import json, urllib.request
out = {}
for t in ["slk-seb", "slk-roh", "slk-sevp", "slk-mil", "eng-kjv", "eng-nlt"]:
    with urllib.request.urlopen(f"http://10.77.8.134:8080/bible/books?translation={t}", timeout=30) as r:
        rows = json.load(r)
    out[t] = {b["code"]: b["book"] for b in sorted(rows, key=lambda b: b["number"])}
with open("data/bible-book-names/presenter-stored.json", "w", encoding="utf-8") as f:
    f.write(json.dumps(out, ensure_ascii=False, indent=1) + "\n")
print({k: len(v) for k, v in out.items()})   # 66 66 66 4 66 66
EOF
```

(dev's `/bible/books` equals the prod and PP snapshots byte for byte — checked for all six translations on 2026-10-13.)

- [ ] **Step 3: `data/bible-book-names/curated.yaml`** (short defaults = #850 Finding 6 table; `jd`, `1p`, `2p` re-point the Czech forms the self-check found landing on the wrong book; frequency = OpenBible.info ranks):

```yaml
# Hand-curated inputs of scripts/dev/gen_bible_book_names.py (#850 spec §5.2).
# Regenerate after any edit: python3 scripts/dev/gen_bible_book_names.py

# Keys of at most two letters (folded: ordinal digit + letters) -> book codes,
# best first. The resolver puts these first, then the other matches by score.
# Source: #850 Finding 6 / the "ambiguous short inputs" table.
short_defaults:
  e: [EPH, EZK, EXO, EZR]
  ez: [EZK, EZR]
  f: [PHP, PHM]
  g: [GEN, GAL]
  h: [HEB, HOS, HAB, HAG]
  j: [JHN, JER, JOB, JAS, JOS, JOL]
  jd: [JUD, JDG]
  jn: [JHN, 1JN, 2JN, 3JN, JON]
  jo: [JOB, JOS, JOL, JON, JHN]
  ju: [JUD, JDG]
  kr: [1KI, 2KI, 1CH, 2CH]
  l: [LUK, LEV, LAM]
  m: [MAT, MRK, MIC, MAL]
  na: [LAM, NAM]
  pt: [1PE, 2PE]
  r: [ROM, RUT, REV]
  s: [ACT, JDG, ZEP, 1SA]
  z: [PSA, REV, HEB, ZEC, LAM, ZEP]
  za: [PSA, ZEC, LAM]
  1p: [1PE, 1CH]
  2p: [2PE, 2CH]

# Initialisms matched BEFORE folding (BPRP slk marks "PŠ" normalize: none);
# folded they would collide with Psalms ("ps").
no_fold:
  - {form: "P.Š.", code: SNG}
  - {form: "PŠ", code: SNG}

# (form, code) pairs dropped from every source. "Ab" is Habakkuk in SEP and
# BPRP; Obadiah is "Abd" / "Ob.".
remove:
  - {form: "Ab", code: OBA}

# Forms no source carries. "Mr" (Mark) is used in ai/style_guide.md.
# SEB's stored "Filimonovi" (sic) comes from presenter-stored.json — keep it.
add:
  - {form: "Mr", code: MRK, tier: 1}

# OpenBible.info cross-reference votes per book, rank 1 = most cited
# (https://a.openbible.info/data/cross-references.zip, CC-BY, 2026-10-12).
frequency_rank:
  GEN: 16
  EXO: 19
  LEV: 30
  NUM: 28
  DEU: 12
  JOS: 33
  JDG: 46
  RUT: 63
  1SA: 27
  2SA: 39
  1KI: 32
  2KI: 38
  1CH: 41
  2CH: 29
  EZR: 53
  NEH: 47
  EST: 58
  JOB: 21
  PSA: 1
  PRO: 8
  ECC: 43
  SNG: 57
  ISA: 2
  JER: 6
  LAM: 49
  EZK: 15
  DAN: 31
  HOS: 40
  JOL: 52
  AMO: 50
  OBA: 66
  JON: 61
  MIC: 44
  NAM: 60
  HAB: 55
  ZEP: 56
  HAG: 59
  ZEC: 35
  MAL: 51
  MAT: 4
  MRK: 22
  LUK: 7
  JHN: 3
  ACT: 9
  ROM: 5
  1CO: 14
  2CO: 17
  GAL: 25
  EPH: 13
  PHP: 26
  COL: 23
  1TH: 36
  2TH: 48
  1TI: 37
  2TI: 34
  TIT: 45
  PHM: 64
  HEB: 11
  JAS: 24
  1PE: 18
  2PE: 42
  1JN: 20
  2JN: 62
  3JN: 65
  JUD: 54
  REV: 10
```

- [ ] **Step 4: `THIRD_PARTY_NOTICES.md`**

```markdown
# Third-party notices

## Bible-Passage-Reference-Parser-Languages (MIT)

`data/bible-book-names/bprp-slk.yaml`, `bprp-ces.yaml`, `bprp-eng.yaml` and
`bprp-eng-preferred.yaml` are copies (plus one source comment line) of
`book_names/all/{slk,ces,eng}.yaml` and `book_names/preferred/eng.yaml` from
<https://github.com/openbibleinfo/Bible-Passage-Reference-Parser-Languages> at commit
`5735787d970a87483eac580e5c1d1b5192c3ed48` (2026-02-08). License: MIT,
Copyright (c) 2026 Stephen Smith — full text in `data/bible-book-names/LICENSE-BPRP.md`.
`crates/presenter-core/src/bible/resolver/names_table.rs` is generated from them by
`scripts/dev/gen_bible_book_names.py`.

## OpenBible.info cross-references (CC BY)

`frequency_rank` in `data/bible-book-names/curated.yaml` ranks the 66 books by the
cross-reference votes in <https://a.openbible.info/data/cross-references.zip>
(OpenBible.info, Creative Commons Attribution license), downloaded 2026-10-12.

## biblia.sk book menus

`data/bible-book-names/biblia-sk.json` lists the book abbreviations and titles of the SEB,
SEP, SSV and ROH reader menus of <https://www.biblia.sk> (Slovenská biblická spoločnosť):
names and abbreviations only, no Bible text.
```

- [ ] **Step 5: Commit** — paths `data/bible-book-names THIRD_PARTY_NOTICES.md`, message:

```text
data(bible): committed book-name sources for the reference resolver (#850 A1a) [no-test: data only, read by the generator tests of the next commits]

BPRP-Languages slk/ces/eng (MIT, pinned 5735787d, LICENSE copied), the
biblia.sk reader menus of SEB/SEP/SSV/ROH, the stored names of the six
installed translations and the curated rules (short defaults, no-fold,
remove Ab→Obadiah, add Mr→Mark, OpenBible frequency ranks).

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 8: `Ab` is Habakkuk, not Obadiah

**Files:**
- Modify: `crates/presenter-core/src/bible/canonical.rs:610` (delete `    ("Ab", "OBA"),`)
- Modify: `crates/presenter-core/src/bible/tests/canonical.rs` (append after line 57)

**Interfaces:** `canonical_book_by_name("Ab")` now returns `None` (it is used only by the importer, `import.rs:190`, and the AI `load_bible_verses` tool; no stored name is "Ab").

- [ ] **Step 1: Write the failing test** — append to `bible/tests/canonical.rs`:

```rust

#[test]
fn ab_is_no_longer_obadiah() {
    // #850: "Ab" is Habakkuk in SEP and BPRP; Obadiah is "Abd" / "Ob.".
    assert_eq!(canonical_book_by_name("Ab"), None);
    let obadiah = canonical_book_by_name("Abdiáš").expect("Obadiah by its Slovak name");
    assert_eq!(obadiah.code, "OBA");
}
```

- [ ] **Step 2: How it is verified.** CI `test` job (it fails today: "Ab" → OBA).

- [ ] **Step 3: Implement** — delete line 610 `    ("Ab", "OBA"),` of `SLOVAK_ABBREVIATIONS` (572-622). `grep -n '"Ab"' crates/presenter-core/src/bible/canonical.rs` → nothing.

- [ ] **Step 4: Commit** — paths `crates/presenter-core/src/bible/canonical.rs crates/presenter-core/src/bible/tests/canonical.rs`, message:

```text
fix(bible): "Ab" no longer resolves to Obadiah (#850 A1a)

SEP and BPRP use "Ab" for Habakkuk; Obadiah is "Abd" / "Ob.". The exact
importer alias table keeps every other entry.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 9: The generator, its unittest and the CI `--check`

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/testdata/fold-cases.json`
- Create: `scripts/dev/test_gen_bible_book_names.py`, `scripts/dev/gen_bible_book_names.py`
- Create (generated): `crates/presenter-core/src/bible/resolver/names_table.rs` (wired into the crate in Task 10)
- Modify: `.github/workflows/pipeline.yml` (new step right after the "Test Cloudflare Access reconcile script (#850)" step from Task 4)

**Interfaces:**
- Produces (Python): `fold(text: str) -> tuple[int, str]` (ordinal 0 = none), `no_fold_key(text) -> str`, `tier(source) -> int`, `parse_canonical(text) -> (books, arrays, pairs)` (raises `GeneratorError` unless 66/66/66/66), `collect_forms(data, canonical_text) -> (list[Form], list[str])`, `build_table(forms, codes, curated) -> (list[Row], list[tuple[str, int]])`, `short_defaults(curated, rows, codes) -> list[tuple[str, list[int]]]` (raises on an unknown or unreachable code), `frequency(curated, codes) -> list[int]`, `render(...) -> str`, `generate(data=DATA, canonical=CANONICAL) -> str`, `main(argv=None) -> int` (`--check`, `--data`, `--canonical`, `--output`).
- Produces (Rust, generated): `pub static BOOK_CODES: [&str; 66]`, `pub static BOOK_KEYS: &[(&str, &str, u8, u8)]` (`(folded key, form as written, book index, tier << 4 | ordinal)`, 768 rows sorted by key → ordinal → index), `pub static NO_FOLD: &[(&str, u8)]` (`("pš", 21)`), `pub static SHORT_DEFAULTS: &[(&str, &[u8])]` (21 keys), `pub static FREQUENCY_RANK: [u8; 66]`. The spec's 3-tuple gains the display form: the lookup JSON's `"matched": "Žid"` and the console label "Hebrejom 11:1 · Žid · SEB" need it.

- [ ] **Step 1: Shared fold cases** — the same file feeds the Python unittest (this task) and the Rust fold test (Task 10):

```bash
cd "$REPO" && mkdir -p crates/presenter-core/src/bible/resolver/testdata && python3 - <<'EOF'
import json
cases = [
 ("Židom", 0, "zidom"), ("Žd", 0, "zd"), ("Ř", 0, "r"), ("Jób", 0, "job"),
 ("1 Korinťanom", 1, "korintanom"), ("1. Korinťanom", 1, "korintanom"), ("1kor", 1, "kor"),
 ("I Kor", 1, "kor"), ("I. Jánov", 1, "janov"), ("II. Kor", 2, "kor"), ("III. Jánov", 3, "janov"),
 ("IV. Mojžišova", 4, "mojzisova"), ("V. Mojžišova", 5, "mojzisova"),
 ("prvý Ján", 1, "jan"), ("Prvá kniha Mojžišova", 1, "mojzisova"), ("Prvýlist Jánov", 1, "janov"),
 ("Druhýlist Jánov", 2, "janov"), ("Tretílist Jánov", 3, "janov"), ("první Korintským", 1, "korintskym"),
 ("Štvrtá kniha Mojžišova", 4, "mojzisova"), ("Piata Mojžišova", 5, "mojzisova"),
 ("1 k. Mojžišova", 1, "mojzisova"), ("1 k Mojžišova", 1, "mojzisova"), ("K. stvorenia", 0, "stvorenia"),
 ("Evanjelium podľa Jána", 0, "jana"), ("List apoštola Pavla Rimanom", 0, "rimanom"),
 ("Kniha proroka Abdiáša", 0, "abdiasa"), ("Book of Revelation", 0, "revelation"), ("Saint John", 0, "john"),
 ("Iz", 0, "iz"), ("I", 0, "i"), ("I.", 1, ""), ("prvý", 1, ""), ("1", 1, ""), ("P.Š.", 0, "ps"), ("Rim.", 0, "rim"),
 ("1K", 1, "k"), ("1‑Kor", 1, "kor"), ("  1 Ján  ", 1, "jan"), ("Pieseň Šalamúnova", 0, "piesensalamunova"),
 ("Skutky apoštolov", 0, "skutky"), ("7 Kor", 0, "7kor"),
]
doc = {"note": "#850: book-text fold cases shared by the Rust fold tests and test_gen_bible_book_names.py — both folds must agree",
       "cases": [{"input": t, "ordinal": o, "key": k} for t, o, k in cases]}
with open("crates/presenter-core/src/bible/resolver/testdata/fold-cases.json", "w", encoding="utf-8") as f:
    json.dump(doc, f, ensure_ascii=False, indent=1)
print(len(cases), "cases")
EOF
```

- [ ] **Step 2: Write the failing unittest** — `scripts/dev/test_gen_bible_book_names.py`:

```python
#!/usr/bin/env python3
"""Self-tests for gen_bible_book_names.py (#850 spec §5.2, §10).

Run: python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' -v
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import gen_bible_book_names as gen  # noqa: E402

FOLD_CASES = gen.REPO / "crates" / "presenter-core" / "src" / "bible" / "resolver" / "testdata" / "fold-cases.json"


def table_rows() -> list[gen.Row]:
    curated = gen.yaml.safe_load((gen.DATA / "curated.yaml").read_text(encoding="utf-8"))
    forms, codes = gen.collect_forms(gen.DATA, gen.CANONICAL.read_text(encoding="utf-8"))
    rows, _ = gen.build_table(forms, codes, curated)
    return rows


class CanonicalSourceTests(unittest.TestCase):
    def test_canonical_arrays_have_66_entries_each(self):
        books, arrays, pairs = gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))
        self.assertEqual(len(books), 66)
        for name in gen.SLOVAK_ARRAYS:
            self.assertEqual(len(arrays[name]), 66, name)
        self.assertNotIn(("Ab", "OBA"), pairs["SLOVAK_ABBREVIATIONS"])

    def test_a_short_array_fails_loudly(self):
        text = gen.CANONICAL.read_text(encoding="utf-8").replace('    "Zjavenie Jána",\n', "", 1)
        with self.assertRaises(gen.GeneratorError):
            gen.parse_canonical(text)


class FoldTests(unittest.TestCase):
    def test_shared_fold_cases(self):
        cases = json.loads(FOLD_CASES.read_text(encoding="utf-8"))["cases"]
        self.assertGreaterEqual(len(cases), 40)
        for case in cases:
            self.assertEqual(gen.fold(case["input"]), (case["ordinal"], case["key"]), case["input"])

    def test_tiers(self):
        self.assertEqual(gen.tier("db:slk-seb"), 1)
        self.assertEqual(gen.tier("biblia.sk:sep:abbr"), 1)
        self.assertEqual(gen.tier("canonical.rs:SLOVAK_ABBREVIATIONS"), 1)
        self.assertEqual(gen.tier("curated:tier1"), 1)
        self.assertEqual(gen.tier("bprp:slk"), 2)
        self.assertEqual(gen.tier("db:eng-kjv"), 2)
        self.assertEqual(gen.tier("canonical.rs:english_name"), 2)
        self.assertEqual(gen.tier("osis"), 2)
        self.assertEqual(gen.tier("bprp:ces"), 3)
        self.assertEqual(gen.tier("bprp:eng:short"), 3)


class TableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = table_rows()
        codes = [code for code, _, _ in gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))[0]]
        cls.by_key = {}
        for row in cls.rows:
            cls.by_key.setdefault((row.key, row.ordinal), set()).add(codes[row.index])

    def test_data_fixes(self):
        self.assertEqual(self.by_key[("ab", 0)], {"HAB"}, "Ab is Habakkuk, never Obadiah")
        self.assertIn("MRK", self.by_key[("mr", 0)])
        self.assertIn("PHM", self.by_key[("filimonovi", 0)], "keep SEB's stored misspelling")
        self.assertEqual(self.by_key[("ps", 0)], {"PSA"}, "P.Š. must not fold into Psalms")
        self.assertIn("HEB", self.by_key[("zid", 0)])

    def test_slovak_beats_english_on_the_1k_collision(self):
        tiers = {(r.index, r.tier) for r in self.rows if r.key == "k" and r.ordinal == 1}
        self.assertIn((45, 1), tiers, "1K = 1 Korinťanom at tier 1 (SEP)")
        self.assertIn((10, 3), tiers, "1 K = 1 Kings only at tier 3 (BPRP English)")

    def test_rows_are_sorted_and_unique(self):
        keys = [(r.key, r.ordinal, r.index) for r in self.rows]
        self.assertEqual(keys, sorted(keys))
        self.assertEqual(len(keys), len(set(keys)))

    def test_short_defaults_reject_an_unreachable_code(self):
        curated = {"short_defaults": {"z": ["PSA", "GEN"]}}
        codes = [c for c, _, _ in gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))[0]]
        with self.assertRaises(gen.GeneratorError):
            gen.short_defaults(curated, self.rows, codes)


class CheckModeTests(unittest.TestCase):
    def test_check_passes_on_a_fresh_table_and_fails_on_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "names_table.rs"
            self.assertEqual(gen.main(["--output", str(output)]), 0)
            self.assertEqual(gen.main(["--check", "--output", str(output)]), 0)
            output.write_text(output.read_text(encoding="utf-8").replace('"zid"', '"zyd"'), encoding="utf-8")
            self.assertEqual(gen.main(["--check", "--output", str(output)]), 1)

    def test_committed_table_is_up_to_date(self):
        self.assertEqual(gen.main(["--check"]), 0)

    def test_a_missing_source_fails_without_writing(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "data"
            shutil.copytree(gen.DATA, data)
            (data / "biblia-sk.json").unlink()
            output = Path(tmp) / "names_table.rs"
            self.assertEqual(gen.main(["--data", str(data), "--output", str(output)]), 1)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
```

Run: `python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' -v` → `ModuleNotFoundError: No module named 'gen_bible_book_names'` (RED).

- [ ] **Step 3: Implement the generator** — `scripts/dev/gen_bible_book_names.py` (a port of the prototype's `build_inventory.py` + the `fold`/`tier` of `collisions.py`; entries whose `osis` is a list are skipped — the prototype's regex parser silently dropped them too):

```python
#!/usr/bin/env python3
"""Generate the Bible reference resolver's book-name table (#850 spec §5.2).

Reads the committed sources under ``data/bible-book-names/`` plus the Slovak
arrays of ``crates/presenter-core/src/bible/canonical.rs`` and writes
``crates/presenter-core/src/bible/resolver/names_table.rs``.

    python3 scripts/dev/gen_bible_book_names.py           # write the table
    python3 scripts/dev/gen_bible_book_names.py --check   # CI: fail on drift

``fold()`` here and ``fold_book_text`` in ``resolver/fold.rs`` must agree on
every form; the Rust tests re-fold every generated row's display form and the
shared ``resolver/testdata/fold-cases.json`` runs through both.
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
DATA = REPO / "data" / "bible-book-names"
CANONICAL = REPO / "crates" / "presenter-core" / "src" / "bible" / "canonical.rs"
OUTPUT = REPO / "crates" / "presenter-core" / "src" / "bible" / "resolver" / "names_table.rs"
HEADER = "// @generated by scripts/dev/gen_bible_book_names.py — do not edit"
SHORT_KEY_MAX = 2
STORED_TRANSLATIONS = ("slk-seb", "slk-roh", "slk-sevp", "slk-mil", "eng-kjv", "eng-nlt")
SLOVAK_ARRAYS = ("SLOVAK_ECUMENICKY_NAMES", "SLOVAK_ROHACEK_NAMES", "SLOVAK_OBOHU_NAMES")
OSIS_IDS = (
    "Gen Exod Lev Num Deut Josh Judg Ruth 1Sam 2Sam 1Kgs 2Kgs 1Chr 2Chr Ezra Neh Esth Job Ps "
    "Prov Eccl Song Isa Jer Lam Ezek Dan Hos Joel Amos Obad Jonah Mic Nah Hab Zeph Hag Zech Mal "
    "Matt Mark Luke John Acts Rom 1Cor 2Cor Gal Eph Phil Col 1Thess 2Thess 1Tim 2Tim Titus Phlm "
    "Heb Jas 1Pet 2Pet 1John 2John 3John Jude Rev"
).split()
BIBLIA_SK_SLUGS = (
    "gn ex lv nm dt joz sdc rut 1sam 2sam 1krl 2krl 1krn 2krn ezd neh est job z pris koh vlp iz "
    "jer nar ez dan oz joel am abd jon mich nah hab sof ag zach mal mt mk lk jn sk rim 1kor 2kor "
    "ga ef flp kol 1tes 2tes 1tim 2tim tit flm heb jk 1pt 2pt 1jn 2jn 3jn jud zj"
).split()


class GeneratorError(RuntimeError):
    """A source file is malformed — the table must not be written."""


@dataclass(frozen=True)
class Form:
    code: str
    text: str
    source: str


@dataclass(frozen=True)
class Row:
    key: str
    ordinal: int  # 0 = no ordinal
    index: int  # book number - 1
    tier: int
    display: str


# ---------------------------------------------------------------- folding


ORDINALS = [
    (re.compile(r"(prva|prvy|prvni|prve)(\s+(kniha|list|epistola))?\s*"), 1),
    (re.compile(r"(druha|druhy|druhe)(\s+(kniha|list|epistola))?\s*"), 2),
    (re.compile(r"(tretia|treti|treta)(\s+(kniha|list|epistola))?\s*"), 3),
    (re.compile(r"(stvrta|ctvrta)(\s+kniha)?\s*"), 4),
    (re.compile(r"(piata|pata)(\s+kniha)?\s*"), 5),
    (re.compile(r"iii(\.|\s)\s*"), 3),
    (re.compile(r"ii(\.|\s)\s*"), 2),
    (re.compile(r"iv(\.|\s)\s*"), 4),
    (re.compile(r"i(\.|\s)\s*"), 1),
    (re.compile(r"v\.\s*"), 5),
    (re.compile(r"([1-5])\s*\.?\s*k\.?(?=\s)"), None),
    (re.compile(r"([1-5])\s*\.?\s*"), None),
]
FILLER = re.compile(
    r"(^|\s)k\.?(?=\s)|\b(kniha|knihy|list|epistola|cize|evanjelium|evangelium|podla|podle|"
    r"svateho|svatych|apostola|apostolov|pavla|proroka|prorok|of|the|book|gospel|according|to|"
    r"saint|st)\b"
)


def strip_marks(text: str) -> str:
    decomposed = unicodedata.normalize("NFD", text)
    return "".join(c for c in decomposed if not unicodedata.combining(c)).lower()


def fold(text: str) -> tuple[int, str]:
    """(ordinal, key) of a book form; ordinal 0 = none. Mirrors fold.rs."""
    folded = re.sub(r"[‐-―]", "-", strip_marks(text).strip())
    ordinal = 0
    for pattern, value in ORDINALS:
        match = pattern.match(folded)
        if match:
            ordinal = value if value is not None else int(match.group(1))
            folded = folded[match.end():]
            break
    folded = FILLER.sub(" ", folded)
    return ordinal, re.sub(r"[^a-z0-9]", "", folded)


def no_fold_key(text: str) -> str:
    """A NO_FOLD form's lookup key: lowercase, dots and whitespace removed."""
    return re.sub(r"[.\s]", "", text.lower())


def tier(source: str) -> int:
    if source.startswith("curated:tier"):
        return int(source.removeprefix("curated:tier"))
    if source.startswith(("db:slk-", "biblia.sk", "canonical.rs:SLOVAK_")):
        return 1
    if source.startswith(("bprp:slk", "db:eng-", "canonical.rs:", "osis", "bprp:eng:preferred")):
        return 2
    return 3


# ---------------------------------------------------------------- sources


def parse_canonical(text: str) -> tuple[list[tuple[str, int, str]], dict, dict]:
    books = [
        (code, int(number), english)
        for code, number, english in re.findall(
            r'code: "([0-9A-Z]{3})",\s*number: (\d+),\s*english_name: "([^"]+)"', text
        )
    ]
    if len(books) != 66:
        raise GeneratorError(f"canonical.rs: expected 66 CANONICAL_BOOKS, found {len(books)}")
    arrays = {}
    for name in SLOVAK_ARRAYS:
        match = re.search(name + r": \[&str; 66\] = \[(.*?)\];", text, re.S)
        if match is None:
            raise GeneratorError(f"canonical.rs: {name} not found")
        values = re.findall(r'"([^"]*)"', match.group(1))
        if len(values) != 66:
            raise GeneratorError(f"canonical.rs: {name} has {len(values)} entries, expected 66")
        arrays[name] = values
    pairs = {}
    for name in ("ENGLISH_EXTRA_ALIASES", "SLOVAK_ABBREVIATIONS"):
        match = re.search(name + r": &\[\(&str, &str\)\] = &\[(.*?)\];", text, re.S)
        if match is None:
            raise GeneratorError(f"canonical.rs: {name} not found")
        pairs[name] = re.findall(r'\("([^"]*)", "([^"]*)"\)', match.group(1))
    return books, arrays, pairs


def bprp_all(path: Path) -> list[tuple[str, str]]:
    """(osis, text) pairs. Entries listing several books (eng "Jo" = John, Jonah,
    Job, ...) are skipped: the resolver ranks ambiguous short forms itself."""
    rows = yaml.safe_load(path.read_text(encoding="utf-8"))
    return [(row["osis"], str(text)) for row in rows if isinstance(row["osis"], str) for text in row["texts"]]


def bprp_preferred(path: Path) -> list[tuple[str, str]]:
    groups = yaml.safe_load(path.read_text(encoding="utf-8"))
    out = []
    for group in groups.values():
        for osis, names in group.items():
            for value in names.values():
                for text in value if isinstance(value, list) else [value]:
                    out.append((osis, str(text)))
    return out


def english_short(text: str) -> bool:
    """BPRP English forms of at most 5 letters without a doubled letter."""
    letters = re.sub(r"^[1-3I]+\s*\.?\s*", "", text)
    return bool(re.fullmatch(r"[A-Za-z]{1,5}", letters)) and not re.search(r"(.)\1", letters.lower())


def collect_forms(data: Path, canonical_text: str) -> tuple[list[Form], list[str]]:
    books, arrays, pairs = parse_canonical(canonical_text)
    codes = [code for code, _, _ in books]
    osis = dict(zip(OSIS_IDS, codes))
    slugs = dict(zip(BIBLIA_SK_SLUGS, codes))
    forms: list[Form] = []

    def add(code: str, text: str, source: str) -> None:
        text = unicodedata.normalize("NFC", text.strip())
        if text:
            forms.append(Form(code, text, source))

    for code, _, english in books:
        add(code, code, "canonical.rs:code")
        add(code, english, "canonical.rs:english_name")
    for name in SLOVAK_ARRAYS:
        for index, text in enumerate(arrays[name]):
            add(codes[index], text, f"canonical.rs:{name}")
    for name in ("ENGLISH_EXTRA_ALIASES", "SLOVAK_ABBREVIATIONS"):
        for text, code in pairs[name]:
            add(code, text, f"canonical.rs:{name}")
    stored = json.loads((data / "presenter-stored.json").read_text(encoding="utf-8"))
    for translation in STORED_TRANSLATIONS:
        for code, text in stored[translation].items():
            add(code, text, f"db:{translation}")
    for lang in ("slk", "ces"):
        for osis_id, text in bprp_all(data / f"bprp-{lang}.yaml"):
            if osis_id in osis:
                add(osis[osis_id], text, f"bprp:{lang}")
    for osis_id, text in bprp_preferred(data / "bprp-eng-preferred.yaml"):
        if osis_id in osis:
            add(osis[osis_id], text, "bprp:eng:preferred")
    for osis_id, code in osis.items():
        add(code, osis_id, "osis")
    for osis_id, text in bprp_all(data / "bprp-eng.yaml"):
        if osis_id in osis and english_short(text):
            add(osis[osis_id], text, "bprp:eng:short")
    menus = json.loads((data / "biblia-sk.json").read_text(encoding="utf-8"))
    for system, rows in menus.items():
        for slug, abbreviation, title in rows:
            if slug not in slugs:
                continue  # deuterocanonical book, not installed in Presenter
            add(slugs[slug], abbreviation, f"biblia.sk:{system}:abbr")
            add(slugs[slug], title, f"biblia.sk:{system}:title")
            for part in re.split(r"\s+[—-]\s+|\(|\)", title):
                if part.strip() and part.strip() != title:
                    add(slugs[slug], part.strip(), f"biblia.sk:{system}:title-part")
    return forms, codes


# ---------------------------------------------------------------- table


def build_table(forms: list[Form], codes: list[str], curated: dict) -> tuple[list[Row], list[tuple[str, int]]]:
    index = {code: i for i, code in enumerate(codes)}
    for entry in curated.get("add", []):
        forms = forms + [Form(entry["code"], entry["form"], f"curated:tier{entry['tier']}")]
    removed = {(entry["form"], entry["code"]) for entry in curated.get("remove", [])}
    no_fold = {entry["form"]: entry["code"] for entry in curated.get("no_fold", [])}
    best: dict[tuple[str, int, int], tuple[int, int, str]] = {}
    for order, form in enumerate(forms):
        if form.code not in index:
            raise GeneratorError(f"unknown book code {form.code!r} ({form.source})")
        if (form.text, form.code) in removed or form.text in no_fold:
            continue
        ordinal, key = fold(form.text)
        if not key:
            continue
        slot = (key, ordinal, index[form.code])
        candidate = (tier(form.source), order, form.text)
        if slot not in best or candidate < best[slot]:
            best[slot] = candidate
    rows = [Row(key, ordinal, idx, value[0], value[2]) for (key, ordinal, idx), value in best.items()]
    rows.sort(key=lambda row: (row.key, row.ordinal, row.index))
    fold_rows = sorted({(no_fold_key(text), index[code]) for text, code in no_fold.items()})
    present = {row.index for row in rows}
    missing = [codes[i] for i in range(66) if i not in present]
    if missing:
        raise GeneratorError(f"books without any form: {missing}")
    return rows, fold_rows


def matches(rows: list[Row], ordinal: int, key: str) -> set[int]:
    """Book indexes whose rows start with ``key`` (exact or prefix) for ``ordinal``."""
    found = set()
    for row in rows:
        if row.key.startswith(key) and (ordinal == 0 or row.ordinal == ordinal):
            found.add(row.index)
    return found


def short_defaults(curated: dict, rows: list[Row], codes: list[str]) -> list[tuple[str, list[int]]]:
    index = {code: i for i, code in enumerate(codes)}
    out = []
    for full_key, book_codes in curated["short_defaults"].items():
        full_key = str(full_key)
        match = re.fullmatch(r"([1-5]?)([a-z]+)", full_key)
        if match is None or len(match.group(2)) > SHORT_KEY_MAX:
            raise GeneratorError(f"short_defaults key {full_key!r} is not <=2 folded letters")
        ordinal = int(match.group(1) or 0)
        reachable = matches(rows, ordinal, match.group(2))
        books = []
        for code in book_codes:
            if code not in index:
                raise GeneratorError(f"short_defaults {full_key!r}: unknown code {code!r}")
            if index[code] not in reachable:
                raise GeneratorError(f"short_defaults {full_key!r}: {code} matches no form of that key")
            books.append(index[code])
        out.append((full_key, books))
    return sorted(out)


def frequency(curated: dict, codes: list[str]) -> list[int]:
    ranks = curated["frequency_rank"]
    if sorted(ranks) != sorted(codes) or sorted(ranks.values()) != list(range(1, 67)):
        raise GeneratorError("frequency_rank must rank each of the 66 codes exactly once, 1..66")
    return [ranks[code] for code in codes]


# ---------------------------------------------------------------- render


def rust_str(text: str) -> str:
    if any(ord(c) < 0x20 for c in text):
        raise GeneratorError(f"control character in form {text!r}")
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def packed_lines(values: list[str], per_line: int) -> list[str]:
    return ["    " + ", ".join(values[i:i + per_line]) + "," for i in range(0, len(values), per_line)]


def render(rows, fold_rows, shorts, ranks, codes) -> str:
    lines = [
        HEADER,
        "//! Book-name keys of the Bible reference resolver (#850 spec §5.2),",
        "//! generated from `data/bible-book-names/` and the Slovak arrays of",
        "//! `bible/canonical.rs`. CI runs the generator with `--check`.",
        "",
        "/// USFM code per book index (book number - 1).",
        "pub static BOOK_CODES: [&str; 66] = [",
        *packed_lines([rust_str(c) for c in codes], 11),
        "];",
        "",
        f"/// `(folded key, form as written, book index, tier << 4 | ordinal)`, {len(rows)} rows",
        "/// sorted by key, then ordinal (0 = none typed), then book index.",
        "pub static BOOK_KEYS: &[(&str, &str, u8, u8)] = &[",
    ]
    for row in rows:
        lines.append(f"    ({rust_str(row.key)}, {rust_str(row.display)}, {row.index}, 0x{(row.tier << 4) | row.ordinal:02x}),")
    lines += [
        "];",
        "",
        "/// Forms matched before folding (lowercase, no dots or spaces) -> book index.",
        "pub static NO_FOLD: &[(&str, u8)] = &[",
        *[f"    ({rust_str(key)}, {idx})," for key, idx in fold_rows],
        "];",
        "",
        "/// Keys of at most two letters (ordinal digit + letters) -> book indexes, best first.",
        "pub static SHORT_DEFAULTS: &[(&str, &[u8])] = &[",
        *[f"    ({rust_str(key)}, &[{', '.join(str(i) for i in books)}])," for key, books in shorts],
        "];",
        "",
        "/// OpenBible.info cross-reference rank per book index (1 = most cited).",
        "pub static FREQUENCY_RANK: [u8; 66] = [",
        *packed_lines([str(r) for r in ranks], 11),
        "];",
        "",
    ]
    return "\n".join(lines)


def generate(data: Path = DATA, canonical: Path = CANONICAL) -> str:
    curated = yaml.safe_load((data / "curated.yaml").read_text(encoding="utf-8"))
    forms, codes = collect_forms(data, canonical.read_text(encoding="utf-8"))
    rows, fold_rows = build_table(forms, codes, curated)
    return render(rows, fold_rows, short_defaults(curated, rows, codes), frequency(curated, codes), codes)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail when the committed table is stale")
    parser.add_argument("--data", type=Path, default=DATA)
    parser.add_argument("--canonical", type=Path, default=CANONICAL)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args(argv)
    try:
        text = generate(args.data, args.canonical)
    except (GeneratorError, OSError, KeyError, yaml.YAMLError) as err:
        print(f"::error::gen_bible_book_names: {err}", file=sys.stderr)
        return 1
    if args.check:
        current = args.output.read_text(encoding="utf-8") if args.output.exists() else ""
        if current == text:
            print(f"{args.output.name} is up to date")
            return 0
        diff = difflib.unified_diff(current.splitlines(), text.splitlines(), "committed", "generated", lineterm="", n=1)
        print("::error::names_table.rs is stale — run python3 scripts/dev/gen_bible_book_names.py", file=sys.stderr)
        print("\n".join(list(diff)[:40]), file=sys.stderr)
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text, encoding="utf-8")
    print(f"wrote {args.output} ({text.count(chr(10))} lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Generate and check — GREEN**

```bash
cd "$REPO"
python3 scripts/dev/gen_bible_book_names.py          # wrote …/names_table.rs (827 lines)
python3 scripts/dev/gen_bible_book_names.py --check  # names_table.rs is up to date
python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' -v   # Ran 11 tests … OK
head -20 crates/presenter-core/src/bible/resolver/names_table.rs
grep -n '("zid"\|("ab"\|("mr"\|("filimonovi"' crates/presenter-core/src/bible/resolver/names_table.rs
```

Expected rows: `("ab", "Ab", 34, 0x10)`, `("filimonovi", "Filimonovi", 56, 0x10)`, `("mr", "Mr", 40, 0x10)`, `("zid", "Žid", 57, 0x10)`; the `BOOK_KEYS` doc line says `768 rows`. 827 lines = over the 800 warning, under the 1000 cap (a warning, not a failure).

- [ ] **Step 5: CI step** — in `pipeline.yml`, directly after the Task 4 step:

```yaml

      # #850 A1a: the Bible resolver's book-name table is generated from
      # data/bible-book-names/ — a stale committed table fails here.
      - name: Check generated Bible book-name table (#850)
        run: |
          set -euo pipefail
          python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' -v
          python3 scripts/dev/gen_bible_book_names.py --check
```

(`quality` installs `python3-yaml`, `pipeline.yml:417`.) Check: `python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" .github/workflows/pipeline.yml`.

- [ ] **Step 6: Commit** — paths `scripts/dev/gen_bible_book_names.py scripts/dev/test_gen_bible_book_names.py crates/presenter-core/src/bible/resolver/testdata/fold-cases.json crates/presenter-core/src/bible/resolver/names_table.rs .github/workflows/pipeline.yml`, message:

```text
feat(bible): generate the resolver's book-name table from committed sources (#850 A1a)

scripts/dev/gen_bible_book_names.py merges canonical.rs, the stored names,
BPRP slk/ces/eng and the biblia.sk menus, folds every form, keeps the best
tier per (key, ordinal, book) with its display form, validates the curated
short defaults and writes names_table.rs (768 keys). --check runs in the
CI quality job next to its unittest; fold-cases.json is shared with the
Rust fold tests.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 10: The Rust fold, the resolver module and the table tests

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/mod.rs`, `crates/presenter-core/src/bible/resolver/fold.rs`
- Create: `crates/presenter-core/src/bible/resolver/tests/mod.rs`, `crates/presenter-core/src/bible/resolver/tests/table.rs` (`tests/*.rs` is file-size exempt; same layout as `bible/tests/`)
- Modify: `crates/presenter-core/src/bible/mod.rs:3-4` (`pub mod resolver;` between `mod reference;` and `pub mod search;`)

**Interfaces:**
- Consumes: `names_table` (Task 9); `unicode_normalization::{char::is_combining_mark, UnicodeNormalization}` (already a dependency, `presenter-core/Cargo.toml`).
- Produces: `pub fn fold_book_text(input: &str) -> FoldedBook`, `pub struct FoldedBook { pub ordinal: Option<u8>, pub key: String }`, `pub(crate) fn strip_marks(input: &str) -> String` (used by the grammar in Task 14), `pub mod names_table` (public data — reachable, so no `dead_code` before A1b uses it).

- [ ] **Step 1: Write the failing tests.** `resolver/tests/mod.rs`:

```rust
//! Resolver tests (#850 spec §10). Fixtures live in `../testdata/`.

mod table;
```

`resolver/tests/table.rs`:

```rust
//! The fold and the generated name table (#850 spec §5.2, §5.3, §10).

use serde::Deserialize;

use crate::bible::resolver::names_table::{
    BOOK_CODES, BOOK_KEYS, FREQUENCY_RANK, NO_FOLD, SHORT_DEFAULTS,
};
use crate::bible::resolver::{fold_book_text, FoldedBook};

#[derive(Deserialize)]
struct FoldCases {
    cases: Vec<FoldCase>,
}

#[derive(Deserialize)]
struct FoldCase {
    input: String,
    ordinal: u8,
    key: String,
}

fn row_codes(key: &str, ordinal: u8) -> Vec<&'static str> {
    BOOK_KEYS
        .iter()
        .filter(|row| row.0 == key && row.3 & 0x0F == ordinal)
        .map(|row| BOOK_CODES[usize::from(row.2)])
        .collect()
}

#[test]
fn fold_matches_the_cases_shared_with_the_generator() {
    let cases: FoldCases = serde_json::from_str(include_str!("../testdata/fold-cases.json"))
        .expect("fold cases parse");
    assert!(cases.cases.len() >= 40);
    for case in cases.cases {
        let expected = FoldedBook {
            ordinal: (case.ordinal != 0).then_some(case.ordinal),
            key: case.key.clone(),
        };
        assert_eq!(
            fold_book_text(&case.input),
            expected,
            "fold({:?})",
            case.input
        );
    }
}

#[test]
fn every_generated_row_folds_back_to_its_key() {
    for &(key, form, index, packed) in BOOK_KEYS {
        let folded = fold_book_text(form);
        assert_eq!(folded.key, key, "{form:?} (book {index})");
        assert_eq!(
            folded.ordinal.unwrap_or(0),
            packed & 0x0F,
            "{form:?} ordinal"
        );
    }
}

#[test]
fn table_covers_66_distinct_codes_with_valid_rows() {
    let mut codes = BOOK_CODES.to_vec();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 66);
    let mut seen = [false; 66];
    for &(_, _, index, packed) in BOOK_KEYS {
        assert!(usize::from(index) < 66);
        assert!((1..=3).contains(&(packed >> 4)), "tier of {packed:#x}");
        assert!(packed & 0x0F <= 5, "ordinal of {packed:#x}");
        seen[usize::from(index)] = true;
    }
    assert!(seen.iter().all(|book| *book), "every book has a key");
}

#[test]
fn table_rows_are_sorted_without_duplicate_key_ordinal_code() {
    for pair in BOOK_KEYS.windows(2) {
        let a = (pair[0].0, pair[0].3 & 0x0F, pair[0].2);
        let b = (pair[1].0, pair[1].3 & 0x0F, pair[1].2);
        assert!(a < b, "{a:?} then {b:?}");
    }
}

#[test]
fn short_defaults_and_frequency_rank_are_valid() {
    for pair in SHORT_DEFAULTS.windows(2) {
        assert!(pair[0].0 < pair[1].0, "SHORT_DEFAULTS sorted");
    }
    for &(key, books) in SHORT_DEFAULTS {
        let letters = key.trim_start_matches(|c: char| c.is_ascii_digit());
        assert!(!letters.is_empty() && letters.len() <= 2, "{key}");
        assert!(
            !books.is_empty() && books.iter().all(|b| usize::from(*b) < 66),
            "{key}"
        );
    }
    let mut ranks = FREQUENCY_RANK.to_vec();
    ranks.sort_unstable();
    assert_eq!(ranks, (1..=66).collect::<Vec<u8>>());
}

#[test]
fn table_carries_the_data_fixes() {
    assert_eq!(row_codes("ab", 0), ["HAB"], "Ab is Habakkuk, never Obadiah");
    assert_eq!(
        row_codes("ps", 0),
        ["PSA"],
        "P.Š. must not fold into Psalms"
    );
    assert!(row_codes("mr", 0).contains(&"MRK"));
    assert!(row_codes("filimonovi", 0).contains(&"PHM"));
    assert!(row_codes("zid", 0).contains(&"HEB"));
    assert_eq!(NO_FOLD, [("pš", 21_u8)].as_slice());
}

#[test]
fn slovak_1k_and_2k_beat_english_kings() {
    let tier_of = |index: u8| {
        BOOK_KEYS
            .iter()
            .find(|row| row.0 == "k" && row.3 & 0x0F == 1 && row.2 == index)
            .map(|row| row.3 >> 4)
    };
    assert_eq!(tier_of(45), Some(1), "1K = 1 Korinťanom (SEP), tier 1");
    assert_eq!(
        tier_of(10),
        Some(3),
        "1 K = 1 Kings only in BPRP English, tier 3"
    );
}
```

- [ ] **Step 2: How it is verified.** CI `test` job. `every_generated_row_folds_back_to_its_key` re-folds all 768 display forms with the Rust fold — the Python↔Rust parity guard (Review Focus 5).

- [ ] **Step 3: Implement.** `resolver/fold.rs`:

```rust
//! Book-text folding (#850 spec §5.3) — the Rust twin of `fold()` in
//! `scripts/dev/gen_bible_book_names.py`. Both must fold every form the same
//! way: `tests.rs` re-folds each generated row's display form and runs the
//! shared `testdata/fold-cases.json`, which the Python unittest runs too.

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// A typed or stored book name reduced to an ordinal and a lookup key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedBook {
    /// The leading book number (`1 Kor`, `I.`, `prvý`, `1 k.`), 1–5.
    pub ordinal: Option<u8>,
    /// The name without diacritics, filler words and punctuation (`[a-z0-9]`).
    pub key: String,
}

/// Spelled ordinals, tried in this order (`tretia` before `treti`).
const ORDINAL_WORDS: &[(&str, u8)] = &[
    ("prva", 1),
    ("prvy", 1),
    ("prvni", 1),
    ("prve", 1),
    ("druha", 2),
    ("druhy", 2),
    ("druhe", 2),
    ("tretia", 3),
    ("treti", 3),
    ("treta", 3),
    ("stvrta", 4),
    ("ctvrta", 4),
    ("piata", 5),
    ("pata", 5),
];

/// Roman ordinals; each needs a `.` or a space after it (`I Kor`, `II. Kor`).
const ROMAN_ORDINALS: &[(&str, u8)] = &[("iii", 3), ("ii", 2), ("iv", 4), ("i", 1)];

/// Words dropped from a book name (`Evanjelium podľa Jána` → `jana`).
const FILLER_WORDS: &[&str] = &[
    "kniha",
    "knihy",
    "list",
    "epistola",
    "cize",
    "evanjelium",
    "evangelium",
    "podla",
    "podle",
    "svateho",
    "svatych",
    "apostola",
    "apostolov",
    "pavla",
    "proroka",
    "prorok",
    "of",
    "the",
    "book",
    "gospel",
    "according",
    "to",
    "saint",
    "st",
];

/// Fold a book name: NFD without marks, lowercase, leading ordinal split off,
/// filler words dropped, then only `[a-z0-9]` kept.
pub fn fold_book_text(input: &str) -> FoldedBook {
    let lowered: String = strip_marks(input)
        .trim()
        .chars()
        .map(|c| {
            if ('\u{2010}'..='\u{2015}').contains(&c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let (ordinal, rest) = split_ordinal(&lowered);
    let key = drop_fillers(rest)
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    FoldedBook { ordinal, key }
}

/// NFD, combining marks dropped, lowercase.
pub(crate) fn strip_marks(input: &str) -> String {
    input
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn split_ordinal(text: &str) -> (Option<u8>, &str) {
    match ordinal_word(text)
        .or_else(|| roman_ordinal(text))
        .or_else(|| digit_ordinal(text))
    {
        Some((ordinal, rest)) => (Some(ordinal), rest),
        None => (None, text),
    }
}

/// `prvá`, `druhý list`, `tretia kniha` … (the optional book word is consumed).
fn ordinal_word(text: &str) -> Option<(u8, &str)> {
    let (rest, ordinal) = ORDINAL_WORDS
        .iter()
        .find_map(|&(word, n)| text.strip_prefix(word).map(|rest| (rest, n)))?;
    let book_words: &[&str] = if ordinal <= 3 {
        &["kniha", "list", "epistola"]
    } else {
        &["kniha"]
    };
    let after_space = rest.trim_start();
    let rest = if after_space.len() < rest.len() {
        book_words
            .iter()
            .find_map(|word| after_space.strip_prefix(word))
            .unwrap_or(rest)
    } else {
        rest
    };
    Some((ordinal, rest.trim_start()))
}

/// `I.`, `II `, `iv.` … and `v.` (five needs the dot).
fn roman_ordinal(text: &str) -> Option<(u8, &str)> {
    for &(numeral, ordinal) in ROMAN_ORDINALS {
        if let Some(rest) = text.strip_prefix(numeral) {
            let mut chars = rest.chars();
            if chars.next().is_some_and(|c| c == '.' || c.is_whitespace()) {
                return Some((ordinal, chars.as_str().trim_start()));
            }
        }
    }
    text.strip_prefix("v.").map(|rest| (5, rest.trim_start()))
}

/// `1`, `1.`, `1 k.` (the "kniha" abbreviation needs a space after it).
fn digit_ordinal(text: &str) -> Option<(u8, &str)> {
    let mut chars = text.chars();
    let digit = chars.next()?.to_digit(10)?;
    let ordinal = u8::try_from(digit).ok().filter(|d| (1..=5).contains(d))?;
    let after = chars.as_str().trim_start();
    let after = after.strip_prefix('.').unwrap_or(after).trim_start();
    if let Some(rest) = after.strip_prefix('k') {
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        if rest.starts_with(char::is_whitespace) {
            return Some((ordinal, rest));
        }
    }
    Some((ordinal, after))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Drop whole filler words and a standalone `k` / `k.` followed by a space.
fn drop_fillers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if !(FILLER_WORDS.contains(&word.as_str()) || is_kniha_abbreviation(&chars, start, i)) {
            out.push_str(&word);
        }
        out.push(' ');
    }
    out
}

/// `k` / `k.` with whitespace (or the start) before it and whitespace after it.
fn is_kniha_abbreviation(chars: &[char], start: usize, end: usize) -> bool {
    if end - start != 1 || chars[start] != 'k' {
        return false;
    }
    let before_ok = start == 0 || chars[start - 1].is_whitespace();
    let after = if chars.get(end) == Some(&'.') {
        end + 1
    } else {
        end
    };
    before_ok && chars.get(after).is_some_and(|c| c.is_whitespace())
}
```

`resolver/mod.rs` (A1a version; `#[rustfmt::skip]` leaves the generated file's layout to the generator):

```rust
//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5). This first part holds the book-text fold and the
//! generated name table (`scripts/dev/gen_bible_book_names.py`); the grammar,
//! ranking and versification follow in A1b. The importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
#[rustfmt::skip]
pub mod names_table;

pub use fold::{fold_book_text, FoldedBook};

#[cfg(test)]
mod tests;
```

`bible/mod.rs` lines 1-5 become:

```rust
mod canonical;
mod presentation;
mod reference;
pub mod resolver;
pub mod search;
mod translation;
```

- [ ] **Step 4: Local gates**

```bash
cd "$REPO"
cargo fmt --all -- --check
python3 scripts/dev/gen_bible_book_names.py --check      # the skip keeps rustfmt off the table
QC_TARGETS=$'crates/presenter-core/src/bible/resolver/fold.rs\ncrates/presenter-core/src/bible/resolver/mod.rs' python3 scripts/dev/fn_length_check.py .
bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/bible/resolver/fold.rs          # 198
bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/bible/resolver/names_table.rs   # 827 (warn, < 1000)
```

- [ ] **Step 5: Commit** — paths `crates/presenter-core/src/bible/mod.rs crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/fold.rs crates/presenter-core/src/bible/resolver/tests`, message:

```text
feat(bible): resolver module with the Rust book-text fold and the table (#850 A1a)

bible::resolver holds fold_book_text — NFD without marks, the leading
ordinal (1, 1., I., prvý, 1 k.), filler words, [a-z0-9] — and the generated
names_table. Tests pin the fold cases shared with the generator, re-fold
every generated row, and check the table invariants and data fixes.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 11: The resolver playbook rule, then ship A1a

**Files:**
- Create: `.claude/rules/bible-resolver.md`
- Modify: `CLAUDE.md` (one Playbook Router row after the two rows Task 5 added)

**Interfaces:** none.

- [ ] **Step 1: Write the rule** — `.claude/rules/bible-resolver.md`:

```markdown
---
paths:
  - "crates/presenter-core/src/bible/resolver/**"
  - "crates/presenter-core/src/bible/canonical.rs"
  - "scripts/dev/gen_bible_book_names.py"
  - "scripts/dev/test_gen_bible_book_names.py"
  - "data/bible-book-names/**"
---

# Bible reference resolver (#850 part A)

## The name table is generated — never hand-edit `names_table.rs`

- Edit a source under `data/bible-book-names/` (the curated rules live in `curated.yaml`) or a
  Slovak array in `canonical.rs`, then run `python3 scripts/dev/gen_bible_book_names.py` and commit
  both. CI's `quality` job runs the generator's unittest and `--check`; a stale table fails it.
- `resolver/mod.rs` declares `#[rustfmt::skip] pub mod names_table;` — the generator owns the
  file's layout, so `cargo fmt --check` and `--check` never fight. Keep that attribute.
- Rows are `(folded key, form as written, book index, tier << 4 | ordinal)`, sorted by key, then
  ordinal, then index. They are unique per **(key, ordinal, book)**; the same (key, book) repeats
  across ordinals on purpose (`kralov` → 1KI with and without `1`).
- `canonical_book_by_name` (the importer, `import.rs:190`) stays exact. The fuzzy resolver must
  never assign codes at ingest.

## Two folds, one behaviour

`fold()` (Python, builds the keys) and `fold_book_text` (Rust, folds what the operator types)
must agree on every form. Two guards: `resolver/testdata/fold-cases.json` runs through BOTH
(`test_shared_fold_cases` and `fold_matches_the_cases_shared_with_the_generator`), and
`every_generated_row_folds_back_to_its_key` re-folds each row's display form in Rust. Change both
folds in the same commit and add the new case to `fold-cases.json`.

## Data decisions (2026-10-13)

`Ab` = Habakkuk (removed from canonical.rs AND listed under `curated.yaml: remove`); `P.Š.` / `PŠ`
= Song of Songs, matched before folding (`no_fold`); `Mr` = Mark at tier 1 (`add`); SEB's stored
misspelling `Filimonovi` stays a key. BPRP entries that list several books (eng `Jo` = John,
Jonah, Job …) are skipped by the generator.
```

CLAUDE.md row:

```markdown
| Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr) | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
```

Commit — paths `.claude/rules/bible-resolver.md CLAUDE.md`, message `docs(playbook): Bible resolver rule — generated table, fold parity, data decisions (#850 A1a) [no-test: docs only]` + body line + Co-Authored-By.

- [ ] **Step 2: Unit gates + push.** Task 5 Step 2 commands plus `python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' && python3 scripts/dev/gen_bible_book_names.py --check`; push `dev`; monitor `pipeline.yml` to terminal (CI compiles the fold + table tests for the first time — on a red Clippy/Test job read `--log-failed`, fix in one commit).

- [ ] **Step 3: Release PR** `Release v0.4.317: book-name table and fold for the Bible reference resolver (#850 A1a)`, merge when clean, verify SNV (version 0.4.317 in `/healthz` and the footer DOM; no behaviour change expected), `gh release create v0.4.317 --target main --title v0.4.317 --generate-notes`, verify PP. `playbook-review`, update #850.

---

## PR unit A1b — the reference resolver (v0.4.318)

Starts after the A1a release (main = 0.4.317 = dev). Module wiring grows task by task; Rust compiles only at the unit's push (Task 18), so intermediate commits may carry `dead_code` warnings for `pub(crate)` items that Task 17 wires into the API.

### Task 12: Post-release version bump to 0.4.318

**Files:** Modify `Cargo.toml:15`, `Cargo.lock`.

**Interfaces:** none.

- [ ] **Step 1:** Run Task 6 Steps 1-2 again (expected `main=0.4.317 dev=0.4.317` → `NEXT=0.4.318`; STOP if A1a is not released).
- [ ] **Step 2: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to <NEXT> [no-test: version bump only]` (0.4.318 expected) + the Co-Authored-By line.

### Task 13: Resolver types and Slovak hints (`counted` moves to core)

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/types.rs`, `crates/presenter-core/src/bible/resolver/hint.rs`, `crates/presenter-core/src/bible/resolver/tests/hint.rs`
- Modify: `crates/presenter-core/src/bible/resolver/mod.rs` (whole file, below)
- Modify: `crates/presenter-core/src/bible/resolver/tests/mod.rs` (`mod hint;` above `mod table;`)
- Modify: `crates/presenter-ui/src/state/bible_range.rs:4` (import) and `:140-148` (delete the private `counted`)

**Interfaces:**
- Produces: `pub enum Tier { T1 = 1, T2 = 2, T3 = 3 }`, `pub enum MatchKind { Exact, OrdinalStripped, Prefix, Fuzzy }` (both `Ord`, lower wins), `pub struct BookCandidate { pub code: String, pub number: u16, pub tier: Tier, pub kind: MatchKind, pub penalty: u8, pub matched_form: String }`, `pub enum RawSpec { Chapter(u16), ChapterRange { start, end }, Verses { chapter, ranges: Vec<(u16, u16)> }, CrossChapter { chapter, verse_start, end_chapter, verse_end } }` + `pub fn unbound_segments(&self) -> Vec<Segment>`, `pub struct ParsedQuery { pub input, pub book_text, pub ordinal: Option<u8>, pub key, pub bare_ordinal: Option<u8>, pub candidates: Vec<BookCandidate>, pub spec: Option<RawSpec>, pub leftover }`, `pub struct Segment { pub chapter: u16, pub verse_start: Option<u16>, pub verse_end: Option<u16> }` + `pub const fn whole(chapter) -> Self`, `pub const fn verses(chapter, start, end) -> Self`, `pub struct BookRef { pub code, pub number: u16, pub name }`, `pub struct Passage { pub book: BookRef, pub matched_form: String, pub segments: Vec<Segment> }`, `pub enum Resolution { Passage { passage, alternatives: Vec<Passage> }, Ambiguous { default, alternatives }, OutOfRange { book: BookRef, requested: Segment, hint: String, fix: Option<Passage> }, BookOnly { book, alternatives: Vec<BookRef> }, Incomplete { ordinal: u8, books: Vec<BookRef> }, NoBook { segments: Vec<Segment>, hint: Option<String> } }` (spec §5.1; `RawSpec` and `ParsedQuery::bare_ordinal` are additions), `pub const NO_BOOK_HINT: &str = "Napíš aj knihu"`, `pub const MISSING_BOOK_HINT: &str`, `pub fn counted(n: u16, one: &str, few: &str, many: &str) -> String`, `pub fn chapters_hint(chapter_count: u16) -> String`, `pub fn verses_hint(verse_count: u16) -> String`, `pub fn segment_label(book_name: &str, segment: &Segment) -> String`.
- Consumed by presenter-ui: `presenter_core::bible::resolver::counted` (same signature as the removed private fn, `bible_range.rs:141`).

- [ ] **Step 1: Write the failing tests** — `resolver/tests/hint.rs`:

```rust
//! Slovak hints and labels (#850 spec §5.6).

use crate::bible::resolver::{chapters_hint, counted, segment_label, verses_hint, Segment};

#[test]
fn slovak_counted_forms() {
    assert_eq!(counted(1, "kapitolu", "kapitoly", "kapitol"), "1 kapitolu");
    assert_eq!(counted(3, "kapitolu", "kapitoly", "kapitol"), "3 kapitoly");
    assert_eq!(counted(5, "kapitolu", "kapitoly", "kapitol"), "5 kapitol");
    assert_eq!(
        counted(150, "kapitolu", "kapitoly", "kapitol"),
        "150 kapitol"
    );
    assert_eq!(chapters_hint(13), "Kniha má len 13 kapitol");
    assert_eq!(chapters_hint(1), "Kniha má len 1 kapitolu");
    assert_eq!(verses_hint(40), "Kapitola má len 40 veršov");
    assert_eq!(verses_hint(4), "Kapitola má len 4 verše");
}

#[test]
fn segment_labels() {
    assert_eq!(segment_label("Žalmy", &Segment::whole(23)), "Žalmy 23");
    assert_eq!(
        segment_label("Hebrejom", &Segment::verses(11, 1, 1)),
        "Hebrejom 11:1"
    );
    assert_eq!(
        segment_label("Ján", &Segment::verses(3, 16, 18)),
        "Ján 3:16-18"
    );
}
```

and add `mod hint;` to `resolver/tests/mod.rs` (above `mod table;`).

- [ ] **Step 2: How it is verified.** CI `test` job (presenter-core) and the UI host tests of `bible_range.rs` (`a_chapter_past_the_last_one_is_clamped_with_a_hint` etc., unchanged) plus the `bible-range-hint.spec.ts` E2E.

- [ ] **Step 3: Implement.** `resolver/types.rs`:

```rust
//! Resolver data types (#850 spec §5.1).

/// The source family a book form comes from (spec §5.5); lower wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Slovak: stored names, the four biblia.sk systems, the canonical Slovak lists.
    T1 = 1,
    /// BPRP Slovak, English names, OSIS ids.
    T2 = 2,
    /// Czech and BPRP English short forms.
    T3 = 3,
}

/// How a typed key met a book form; lower wins (spec §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    Exact,
    /// `kr` = the `1 Kr` family without its number.
    OrdinalStripped,
    Prefix,
    /// Damerau-Levenshtein within 1 (key length 4–6) or 2 (7+).
    Fuzzy,
}

/// One book a typed name may mean, best first in [`ParsedQuery::candidates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookCandidate {
    pub code: String,
    pub number: u16,
    pub tier: Tier,
    pub kind: MatchKind,
    /// 1 when a numbered book matched without a typed ordinal; fuzzy adds the distance.
    pub penalty: u8,
    /// The form that matched, as its source writes it (`Žid`).
    pub matched_form: String,
}

/// The chapter/verse part as typed, before a book's structure is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSpec {
    /// `11`; on a single-chapter book a lone `n > 1` is verse `n`.
    Chapter(u16),
    /// `23-24`.
    ChapterRange { start: u16, end: u16 },
    /// `11:1`, `11:1-3`, `5:3.5.7`, `3:16,18`; each range has `end >= start`.
    Verses {
        chapter: u16,
        ranges: Vec<(u16, u16)>,
    },
    /// `1:1-2:3`.
    CrossChapter {
        chapter: u16,
        verse_start: u16,
        end_chapter: u16,
        verse_end: u16,
    },
}

impl RawSpec {
    /// The segments as typed, without a structure (for `Resolution::NoBook`).
    pub fn unbound_segments(&self) -> Vec<Segment> {
        match self {
            Self::Chapter(chapter) => vec![Segment::whole(*chapter)],
            Self::ChapterRange { start, end } => vec![Segment::whole(*start), Segment::whole(*end)],
            Self::Verses { chapter, ranges } => ranges
                .iter()
                .map(|&(start, end)| Segment::verses(*chapter, start, end))
                .collect(),
            Self::CrossChapter {
                chapter,
                verse_start,
                end_chapter,
                verse_end,
            } => vec![
                Segment {
                    chapter: *chapter,
                    verse_start: Some(*verse_start),
                    verse_end: None,
                },
                Segment::verses(*end_chapter, 1, *verse_end),
            ],
        }
    }
}

/// Pure text analysis of one input: book candidates, the chapter/verse part
/// and the words the grammar did not use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub input: String,
    /// The book part as typed (`zid`, `1 kor`, `I.`).
    pub book_text: String,
    pub ordinal: Option<u8>,
    pub key: String,
    /// A bare ordinal with no book (`1`, `2.`).
    pub bare_ordinal: Option<u8>,
    pub candidates: Vec<BookCandidate>,
    pub spec: Option<RawSpec>,
    /// Text after the last chapter/verse token the grammar used.
    pub leftover: String,
}

/// One contiguous piece of a passage; both verses `None` = the whole chapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub chapter: u16,
    pub verse_start: Option<u16>,
    pub verse_end: Option<u16>,
}

impl Segment {
    pub const fn whole(chapter: u16) -> Self {
        Self {
            chapter,
            verse_start: None,
            verse_end: None,
        }
    }

    pub const fn verses(chapter: u16, start: u16, end: u16) -> Self {
        Self {
            chapter,
            verse_start: Some(start),
            verse_end: Some(end),
        }
    }
}

/// A book as the MAIN translation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRef {
    pub code: String,
    pub number: u16,
    /// The stored name in the main translation (`Hebrejom` on slk-seb).
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub book: BookRef,
    pub matched_form: String,
    pub segments: Vec<Segment>,
}

/// What an input means against the main translation (spec §5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// `alternatives` = other books that fit, best first.
    Passage {
        passage: Passage,
        alternatives: Vec<Passage>,
    },
    /// The default ties on (tier, kind) with another fitting book.
    Ambiguous {
        default: Passage,
        alternatives: Vec<Passage>,
    },
    OutOfRange {
        book: BookRef,
        requested: Segment,
        hint: String,
        fix: Option<Passage>,
    },
    /// A book with no numbers typed.
    BookOnly {
        book: BookRef,
        alternatives: Vec<BookRef>,
    },
    /// Only an ordinal typed (`1`, `I.`, `prvý`).
    Incomplete { ordinal: u8, books: Vec<BookRef> },
    /// `11:1` → hint "Napíš aj knihu"; words only → text search (no hint).
    NoBook {
        segments: Vec<Segment>,
        hint: Option<String>,
    },
}
```

`resolver/hint.rs`:

```rust
//! Slovak hints and labels (#850 spec §5.6) — the wording the Bible page
//! already shows (`presenter-ui` `state/bible_range.rs`) and the corpus expects.

use super::types::Segment;

/// Numbers typed without a book (`11:1`).
pub const NO_BOOK_HINT: &str = "Napíš aj knihu";
/// The typed name means a book the main translation does not have (a partial
/// translation such as slk-mil).
pub const MISSING_BOOK_HINT: &str = "Táto kniha v hlavnom preklade nie je";

/// `n` with the Slovak noun form it takes: 1 → `one`, 2–4 → `few`, else `many`.
pub fn counted(n: u16, one: &str, few: &str, many: &str) -> String {
    let noun = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {noun}")
}

/// "Kniha má len 13 kapitol".
pub fn chapters_hint(chapter_count: u16) -> String {
    format!(
        "Kniha má len {}",
        counted(chapter_count, "kapitolu", "kapitoly", "kapitol")
    )
}

/// "Kapitola má len 40 veršov".
pub fn verses_hint(verse_count: u16) -> String {
    format!(
        "Kapitola má len {}",
        counted(verse_count, "verš", "verše", "veršov")
    )
}

/// `Hebrejom 11:1`, `Hebrejom 11:1-3`, `Hebrejom 13` (whole chapter).
pub fn segment_label(book_name: &str, segment: &Segment) -> String {
    let chapter = segment.chapter;
    match (segment.verse_start, segment.verse_end) {
        (Some(start), Some(end)) if end > start => format!("{book_name} {chapter}:{start}-{end}"),
        (Some(start), _) => format!("{book_name} {chapter}:{start}"),
        (None, _) => format!("{book_name} {chapter}"),
    }
}
```

`resolver/mod.rs` becomes:

```rust
//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5). This first part holds the book-text fold and the
//! generated name table (`scripts/dev/gen_bible_book_names.py`); the grammar,
//! ranking and versification follow in A1b. The importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
mod hint;
#[rustfmt::skip]
pub mod names_table;
mod types;

pub use fold::{fold_book_text, FoldedBook};
pub use hint::{
    chapters_hint, counted, segment_label, verses_hint, MISSING_BOOK_HINT, NO_BOOK_HINT,
};
pub use types::{
    BookCandidate, BookRef, MatchKind, ParsedQuery, Passage, RawSpec, Resolution, Segment, Tier,
};

#[cfg(test)]
mod tests;
```

`presenter-ui/src/state/bible_range.rs`: after line 2 (the module doc) insert

```rust

use presenter_core::bible::resolver::counted;
```

(so the imports read `use presenter_core::bible::resolver::counted;`, blank line, `use super::bible::clamp_selection;`) and delete lines 140-148 (the `/// n with the Slovak noun form…` doc + `fn counted`).

- [ ] **Step 4: Local gates**

```bash
cd "$REPO"
cargo fmt --all -- --check && (cd crates/presenter-ui && cargo fmt --check)
grep -rn 'counted(' crates/presenter-ui/src | grep -v 'state/bible_range.rs'    # nothing
QC_TARGETS=$'crates/presenter-core/src/bible/resolver/types.rs\ncrates/presenter-core/src/bible/resolver/hint.rs' python3 scripts/dev/fn_length_check.py .
```

- [ ] **Step 5: Commit** — paths `crates/presenter-core/src/bible/resolver/types.rs crates/presenter-core/src/bible/resolver/hint.rs crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/tests/hint.rs crates/presenter-core/src/bible/resolver/tests/mod.rs crates/presenter-ui/src/state/bible_range.rs`, message:

```text
feat(bible): resolver types and Slovak hints; counted() moves to core (#850 A1b)

The Resolution kinds of spec §5.1 plus RawSpec (the typed chapter/verse part
before a structure is known). hint.rs owns the existing wording ("Kniha má
len 13 kapitol", "Kapitola má len 40 veršov", "Napíš aj knihu") and
counted(), which presenter-ui's bible_range now imports.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 14: The chapter/verse grammar

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/grammar.rs`, `crates/presenter-core/src/bible/resolver/tests/grammar.rs`
- Modify: `resolver/mod.rs` (`mod grammar;` between `mod fold;` and `mod hint;`), `resolver/tests/mod.rs` (`mod grammar;` above `mod hint;`)

**Interfaces:**
- Consumes: `fold::strip_marks` (Task 10), `types::RawSpec` (Task 13).
- Produces: `pub(crate) struct SplitInput { pub(crate) book_text: String, pub(crate) bare_ordinal: Option<u8>, pub(crate) spec: Option<RawSpec>, pub(crate) leftover: String }`, `pub(crate) fn split_input(input: &str) -> SplitInput`.
- Rules (spec §5.4, ported from `verify_corpus.py` `split`/`parse_spec`): a leading `1`–`5` (spaces, one `.`) before a letter is an ordinal and the book text runs to the next digit (spec says 1–3; SEP's `4M`/`5M` = Numbers/Deuteronomy need 1–5); separator precedence `:` (then `,`/`.` list) → `,` (then `.`) → first `.` → space; `-` `–` `—` `až` `az` are ranges; `n-m` after `:` with `m < n` keeps `n`; `0` reads as `1` (the UI's `bound_chapter` precedent); numbers saturate at `u16::MAX`; extensions over the prototype: trailing separators are ignored, the longest parseable prefix wins and the rest is `leftover`, at most 64 tokens, the book text keeps its dots so `I.` stays a roman ordinal (the prototype stripped them and returned Izaiáš for `I.`), a bare `1`/`2`/`3` (optionally `.`) is `bare_ordinal`.

- [ ] **Step 1: Write the failing tests** — `resolver/tests/grammar.rs`:

```rust
//! The chapter/verse grammar (#850 spec §5.4).

use crate::bible::resolver::grammar::split_input;
use crate::bible::resolver::RawSpec;

fn verses(chapter: u16, ranges: &[(u16, u16)]) -> RawSpec {
    RawSpec::Verses {
        chapter,
        ranges: ranges.to_vec(),
    }
}

#[test]
fn every_separator_and_range_form() {
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 1,
        end_chapter: 2,
        verse_end: 3,
    };
    let cases = [
        ("heb 11:1", verses(11, &[(1, 1)])),
        ("heb 11,1", verses(11, &[(1, 1)])),
        ("heb 11.1", verses(11, &[(1, 1)])),
        ("heb 11 1", verses(11, &[(1, 1)])),
        ("heb11:1", verses(11, &[(1, 1)])),
        ("heb 11 : 1", verses(11, &[(1, 1)])),
        ("heb 11:1-3", verses(11, &[(1, 3)])),
        ("heb 11:1–3", verses(11, &[(1, 3)])),
        ("heb 11:1—3", verses(11, &[(1, 3)])),
        ("heb 11,1 až 3", verses(11, &[(1, 3)])),
        ("heb 11,1 az 3", verses(11, &[(1, 3)])),
        ("mt 5:3.5.7", verses(5, &[(3, 3), (5, 5), (7, 7)])),
        ("jn 3:16,18", verses(3, &[(16, 16), (18, 18)])),
        ("mt 5,3.5", verses(5, &[(3, 3), (5, 5)])),
        ("ef 2 8-9", verses(2, &[(8, 9)])),
        ("1 kor 13 4-7", verses(13, &[(4, 7)])),
        ("heb 11:5-3", verses(11, &[(5, 5)])),
        ("ž 23-24", RawSpec::ChapterRange { start: 23, end: 24 }),
        ("ž 24-23", RawSpec::Chapter(24)),
        ("heb 11", RawSpec::Chapter(11)),
        ("heb 0", RawSpec::Chapter(1)),
        ("gn 1:1-2:3", cross.clone()),
        ("gn 1,1-2,3", cross),
    ];
    for (input, want) in cases {
        assert_eq!(split_input(input).spec, Some(want), "{input}");
    }
}

#[test]
fn the_book_text_ends_at_the_first_digit_after_an_ordinal() {
    for (input, book) in [
        ("1kor13:4", "1kor"),
        ("1. Korinťanom 13,4", "1. Korinťanom"),
        ("I. Jánov 4,8", "I. Jánov"),
        ("Rim.8,25", "Rim."),
        ("4M 6,24", "4M"),
        ("viera je zárukou", "viera je zárukou"),
    ] {
        assert_eq!(split_input(input).book_text, book, "{input}");
    }
    assert_eq!(split_input("1").bare_ordinal, Some(1));
    assert_eq!(split_input("2.").bare_ordinal, Some(2));
    assert_eq!(split_input("11").bare_ordinal, None);
    assert_eq!(split_input("11").spec, Some(RawSpec::Chapter(11)));
}

#[test]
fn trailing_separators_are_ignored_and_unused_text_is_leftover() {
    for (input, want, leftover) in [
        ("zid 11:", RawSpec::Chapter(11), ""),
        ("zid 11:1-", verses(11, &[(1, 1)]), ""),
        ("zid 11:1 až", verses(11, &[(1, 1)]), "až"),
        ("zid 11:1 viera", verses(11, &[(1, 1)]), "viera"),
        ("zid 11,1:3", verses(11, &[(1, 1)]), "3"),
        ("zid 99999999", RawSpec::Chapter(u16::MAX), ""),
    ] {
        let split = split_input(input);
        assert_eq!(split.spec, Some(want), "{input}");
        assert_eq!(split.leftover, leftover, "{input}");
    }
}

#[test]
fn a_long_paste_is_bounded() {
    let split = split_input(&"1,".repeat(5000));
    assert_eq!(split.spec, Some(verses(1, &[(1, 1)])));
    assert!(!split.leftover.is_empty());
}

#[test]
fn unbound_segments_keep_what_was_typed() {
    use crate::bible::resolver::Segment;
    assert_eq!(
        verses(11, &[(1, 1)]).unbound_segments(),
        [Segment::verses(11, 1, 1)]
    );
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 5,
        end_chapter: 2,
        verse_end: 3,
    };
    assert_eq!(
        cross.unbound_segments(),
        [
            Segment {
                chapter: 1,
                verse_start: Some(5),
                verse_end: None
            },
            Segment::verses(2, 1, 3)
        ]
    );
}
```

- [ ] **Step 2: How it is verified.** CI `test` job. Every case was cross-checked with the Python mirror of these rules (`$SCRATCH/plan-probe/mirror.py`, `split_input`/`parse_spec`).

- [ ] **Step 3: Implement** — `resolver/grammar.rs`:

```rust
//! Hand-written reference grammar (#850 spec §5.4), no `regex` dependency.
//!
//! The book text runs to the first digit after an optional leading ordinal
//! (`1kor13:4`). Chapter–verse separator: `:` (then `,` and `.` list verses),
//! else `,` (then `.` lists), else the first `.`, else a space between two
//! numbers. `-`, `–`, `—`, `až`, `az` make ranges. The console resolves on
//! every keystroke, so trailing separators are ignored and text the grammar
//! cannot use is returned as `leftover`.

use std::cmp::Ordering;

use super::fold::strip_marks;
use super::types::RawSpec;

/// Chapter/verse tokens parsed at most; the rest of a long paste is leftover.
const MAX_SPEC_TOKENS: usize = 64;

pub(crate) struct SplitInput {
    pub(crate) book_text: String,
    pub(crate) bare_ordinal: Option<u8>,
    pub(crate) spec: Option<RawSpec>,
    pub(crate) leftover: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Num(u16),
    Colon,
    Comma,
    Dot,
    Dash,
    Space,
    Word,
}

impl Tok {
    const fn is_separator(self) -> bool {
        matches!(self, Self::Colon | Self::Comma | Self::Dot | Self::Dash)
    }
}

pub(crate) fn split_input(input: &str) -> SplitInput {
    let text: String = input.trim().chars().map(normalize_dash).collect();
    let Some(cut) = spec_start(&text) else {
        return SplitInput {
            book_text: text,
            bare_ordinal: None,
            spec: None,
            leftover: String::new(),
        };
    };
    let book_text = text[..cut].trim().to_string();
    let spec_text = &text[cut..];
    let bare_ordinal = if book_text.is_empty() {
        bare_ordinal(spec_text)
    } else {
        None
    };
    if bare_ordinal.is_some() {
        return SplitInput {
            book_text,
            bare_ordinal,
            spec: None,
            leftover: String::new(),
        };
    }
    let (spec, leftover) = parse_spec(spec_text);
    SplitInput {
        book_text,
        bare_ordinal: None,
        spec,
        leftover,
    }
}

fn normalize_dash(c: char) -> char {
    if ('\u{2010}'..='\u{2015}').contains(&c) || c == '\u{2212}' {
        '-'
    } else {
        c
    }
}

/// Byte index of the first digit after an optional leading ordinal.
fn spec_start(text: &str) -> Option<usize> {
    let from = leading_ordinal_end(text).unwrap_or(0);
    text[from..]
        .find(|c: char| c.is_ascii_digit())
        .map(|index| from + index)
}

/// `1`–`5`, optional spaces and one `.`, then a char that is no digit,
/// space or separator: the book text starts with an ordinal (`1. Kor`).
fn leading_ordinal_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !('1'..='5').contains(&first) {
        return None;
    }
    let mut seen_dot = false;
    for (index, c) in chars {
        if c.is_whitespace() {
            continue;
        }
        if c == '.' && !seen_dot {
            seen_dot = true;
            continue;
        }
        return (!c.is_ascii_digit() && !":,.-".contains(c)).then_some(index);
    }
    None
}

/// `1`, `2.`, `3 ` alone: an ordinal waiting for its book.
fn bare_ordinal(spec_text: &str) -> Option<u8> {
    let trimmed = spec_text.trim();
    match trimmed.strip_suffix('.').unwrap_or(trimmed).trim_end() {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        _ => None,
    }
}

/// Tokens of the chapter/verse part and each token's END byte offset.
fn tokenize(text: &str) -> (Vec<Tok>, Vec<usize>) {
    let (mut toks, mut ends) = (Vec::new(), Vec::new());
    let mut chars = text.char_indices().peekable();
    while let Some(&(_, c)) = chars.peek() {
        if toks.len() == MAX_SPEC_TOKENS {
            break;
        }
        let tok = if c.is_ascii_digit() {
            let mut value: u16 = 0;
            while let Some(digit) = chars.peek().and_then(|&(_, d)| d.to_digit(10)) {
                let digit = u16::try_from(digit).unwrap_or(0);
                value = value.saturating_mul(10).saturating_add(digit);
                chars.next();
            }
            Tok::Num(value.max(1))
        } else if c.is_whitespace() {
            while chars.next_if(|&(_, w)| w.is_whitespace()).is_some() {}
            Tok::Space
        } else if c.is_alphabetic() {
            let mut word = String::new();
            while let Some((_, w)) = chars.next_if(|&(_, w)| w.is_alphabetic()) {
                word.push(w);
            }
            if strip_marks(&word) == "az" {
                Tok::Dash
            } else {
                Tok::Word
            }
        } else {
            chars.next();
            match c {
                ':' => Tok::Colon,
                ',' => Tok::Comma,
                '.' => Tok::Dot,
                '-' => Tok::Dash,
                _ => Tok::Word,
            }
        };
        toks.push(tok);
        ends.push(chars.peek().map_or(text.len(), |&(next, _)| next));
    }
    (toks, ends)
}

/// Parse the longest prefix the grammar understands; the rest is leftover.
fn parse_spec(text: &str) -> (Option<RawSpec>, String) {
    let (toks, ends) = tokenize(text);
    let usable = toks
        .iter()
        .position(|t| *t == Tok::Word)
        .unwrap_or(toks.len());
    let (toks, ends) = drop_loose_spaces(&toks[..usable], &ends[..usable]);
    let mut end = toks.len();
    while end > 0 {
        let used = trimmed_len(&toks[..end]);
        if used == 0 {
            break;
        }
        if let Some(spec) = parse_exact(&toks[..used]) {
            let rest = &text[ends[used - 1]..];
            let leftover = rest
                .trim_start_matches(|c: char| c.is_whitespace() || ":,.-".contains(c))
                .trim_end();
            return (Some(spec), leftover.to_string());
        }
        end = used - 1;
    }
    (None, text.trim().to_string())
}

/// Spaces next to a separator do not count (`11 : 1` = `11:1`).
fn drop_loose_spaces(toks: &[Tok], ends: &[usize]) -> (Vec<Tok>, Vec<usize>) {
    let (mut kept, mut kept_ends) = (Vec::new(), Vec::new());
    for (index, (&tok, &end)) in toks.iter().zip(ends).enumerate() {
        let next_is_separator = toks.get(index + 1).is_some_and(|t| t.is_separator());
        let previous_is_separator = kept.last().is_some_and(|t: &Tok| t.is_separator());
        if tok == Tok::Space && (next_is_separator || previous_is_separator) {
            continue;
        }
        kept.push(tok);
        kept_ends.push(end);
    }
    (kept, kept_ends)
}

/// Length without trailing separators and spaces (`11:` → `11`).
fn trimmed_len(toks: &[Tok]) -> usize {
    toks.iter()
        .rposition(|t| matches!(t, Tok::Num(_)))
        .map_or(0, |index| index + 1)
}

fn parse_exact(toks: &[Tok]) -> Option<RawSpec> {
    match toks {
        [Tok::Num(chapter)] => return Some(RawSpec::Chapter(*chapter)),
        [Tok::Num(start), Tok::Dash, Tok::Num(end)] => {
            return Some(if end <= start {
                RawSpec::Chapter(*start)
            } else {
                RawSpec::ChapterRange {
                    start: *start,
                    end: *end,
                }
            });
        }
        _ => {}
    }
    let separator = [Tok::Colon, Tok::Comma, Tok::Dot, Tok::Space]
        .into_iter()
        .find(|sep| toks.contains(sep))?;
    let [Tok::Num(chapter), sep, rest @ ..] = toks else {
        return None;
    };
    if *sep != separator || rest.is_empty() {
        return None;
    }
    if let [Tok::Num(verse_start), Tok::Dash, Tok::Num(end_chapter), sep_end, Tok::Num(verse_end)] =
        rest
    {
        if *sep_end == separator {
            return Some(cross_chapter(
                *chapter,
                *verse_start,
                *end_chapter,
                *verse_end,
            ));
        }
    }
    let list_separators: &[Tok] = match separator {
        Tok::Colon => &[Tok::Comma, Tok::Dot],
        Tok::Space => &[Tok::Space],
        _ => &[Tok::Dot],
    };
    let ranges = rest
        .split(|t| list_separators.contains(t))
        .map(verse_range)
        .collect::<Option<Vec<_>>>()?;
    Some(RawSpec::Verses {
        chapter: *chapter,
        ranges,
    })
}

/// `16` or `16-18`; a reversed range keeps only its start (`5-3` = `5`).
fn verse_range(item: &[Tok]) -> Option<(u16, u16)> {
    match item {
        [Tok::Num(verse)] => Some((*verse, *verse)),
        [Tok::Num(start), Tok::Dash, Tok::Num(end)] => Some((*start, (*end).max(*start))),
        _ => None,
    }
}

fn cross_chapter(chapter: u16, verse_start: u16, end_chapter: u16, verse_end: u16) -> RawSpec {
    match end_chapter.cmp(&chapter) {
        Ordering::Less => RawSpec::Verses {
            chapter,
            ranges: vec![(verse_start, verse_start)],
        },
        Ordering::Equal => RawSpec::Verses {
            chapter,
            ranges: vec![(verse_start, verse_end.max(verse_start))],
        },
        Ordering::Greater => RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        },
    }
}
```

- [ ] **Step 4: Local gates** — `cargo fmt --all -- --check`; `QC_TARGETS=crates/presenter-core/src/bible/resolver/grammar.rs python3 scripts/dev/fn_length_check.py .` (no function above 80); `bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/bible/resolver/grammar.rs` (295).

- [ ] **Step 5: Commit** — paths `crates/presenter-core/src/bible/resolver/grammar.rs crates/presenter-core/src/bible/resolver/tests/grammar.rs crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/tests/mod.rs`, message:

```text
feat(bible): hand-written chapter/verse grammar for the resolver (#850 A1b)

Book/number split after an optional ordinal, the EU separator precedence
(: then , then . then a space), ranges with - – — až az, lists,
cross-chapter and chapter ranges. Input typed half-way (trailing ':' or
'-') still parses; words after the numbers become leftover; at most 64
tokens are read from a long paste.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 15: Candidate ranking

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/rank.rs`, `crates/presenter-core/src/bible/resolver/tests/rank.rs`
- Modify: `resolver/mod.rs` (`mod rank;` between `pub mod names_table;` and `mod types;`), `resolver/tests/mod.rs` (`mod rank;` above `mod table;`)

**Interfaces:**
- Consumes: `names_table::{BOOK_CODES, BOOK_KEYS, FREQUENCY_RANK, NO_FOLD, SHORT_DEFAULTS}`, `fold::FoldedBook`, `types::{BookCandidate, MatchKind, Tier}`.
- Produces: `pub(crate) struct StoredName { pub(crate) index: u8, pub(crate) ordinal: u8, pub(crate) key: String, pub(crate) form: String }` (built by `Versification::new` in Task 16), `pub(crate) fn rank(folded: &FoldedBook, stored: &[StoredName]) -> Vec<BookCandidate>`, `pub(crate) fn no_fold_candidate(book_text: &str) -> Option<BookCandidate>`, `pub(crate) fn index_of(code: &str) -> Option<u8>`, `pub(crate) fn osa_distance(a: &[u8], b: &[u8]) -> usize`.
- Rules (spec §5.5, ported from `collisions.py` `score_candidates`/`order`): per book the best `(tier, kind, penalty)` over every form whose key starts with the typed key (ordinal must match when typed; penalty 1 for a numbered book matched without one; a one-letter key without ordinal is always `Prefix`); fuzzy only when nothing matched and the key has ≥ 4 letters — tier 1–2 forms of ≥ 4 letters, OSA distance to the whole form and its same-length / one-longer prefix, ≤ 1 at 4–6 letters, ≤ 2 at 7+; order by score, then the family's best `FREQUENCY_RANK` (numbered books without a typed ordinal), then code (`1CO` < `2CO` = the ordinal-1 default); keys of ≤ 2 letters put their `SHORT_DEFAULTS` first. Stored names of the main translation score as tier-1 forms (spec §5.1 last paragraph).

- [ ] **Step 1: Write the failing tests** — `resolver/tests/rank.rs`:

```rust
//! Candidate ranking (#850 spec §5.5).

use crate::bible::resolver::rank::{no_fold_candidate, osa_distance, rank};
use crate::bible::resolver::{fold_book_text, BookCandidate, MatchKind};

fn candidates(input: &str) -> Vec<BookCandidate> {
    no_fold_candidate(input)
        .map(|candidate| vec![candidate])
        .unwrap_or_else(|| rank(&fold_book_text(input), &[]))
}

fn codes(input: &str) -> Vec<String> {
    candidates(input).into_iter().map(|c| c.code).collect()
}

fn first(input: &str) -> String {
    codes(input).into_iter().next().unwrap_or_default()
}

#[test]
fn fold_collisions_and_the_no_fold_list() {
    assert_eq!(first("ab"), "HAB");
    assert_eq!(first("1k"), "1CO");
    assert_eq!(first("2k"), "2CO");
    assert_eq!(first("ps"), "PSA");
    assert_eq!(first("PŠ"), "SNG");
    assert_eq!(first("P.Š."), "SNG");
    assert_eq!(codes("1k")[..3], ["1CO", "1KI", "1CH"]);
}

#[test]
fn short_keys_follow_the_curated_defaults() {
    assert_eq!(first("z"), "PSA");
    assert_eq!(first("ž"), "PSA");
    assert_eq!(first("j"), "JHN");
    assert_eq!(first("jo"), "JOB");
    assert_eq!(first("kr"), "1KI");
    assert_eq!(first("jd"), "JUD");
    assert_eq!(codes("g")[..2], ["GEN", "GAL"]);
    assert_eq!(codes("z")[..6], ["PSA", "REV", "HEB", "ZEC", "LAM", "ZEP"]);
}

#[test]
fn one_letter_aliases_carry_no_extra_weight() {
    let jhn = candidates("j").remove(0);
    assert_eq!((jhn.code.as_str(), jhn.kind), ("JHN", MatchKind::Prefix));
}

#[test]
fn a_numbered_family_defaults_to_its_first_book() {
    assert_eq!(codes("kor")[..2], ["1CO", "2CO"]);
    assert_eq!(codes("tim")[..2], ["1TI", "2TI"]);
    let candidate = candidates("kor").remove(0);
    assert_eq!(
        (candidate.kind, candidate.penalty),
        (MatchKind::OrdinalStripped, 1)
    );
}

#[test]
fn exact_aliases_carry_their_source_form() {
    let heb = candidates("zid").remove(0);
    assert_eq!((heb.code.as_str(), heb.kind), ("HEB", MatchKind::Exact));
    assert_eq!(heb.matched_form, "Žid");
    assert_eq!(heb.number, 58);
}

#[test]
fn fuzzy_only_from_four_letters_within_the_length_limits() {
    let fuzzy = |input: &str| {
        candidates(input)
            .first()
            .filter(|c| c.kind == MatchKind::Fuzzy)
            .map(|c| (c.code.clone(), c.penalty))
    };
    assert!(candidates("hbr").is_empty(), "shorter than 4: never fuzzy");
    assert_eq!(
        fuzzy("hebx"),
        Some(("HEB".to_string(), 1)),
        "4–6 letters: distance 1"
    );
    assert_eq!(fuzzy("hxbx"), None, "4–6 letters: distance 2 is too far");
    assert_eq!(fuzzy("hebrjm"), None, "6 letters: distance 2 is too far");
    assert_eq!(
        fuzzy("hebrjmo"),
        Some(("HEB".to_string(), 2)),
        "7+ letters: distance 2"
    );
    assert_eq!(fuzzy("hebrjom"), Some(("HEB".to_string(), 1)));
}

#[test]
fn osa_distance_counts_a_transposition_once_and_caps_at_three() {
    assert_eq!(osa_distance(b"hebrejom", b"hebrjom"), 1);
    assert_eq!(osa_distance(b"hebrejom", b"hebrejom"), 0);
    assert_eq!(osa_distance(b"zidom", b"zdiom"), 1);
    assert_eq!(osa_distance(b"ab", b"abcdef"), 3);
}
```

- [ ] **Step 2: How it is verified.** CI `test` job; expectations from the mirror (`rank()` in `mirror.py` runs on the same generated rows).

- [ ] **Step 3: Implement** — `resolver/rank.rs`. Table rows (`'static`) and stored names (borrowed) share one per-book board through `score_rows<'a, 'r: 'a>` — never `Iterator::chain` the two (different `Row` lifetimes):

```rust
//! Candidate scoring (#850 spec §5.5). Every form of the generated table (and
//! the main translation's stored names) that the typed key meets is scored
//! `(tier, kind, penalty)`; books are ordered by that score, then by the
//! citation frequency of their family, then by code (so `1 Kor` precedes
//! `2 Kor`). Keys of at most two letters go through `SHORT_DEFAULTS` first.
//! Fuzzy matching runs only when nothing else matched and the key has 4+ letters.

use unicode_normalization::UnicodeNormalization;

use super::fold::FoldedBook;
use super::names_table::{BOOK_CODES, BOOK_KEYS, FREQUENCY_RANK, NO_FOLD, SHORT_DEFAULTS};
use super::types::{BookCandidate, MatchKind, Tier};

const SHORT_KEY_MAX: usize = 2;
const FUZZY_MIN_KEY: usize = 4;
const FUZZY_CAP: usize = 3;

/// A stored book name of the main translation, matched as a tier-1 form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredName {
    pub(crate) index: u8,
    pub(crate) ordinal: u8,
    pub(crate) key: String,
    pub(crate) form: String,
}

#[derive(Clone, Copy)]
struct Row<'a> {
    key: &'a str,
    form: &'a str,
    index: u8,
    tier: Tier,
    ordinal: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    tier: Tier,
    kind: MatchKind,
    penalty: u8,
}

#[derive(Clone, Copy)]
struct Best<'a> {
    score: Score,
    form: &'a str,
}

type Board<'a> = [Option<Best<'a>>; 66];

#[derive(Clone, Copy)]
enum Pass {
    Prefix,
    Fuzzy,
}

/// Book candidates for a folded book text, best first.
pub(crate) fn rank(folded: &FoldedBook, stored: &[StoredName]) -> Vec<BookCandidate> {
    let key = folded.key.as_str();
    if key.is_empty() {
        return Vec::new();
    }
    let ordinal = folded.ordinal.unwrap_or(0);
    let start = BOOK_KEYS.partition_point(|row| row.0 < key);
    let prefixed = BOOK_KEYS
        .get(start..)
        .unwrap_or_default()
        .iter()
        .take_while(|row| row.0.starts_with(key))
        .map(table_row);
    let mut board: Board<'_> = [None; 66];
    score_rows(&mut board, prefixed, key, ordinal, Pass::Prefix);
    let names = stored.iter().map(stored_row);
    score_rows(&mut board, names, key, ordinal, Pass::Prefix);
    if board.iter().all(Option::is_none) && key.len() >= FUZZY_MIN_KEY {
        let rows = BOOK_KEYS.iter().map(table_row);
        score_rows(&mut board, rows, key, ordinal, Pass::Fuzzy);
        let names = stored.iter().map(stored_row);
        score_rows(&mut board, names, key, ordinal, Pass::Fuzzy);
    }
    short_defaults_first(folded, order(&board))
        .into_iter()
        .map(|(index, best)| candidate(index, best.score, best.form))
        .collect()
}

/// `P.Š.` / `PŠ` (Song of Songs), matched before folding — folded it is `ps`.
pub(crate) fn no_fold_candidate(book_text: &str) -> Option<BookCandidate> {
    let key: String = book_text
        .nfc()
        .flat_map(char::to_lowercase)
        .filter(|c| *c != '.' && !c.is_whitespace())
        .collect();
    let &(_, index) = NO_FOLD.iter().find(|(form, _)| *form == key)?;
    let score = Score {
        tier: Tier::T1,
        kind: MatchKind::Exact,
        penalty: 0,
    };
    Some(candidate(index, score, book_text.trim()))
}

pub(crate) fn index_of(code: &str) -> Option<u8> {
    BOOK_CODES
        .iter()
        .position(|candidate| *candidate == code)
        .and_then(|index| u8::try_from(index).ok())
}

fn code_of(index: u8) -> &'static str {
    BOOK_CODES.get(usize::from(index)).copied().unwrap_or("")
}

fn rank_of(index: u8) -> u8 {
    FREQUENCY_RANK
        .get(usize::from(index))
        .copied()
        .unwrap_or(u8::MAX)
}

fn table_row(&(key, form, index, packed): &(&'static str, &'static str, u8, u8)) -> Row<'static> {
    let tier = match packed >> 4 {
        1 => Tier::T1,
        2 => Tier::T2,
        _ => Tier::T3,
    };
    Row {
        key,
        form,
        index,
        tier,
        ordinal: packed & 0x0F,
    }
}

fn stored_row(name: &StoredName) -> Row<'_> {
    Row {
        key: &name.key,
        form: &name.form,
        index: name.index,
        tier: Tier::T1,
        ordinal: name.ordinal,
    }
}

fn prefix_score(row: &Row<'_>, key: &str, ordinal: u8) -> Option<Score> {
    if !row.key.starts_with(key) || (ordinal != 0 && row.ordinal != ordinal) {
        return None;
    }
    // One-letter aliases (SEP J, L, R, G) carry no extra weight.
    let kind = if (key.len() == 1 && ordinal == 0) || row.key != key {
        MatchKind::Prefix
    } else if row.ordinal == ordinal {
        MatchKind::Exact
    } else {
        MatchKind::OrdinalStripped
    };
    Some(Score {
        tier: row.tier,
        kind,
        penalty: u8::from(ordinal == 0 && row.ordinal != 0),
    })
}

/// Damerau-Levenshtein ≤ 1 for keys of 4–6 letters, ≤ 2 for 7+, against the
/// whole form and its same-length prefix (`hebrjom` ~ `hebrejom`). Tier 3
/// forms and forms shorter than 4 letters never match fuzzily.
fn fuzzy_score(row: &Row<'_>, key: &str, ordinal: u8) -> Option<Score> {
    if row.tier == Tier::T3
        || row.key.len() < FUZZY_MIN_KEY
        || (ordinal != 0 && row.ordinal != ordinal)
    {
        return None;
    }
    let limit = if key.len() <= 6 { 1 } else { 2 };
    let (typed, form) = (key.as_bytes(), row.key.as_bytes());
    let same_length = form.get(..typed.len()).unwrap_or(form);
    let one_longer = form.get(..typed.len() + 1).unwrap_or(form);
    let distance = osa_distance(typed, form)
        .min(osa_distance(typed, same_length))
        .min(osa_distance(typed, one_longer));
    (distance <= limit).then(|| Score {
        tier: row.tier,
        kind: MatchKind::Fuzzy,
        penalty: u8::from(ordinal == 0 && row.ordinal != 0)
            .saturating_add(u8::try_from(distance).unwrap_or(u8::MAX)),
    })
}

/// Optimal-string-alignment distance, capped at 3 (and 3 when the lengths
/// differ by more than 2).
pub(crate) fn osa_distance(a: &[u8], b: &[u8]) -> usize {
    if a.len().abs_diff(b.len()) > 2 {
        return FUZZY_CAP;
    }
    let width = b.len() + 1;
    let mut d = vec![0_usize; (a.len() + 1) * width];
    for (j, cell) in d.iter_mut().take(width).enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        d[i * width] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[(i - 1) * width + j] + 1)
                .min(d[i * width + j - 1] + 1)
                .min(d[(i - 1) * width + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[(i - 2) * width + j - 2] + 1);
            }
            d[i * width + j] = best;
        }
    }
    d[a.len() * width + b.len()].min(FUZZY_CAP)
}

/// Score `rows` into `board`, keeping each book's best form. `'r: 'a` lets the
/// `'static` table rows and the borrowed stored names share one board.
fn score_rows<'a, 'r: 'a>(
    board: &mut Board<'a>,
    rows: impl Iterator<Item = Row<'r>>,
    key: &str,
    ordinal: u8,
    pass: Pass,
) {
    for row in rows {
        let found = match pass {
            Pass::Prefix => prefix_score(&row, key, ordinal),
            Pass::Fuzzy => fuzzy_score(&row, key, ordinal),
        };
        if let Some(score) = found {
            keep_best(board, row, score);
        }
    }
}

fn keep_best<'a>(board: &mut Board<'a>, row: Row<'a>, score: Score) {
    if let Some(slot) = board.get_mut(usize::from(row.index)) {
        match slot {
            Some(best) if best.score <= score => {}
            _ => {
                *slot = Some(Best {
                    score,
                    form: row.form,
                });
            }
        }
    }
}

fn order<'a>(board: &Board<'a>) -> Vec<(u8, Best<'a>)> {
    let scored: Vec<(u8, Best<'a>)> = board
        .iter()
        .zip(0_u8..)
        .filter_map(|(best, index)| best.map(|best| (index, best)))
        .collect();
    let mut keyed: Vec<_> = scored
        .iter()
        .map(|&(index, best)| {
            let family = family_rank(index, best.score.penalty, &scored);
            ((best.score, family, code_of(index)), (index, best))
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, entry)| entry).collect()
}

/// A numbered book matched without a typed ordinal ranks by its family's
/// most cited member (`kor` → the Korinťanom family at 1 Kor's rank).
fn family_rank(index: u8, penalty: u8, scored: &[(u8, Best<'_>)]) -> u8 {
    let code = code_of(index);
    if penalty == 0 || !code.starts_with(|c: char| c.is_ascii_digit()) {
        return rank_of(index);
    }
    let family = code.get(1..);
    scored
        .iter()
        .filter(|(other, _)| code_of(*other).get(1..) == family)
        .map(|(other, _)| rank_of(*other))
        .min()
        .unwrap_or_else(|| rank_of(index))
}

fn short_defaults_first<'a>(
    folded: &FoldedBook,
    ordered: Vec<(u8, Best<'a>)>,
) -> Vec<(u8, Best<'a>)> {
    if folded.key.len() > SHORT_KEY_MAX {
        return ordered;
    }
    let full_key = match folded.ordinal {
        Some(ordinal) => format!("{ordinal}{}", folded.key),
        None => folded.key.clone(),
    };
    let Some(&(_, preferred)) = SHORT_DEFAULTS.iter().find(|(key, _)| *key == full_key) else {
        return ordered;
    };
    let mut head: Vec<(u8, Best<'a>)> = preferred
        .iter()
        .filter_map(|index| ordered.iter().find(|(i, _)| i == index).copied())
        .collect();
    head.extend(
        ordered
            .iter()
            .filter(|(i, _)| !preferred.contains(i))
            .copied(),
    );
    head
}

fn candidate(index: u8, score: Score, form: &str) -> BookCandidate {
    BookCandidate {
        code: code_of(index).to_string(),
        number: u16::from(index) + 1,
        tier: score.tier,
        kind: score.kind,
        penalty: score.penalty,
        matched_form: form.to_string(),
    }
}
```

- [ ] **Step 4: Local gates** — `cargo fmt --all -- --check`; `QC_TARGETS=crates/presenter-core/src/bible/resolver/rank.rs python3 scripts/dev/fn_length_check.py .`; the generic `score_rows`, `keep_best`, `order` and `short_defaults_first` are invisible to the checker — measured by hand they are ≤ 26 lines; `bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/bible/resolver/rank.rs` (320).

- [ ] **Step 5: Commit** — paths `crates/presenter-core/src/bible/resolver/rank.rs crates/presenter-core/src/bible/resolver/tests/rank.rs crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/tests/mod.rs`, message:

```text
feat(bible): rank book candidates by tier, match kind and citation frequency (#850 A1b)

Exact, ordinal-stripped, prefix and (only when nothing else matched, 4+
letters) fuzzy matches against the generated table and the main
translation's stored names; numbered families default to their first book;
keys of two letters or less follow the curated SHORT_DEFAULTS; P.Š./PŠ are
matched before folding.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 16: Versification, binding and stepping

**Files:**
- Create: `crates/presenter-core/src/bible/resolver/versification.rs`, `crates/presenter-core/src/bible/resolver/tests/versification.rs`
- Create: `crates/presenter-core/src/bible/resolver/testdata/structure-slk-seb.json`, `structure-eng-kjv.json`, `structure-slk-sevp.json`
- Modify: `resolver/mod.rs` (`mod versification;` after `mod types;`, and `pub use versification::{BookStructure, Step, Versification};` after the `pub use types::{…};` block), `resolver/tests/mod.rs` (whole file, below)

**Interfaces:**
- Consumes: `fold::fold_book_text`, `hint::{chapters_hint, verses_hint}`, `rank::{index_of, StoredName}`, `types::{RawSpec, Segment}`, `crate::bible::BibleBookChapterSummary { book, book_code: Option<String>, book_number: Option<u16>, chapter: u16, verse_count: u16 }` (`translation.rs:131-139`).
- Produces: `pub struct BookStructure { pub code: String, pub number: u16, pub name: String, pub verse_counts: Vec<u16> }` + `pub fn chapter_count(&self) -> u16`, `pub fn verse_count(&self, chapter: u16) -> Option<u16>`; `pub enum Step { Next, Prev }`; `pub struct Versification` + `pub fn new(books: Vec<BookStructure>) -> Self`, `pub fn from_chapter_summaries(rows: &[BibleBookChapterSummary]) -> Self`, `pub fn books(&self) -> &[BookStructure]`, `pub fn book(&self, code: &str) -> Option<&BookStructure>`, `pub fn step(&self, code: &str, chapter: u16, verse: u16, dir: Step) -> Option<(String, u16, u16)>`, `pub(crate) fn stored_names(&self) -> &[StoredName]`; `pub(crate) struct Failure { pub(crate) requested: Segment, pub(crate) hint: String, pub(crate) fix: Vec<Segment> }`, `pub(crate) fn bind(spec: &RawSpec, book: &BookStructure) -> Option<Vec<Segment>>`, `pub(crate) fn out_of_range(spec: &RawSpec, book: &BookStructure) -> Failure`.
- `step` crosses chapter AND book bounds (§5.1); §7.2's "disabled at the end of the book" is the console's decision (B2b compares the returned code). `from_chapter_summaries` sorts by book number, so the name-ordered rows of `query.rs:310` are harmless.

- [ ] **Step 1: Fixtures** — the `GET /bible/books` snapshots (dev = prod = PP, verified 2026-10-13):

```bash
cd "$REPO" && T=crates/presenter-core/src/bible/resolver/testdata
for t in slk-seb eng-kjv slk-sevp; do curl -fsS "http://10.77.8.134:8080/bible/books?translation=$t" -o "$T/structure-$t.json"; done
for t in slk-seb eng-kjv slk-sevp; do cmp "$T/structure-$t.json" "$SCRATCH/resolver-in/live/prod-books-$t.json" && echo "same $t"; done
```

- [ ] **Step 2: Write the failing tests.** `resolver/tests/mod.rs` becomes (fixture helpers shared by the test modules):

```rust
//! Resolver tests (#850 spec §10). Fixtures in `../testdata/`: the fold cases
//! shared with the generator, the 100-case corpus and `/bible/books` snapshots.

mod grammar;
mod hint;
mod rank;
mod table;
mod versification;

use serde::Deserialize;

use super::{BookStructure, Versification};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookDto {
    book: String,
    code: String,
    number: u16,
    chapters: Vec<ChapterDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterDto {
    number: u16,
    verse_count: u16,
}

/// A `GET /bible/books` snapshot as the main translation's structure.
fn structure(json: &str) -> Versification {
    let books: Vec<BookDto> = serde_json::from_str(json).expect("structure fixture parses");
    Versification::new(
        books
            .into_iter()
            .map(|book| {
                let mut chapters = book.chapters;
                chapters.sort_by_key(|chapter| chapter.number);
                BookStructure {
                    code: book.code,
                    number: book.number,
                    name: book.book,
                    verse_counts: chapters.iter().map(|chapter| chapter.verse_count).collect(),
                }
            })
            .collect(),
    )
}

fn seb() -> Versification {
    structure(include_str!("../testdata/structure-slk-seb.json"))
}

fn kjv() -> Versification {
    structure(include_str!("../testdata/structure-eng-kjv.json"))
}

fn sevp() -> Versification {
    structure(include_str!("../testdata/structure-slk-sevp.json"))
}
```

`resolver/tests/versification.rs`:

```rust
//! The main translation's structure (#850 spec §5.6): binding, out-of-range
//! hints and fixes, stepping, building from chapter summaries.

use super::{kjv, seb};
use crate::bible::resolver::versification::{bind, out_of_range};
use crate::bible::resolver::{BookStructure, RawSpec, Segment, Step, Versification};
use crate::bible::BibleBookChapterSummary;

fn book(structure: &Versification, code: &str) -> BookStructure {
    structure.book(code).cloned().expect("book in the fixture")
}

fn verses(chapter: u16, ranges: &[(u16, u16)]) -> RawSpec {
    RawSpec::Verses {
        chapter,
        ranges: ranges.to_vec(),
    }
}

#[test]
fn versification_differs_per_translation() {
    let spec = verses(4, &[(5, 5)]);
    assert_eq!(
        bind(&spec, &book(&kjv(), "MAL")),
        Some(vec![Segment::verses(4, 5, 5)])
    );
    assert_eq!(
        bind(&spec, &book(&seb(), "MAL")),
        None,
        "SEB Malachi has 3 chapters"
    );
    assert_eq!(
        out_of_range(&spec, &book(&seb(), "MAL")).hint,
        "Kniha má len 3 kapitoly"
    );
}

#[test]
fn single_chapter_books_read_a_lone_number_as_a_verse() {
    let jude = book(&seb(), "JUD");
    assert_eq!(
        bind(&RawSpec::Chapter(5), &jude),
        Some(vec![Segment::verses(1, 5, 5)])
    );
    assert_eq!(
        bind(&RawSpec::Chapter(1), &jude),
        Some(vec![Segment::whole(1)])
    );
    let range = RawSpec::ChapterRange { start: 1, end: 5 };
    assert_eq!(bind(&range, &jude), Some(vec![Segment::verses(1, 1, 5)]));
    assert_eq!(
        bind(&verses(2, &[(5, 5)]), &jude),
        None,
        "Jude has no chapter 2"
    );
}

#[test]
fn lists_chapter_ranges_and_cross_chapter_ranges_become_segments() {
    let s = seb();
    assert_eq!(
        bind(&verses(5, &[(3, 3), (5, 5), (7, 7)]), &book(&s, "MAT")),
        Some(vec![
            Segment::verses(5, 3, 3),
            Segment::verses(5, 5, 5),
            Segment::verses(5, 7, 7)
        ])
    );
    let psalms = RawSpec::ChapterRange { start: 23, end: 24 };
    assert_eq!(
        bind(&psalms, &book(&s, "PSA")),
        Some(vec![Segment::whole(23), Segment::whole(24)])
    );
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 1,
        end_chapter: 3,
        verse_end: 2,
    };
    assert_eq!(
        bind(&cross, &book(&s, "GEN")),
        Some(vec![
            Segment::verses(1, 1, 31),
            Segment::whole(2),
            Segment::verses(3, 1, 2)
        ])
    );
}

#[test]
fn every_segment_is_checked_not_only_the_last() {
    let s = seb();
    let list = verses(5, &[(3, 3), (99, 99), (7, 7)]);
    assert_eq!(bind(&list, &book(&s, "MAT")), None);
    assert_eq!(
        out_of_range(&list, &book(&s, "MAT")).hint,
        "Kapitola má len 48 veršov"
    );
    let beyond = RawSpec::CrossChapter {
        chapter: 50,
        verse_start: 1,
        end_chapter: 51,
        verse_end: 2,
    };
    assert_eq!(bind(&beyond, &book(&s, "GEN")), None);
    assert_eq!(
        out_of_range(&beyond, &book(&s, "GEN")).hint,
        "Kniha má len 50 kapitol"
    );
    let psalms = RawSpec::ChapterRange {
        start: 150,
        end: 151,
    };
    assert_eq!(bind(&psalms, &book(&s, "PSA")), None);
    assert_eq!(
        out_of_range(&psalms, &book(&s, "PSA")).requested,
        Segment::whole(151)
    );
}

#[test]
fn out_of_range_offers_the_last_chapter_or_verse_as_fix() {
    let s = seb();
    let heb = book(&s, "HEB");
    let failure = out_of_range(&RawSpec::Chapter(14), &heb);
    assert_eq!(failure.requested, Segment::whole(14));
    assert_eq!(failure.hint, "Kniha má len 13 kapitol");
    assert_eq!(failure.fix, [Segment::whole(13)]);
    let failure = out_of_range(&verses(11, &[(41, 41)]), &heb);
    assert_eq!(failure.requested, Segment::verses(11, 41, 41));
    assert_eq!(failure.hint, "Kapitola má len 40 veršov");
    assert_eq!(failure.fix, [Segment::verses(11, 40, 40)]);
    let failure = out_of_range(&verses(11, &[(39, 45)]), &heb);
    assert_eq!(failure.fix, [Segment::verses(11, 39, 40)]);
    let failure = out_of_range(&RawSpec::Chapter(30), &book(&s, "JUD"));
    assert_eq!(failure.hint, "Kapitola má len 25 veršov");
    assert_eq!(failure.fix, [Segment::verses(1, 25, 25)]);
}

#[test]
fn step_crosses_chapter_and_book_ends() {
    let s = seb();
    let at = |code: &str, chapter, verse| Some((code.to_string(), chapter, verse));
    assert_eq!(s.step("HEB", 11, 1, Step::Next), at("HEB", 11, 2));
    assert_eq!(s.step("HEB", 11, 40, Step::Next), at("HEB", 12, 1));
    assert_eq!(s.step("HEB", 13, 25, Step::Next), at("JAS", 1, 1));
    assert_eq!(s.step("JAS", 1, 1, Step::Prev), at("HEB", 13, 25));
    assert_eq!(s.step("HEB", 12, 1, Step::Prev), at("HEB", 11, 40));
    assert_eq!(s.step("GEN", 1, 1, Step::Prev), None);
    assert_eq!(s.step("REV", 22, 21, Step::Next), None);
    assert_eq!(s.step("XXX", 1, 1, Step::Next), None);
}

#[test]
fn chapter_summaries_build_a_canonical_structure() {
    let row = |book: &str, code: Option<&str>, chapter, verse_count| BibleBookChapterSummary {
        book: book.to_string(),
        book_code: code.map(str::to_string),
        book_number: None,
        chapter,
        verse_count,
    };
    let rows = [
        row("Židom", Some("HEB"), 2, 18),
        row("Genezis", Some("GEN"), 1, 31),
        row("Židom", Some("HEB"), 1, 14),
        row("Bez kódu", None, 1, 5),
        row("Židom", Some("HEB"), 0, 9),
    ];
    let s = Versification::from_chapter_summaries(&rows);
    let codes: Vec<&str> = s.books().iter().map(|b| b.code.as_str()).collect();
    assert_eq!(
        codes,
        ["GEN", "HEB"],
        "canonical order; no code / chapter 0 skipped"
    );
    let heb = book(&s, "HEB");
    assert_eq!((heb.number, heb.verse_counts.clone()), (58, vec![14, 18]));
    assert_eq!(heb.verse_count(3), None);
}
```

- [ ] **Step 3: How it is verified.** CI `test` job. Numbers verified against the fixtures: SEB HEB 13 chapters (11 → 40 verses, 13 → 25), MAL 3 vs KJV 4, GEN 1 = 31 verses, MAT 5 = 48, JUD = 25, JAS follows HEB.

- [ ] **Step 4: Implement** — `resolver/versification.rs`:

```rust
//! The MAIN translation's chapter/verse structure (#850 spec §5.6): built from
//! `/bible/books` in the UI and from the chapter summaries on the server. It
//! binds a typed chapter/verse part to a book (the structure filter of §5.5)
//! and steps verse by verse across chapter and book bounds.

use std::collections::BTreeMap;

use super::fold::fold_book_text;
use super::hint::{chapters_hint, verses_hint};
use super::rank::{index_of, StoredName};
use super::types::{RawSpec, Segment};
use crate::bible::BibleBookChapterSummary;

/// Chapters beyond this in a summary row are malformed input and skipped.
const MAX_CHAPTER: u16 = 200;

/// One book of the main translation: its stored name and verses per chapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookStructure {
    pub code: String,
    pub number: u16,
    pub name: String,
    /// `verse_counts[c - 1]` = verses in chapter `c`; 0 = the chapter is missing.
    pub verse_counts: Vec<u16>,
}

impl BookStructure {
    pub fn chapter_count(&self) -> u16 {
        u16::try_from(self.verse_counts.len()).unwrap_or(u16::MAX)
    }

    /// Verses in `chapter`; `None` when that chapter does not exist.
    pub fn verse_count(&self, chapter: u16) -> Option<u16> {
        let index = usize::from(chapter).checked_sub(1)?;
        self.verse_counts
            .get(index)
            .copied()
            .filter(|count| *count > 0)
    }
}

/// Which way [`Versification::step`] moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Prev,
}

/// The main translation's books in canonical order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Versification {
    books: Vec<BookStructure>,
    stored: Vec<StoredName>,
}

impl Versification {
    pub fn new(mut books: Vec<BookStructure>) -> Self {
        books.sort_by_key(|book| book.number);
        let stored = books.iter().filter_map(stored_name).collect();
        Self { books, stored }
    }

    /// From `bible_book_chapter_summaries` rows (one per book and chapter, in
    /// any order). Rows without a book code are skipped.
    pub fn from_chapter_summaries(rows: &[BibleBookChapterSummary]) -> Self {
        let mut books: BTreeMap<String, BookStructure> = BTreeMap::new();
        for row in rows {
            let Some(code) = row
                .book_code
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
            else {
                continue;
            };
            if row.chapter == 0 || row.chapter > MAX_CHAPTER {
                continue;
            }
            let book = books
                .entry(code.to_string())
                .or_insert_with(|| BookStructure {
                    code: code.to_string(),
                    number: row
                        .book_number
                        .or_else(|| index_of(code).map(|index| u16::from(index) + 1))
                        .unwrap_or(0),
                    name: row.book.clone(),
                    verse_counts: Vec::new(),
                });
            let index = usize::from(row.chapter) - 1;
            if book.verse_counts.len() <= index {
                book.verse_counts.resize(index + 1, 0);
            }
            if let Some(slot) = book.verse_counts.get_mut(index) {
                *slot = row.verse_count;
            }
        }
        Self::new(books.into_values().collect())
    }

    pub fn books(&self) -> &[BookStructure] {
        &self.books
    }

    pub fn book(&self, code: &str) -> Option<&BookStructure> {
        self.books.iter().find(|book| book.code == code)
    }

    pub(crate) fn stored_names(&self) -> &[StoredName] {
        &self.stored
    }

    /// The next / previous verse, crossing chapter AND book bounds (the
    /// console decides whether to stop at the end of a book). `None` before
    /// the first verse, after the last one, or for an unknown book/chapter.
    pub fn step(
        &self,
        code: &str,
        chapter: u16,
        verse: u16,
        dir: Step,
    ) -> Option<(String, u16, u16)> {
        let position = self.books.iter().position(|book| book.code == code)?;
        let book = self.books.get(position)?;
        let count = book.verse_count(chapter)?;
        match dir {
            Step::Next if verse < count => Some((book.code.clone(), chapter, verse + 1)),
            Step::Prev if verse > 1 => Some((book.code.clone(), chapter, (verse - 1).min(count))),
            Step::Next => match (chapter.saturating_add(1)..=book.chapter_count())
                .find(|c| book.verse_count(*c).is_some())
            {
                Some(next) => Some((book.code.clone(), next, 1)),
                None => first_verse(self.books.get(position + 1)?),
            },
            Step::Prev => match (1..chapter)
                .rev()
                .find_map(|c| Some((c, book.verse_count(c)?)))
            {
                Some((previous, last)) => Some((book.code.clone(), previous, last)),
                None => last_verse(self.books.get(position.checked_sub(1)?)?),
            },
        }
    }
}

fn first_verse(book: &BookStructure) -> Option<(String, u16, u16)> {
    let chapter = (1..=book.chapter_count()).find(|c| book.verse_count(*c).is_some())?;
    Some((book.code.clone(), chapter, 1))
}

fn last_verse(book: &BookStructure) -> Option<(String, u16, u16)> {
    let (chapter, count) = (1..=book.chapter_count())
        .rev()
        .find_map(|c| Some((c, book.verse_count(c)?)))?;
    Some((book.code.clone(), chapter, count))
}

fn stored_name(book: &BookStructure) -> Option<StoredName> {
    let folded = fold_book_text(&book.name);
    if folded.key.is_empty() {
        return None;
    }
    Some(StoredName {
        index: index_of(&book.code)?,
        ordinal: folded.ordinal.unwrap_or(0),
        key: folded.key,
        form: book.name.clone(),
    })
}

/// Why a typed chapter/verse part does not fit a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) requested: Segment,
    pub(crate) hint: String,
    /// The nearest passage that exists (last chapter / last verse); empty = none.
    pub(crate) fix: Vec<Segment>,
}

/// On a single-chapter book (OBA, PHM, 2JN, 3JN, JUD) a lone `n > 1` is verse
/// `n` and `n-m` are verses; `1` stays the whole book.
fn adapt(spec: &RawSpec, book: &BookStructure) -> RawSpec {
    if book.chapter_count() != 1 {
        return spec.clone();
    }
    match *spec {
        RawSpec::Chapter(verse) if verse > 1 => RawSpec::Verses {
            chapter: 1,
            ranges: vec![(verse, verse)],
        },
        RawSpec::ChapterRange { start, end } => RawSpec::Verses {
            chapter: 1,
            ranges: vec![(start, end)],
        },
        _ => spec.clone(),
    }
}

/// The segments `spec` names in `book`, or `None` when a chapter or verse does
/// not exist (spec §5.5: the structure filter runs last).
pub(crate) fn bind(spec: &RawSpec, book: &BookStructure) -> Option<Vec<Segment>> {
    let existing = |c: &u16| book.verse_count(*c).is_some();
    match adapt(spec, book) {
        RawSpec::Chapter(chapter) => book
            .verse_count(chapter)
            .map(|_| vec![Segment::whole(chapter)]),
        RawSpec::ChapterRange { start, end } => {
            book.verse_count(start)?;
            book.verse_count(end)?;
            Some((start..=end).filter(existing).map(Segment::whole).collect())
        }
        RawSpec::Verses { chapter, ranges } => {
            let count = book.verse_count(chapter)?;
            ranges.iter().all(|&(_, end)| end <= count).then(|| {
                ranges
                    .iter()
                    .map(|&(start, end)| Segment::verses(chapter, start, end))
                    .collect()
            })
        }
        RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        } => {
            let first = book.verse_count(chapter)?;
            let last = book.verse_count(end_chapter)?;
            if verse_start > first || verse_end > last {
                return None;
            }
            let mut segments = vec![Segment::verses(chapter, verse_start, first)];
            let middle = chapter.saturating_add(1)..end_chapter;
            segments.extend(middle.filter(existing).map(Segment::whole));
            segments.push(Segment::verses(end_chapter, 1, verse_end));
            Some(segments)
        }
    }
}

/// The hint and the nearest fix for a `spec` that [`bind`] refused on `book`.
pub(crate) fn out_of_range(spec: &RawSpec, book: &BookStructure) -> Failure {
    let chapters = book.chapter_count();
    let chapter_failure = |requested: Segment| Failure {
        requested,
        hint: chapters_hint(chapters),
        fix: if chapters == 0 {
            Vec::new()
        } else {
            vec![Segment::whole(chapters)]
        },
    };
    match adapt(spec, book) {
        RawSpec::Chapter(chapter) => chapter_failure(Segment::whole(chapter)),
        RawSpec::ChapterRange { start, end } => {
            let bad = if book.verse_count(start).is_none() {
                start
            } else {
                end
            };
            chapter_failure(Segment::whole(bad))
        }
        RawSpec::Verses { chapter, ranges } => {
            let (first_start, first_end) = ranges.first().copied().unwrap_or((1, 1));
            let Some(count) = book.verse_count(chapter) else {
                return chapter_failure(Segment::verses(chapter, first_start, first_end));
            };
            let (start, end) = ranges
                .iter()
                .copied()
                .find(|&(_, end)| end > count)
                .unwrap_or((first_start, first_end));
            Failure {
                requested: Segment::verses(chapter, start, end),
                hint: verses_hint(count),
                fix: vec![Segment::verses(chapter, start.min(count), count)],
            }
        }
        RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        } => {
            let requested = Segment {
                chapter,
                verse_start: Some(verse_start),
                verse_end: None,
            };
            let (Some(first), Some(last)) =
                (book.verse_count(chapter), book.verse_count(end_chapter))
            else {
                return chapter_failure(requested);
            };
            let clamped = RawSpec::CrossChapter {
                chapter,
                verse_start: verse_start.min(first),
                end_chapter,
                verse_end: verse_end.min(last),
            };
            Failure {
                requested,
                hint: verses_hint(if verse_start > first { first } else { last }),
                fix: bind(&clamped, book).unwrap_or_default(),
            }
        }
    }
}
```

- [ ] **Step 5: Local gates** — `cargo fmt --all -- --check`; `QC_TARGETS=crates/presenter-core/src/bible/resolver/versification.rs python3 scripts/dev/fn_length_check.py .` (`out_of_range` = 67 lines, the longest; must stay < 80); `bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/bible/resolver/versification.rs` (308).

- [ ] **Step 6: Commit** — paths `crates/presenter-core/src/bible/resolver/versification.rs crates/presenter-core/src/bible/resolver/tests/versification.rs crates/presenter-core/src/bible/resolver/tests/mod.rs crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/testdata/structure-slk-seb.json crates/presenter-core/src/bible/resolver/testdata/structure-eng-kjv.json crates/presenter-core/src/bible/resolver/testdata/structure-slk-sevp.json`, message:

```text
feat(bible): bind references to the main translation's versification (#850 A1b)

Versification from /bible/books (UI) or the chapter summaries (server):
every segment of a list, chapter range or cross-chapter range must exist,
single-chapter books read a lone number as a verse, an out-of-range input
gets the existing hint plus the nearest fix, and step() walks verse by
verse across chapter and book bounds.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 17: The public API and the 100-case corpus

**Files:**
- Modify: `crates/presenter-core/src/bible/resolver/mod.rs` (final content, below)
- Create: `crates/presenter-core/src/bible/resolver/testdata/corpus.json` (= `$SCRATCH/850-resolver-corpus.json`, 100 cases)
- Create: `crates/presenter-core/src/bible/resolver/tests/api.rs`, `crates/presenter-core/src/bible/resolver/tests/corpus.rs`
- Modify: `resolver/tests/mod.rs` (`mod api;` and `mod corpus;` above `mod grammar;`)

**Interfaces:**
- Produces (spec §5.1): `pub fn parse(input: &str) -> ParsedQuery`, `pub fn resolve(query: &ParsedQuery, structure: &Versification) -> Resolution`, `pub fn lookup(input: &str, structure: &Versification) -> Resolution`, `pub fn resolve_book_name(name: &str) -> Option<BookCandidate>`, `pub fn resolve_translation_code(input: &str, installed: &[&str]) -> Option<String>`.
- `resolve`: a bare ordinal or an ordinal with no name → `Incomplete` (the structure's books whose code starts with that digit); no name → `NoBook` (hint "Napíš aj knihu" when numbers were typed); candidates re-ranked with the structure's stored names and kept only when the main translation has the book (none left → `NoBook` with `MISSING_BOOK_HINT`); no numbers → `BookOnly`; else every fitting candidate becomes a `Passage` — none fit → `OutOfRange` for the best candidate with `out_of_range`'s hint and fix; the default is `Ambiguous` when any alternative's (tier, kind) is `<=` its own (`==` in the prototype; `<=` covers a `SHORT_DEFAULTS` default that scores worse, e.g. `jd 1`), else `Passage`.
- `resolve_translation_code`: an installed code in any case, else the ONE installed code whose part after the last `-` matches (`KJV` → `eng-kjv`); ambiguous or unknown → `None`.

- [ ] **Step 1: The corpus** — `cp "$SCRATCH/850-resolver-corpus.json" crates/presenter-core/src/bible/resolver/testdata/corpus.json` (outcomes: 83 resolved, 7 ambiguous, 4 out_of_range, 2 book_only, 1 each list / cross_chapter / incomplete / no_book).

- [ ] **Step 2: Write the failing tests.** `resolver/tests/corpus.rs` — the row semantics (spec §10 corpus mapping): `resolved`/`list`/`cross_chapter` and `book_only` with a chapter → `Passage` with the expected segments (a list = one per verse, a cross-chapter range = to the end of its first chapter then from verse 1); `ambiguous` → `Ambiguous`; `out_of_range` → `OutOfRange` with the exact `requested` segment and `hint`; `book_only` without a chapter → `BookOnly`; `incomplete` → ordinal 1 with only `1xx` books; `no_book` → the typed segments + hint; `match: fuzzy` → the first parse candidate is `Fuzzy`. Corpus `alternatives` that fit must open `Resolution` alternatives in corpus order; the others must be parse candidates that never fit (the corpus lists 2 Ján/3 Ján for `jan 3:16`, which have no chapter 3, while §5.1 keeps only fitting books):

```rust
//! The 100-case corpus (#850 spec §2 S1, §10) against slk-seb, the main
//! translation of SNV and dev. A row's `alternatives` are books a picker should
//! offer: those that fit the structure must open `alternatives` in corpus order;
//! those that do not (2 Ján has no chapter 3) must be parse candidates that never fit.

use serde::Deserialize;

use super::seb;
use crate::bible::resolver::versification::bind;
use crate::bible::resolver::{
    lookup, parse, resolve, segment_label, BookRef, MatchKind, ParsedQuery, Passage, Resolution,
    Segment, Versification,
};

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<CorpusCase>,
}

#[derive(Deserialize)]
struct CorpusCase {
    id: u32,
    input: String,
    outcome: String,
    expected: Option<Expected>,
    #[serde(default)]
    alternatives: Vec<String>,
    #[serde(default)]
    hint: Option<String>,
    #[serde(default)]
    verses: Vec<u16>,
    #[serde(default)]
    end_chapter: Option<u16>,
    #[serde(default, rename = "match")]
    match_kind: Option<String>,
}

#[derive(Deserialize)]
struct Expected {
    book: Option<String>,
    chapter: Option<u16>,
    verse_start: Option<u16>,
    verse_end: Option<u16>,
}

fn corpus() -> Vec<CorpusCase> {
    let corpus: Corpus =
        serde_json::from_str(include_str!("../testdata/corpus.json")).expect("corpus parses");
    corpus.cases
}

fn codes(passages: &[Passage]) -> Vec<&str> {
    passages.iter().map(|p| p.book.code.as_str()).collect()
}

/// A list = one segment per verse; a cross-chapter range = to the end of its
/// first chapter, the whole middle chapters, then from verse 1.
fn expected_segments(case: &CorpusCase, structure: &Versification) -> Vec<Segment> {
    let Some(expected) = &case.expected else {
        return Vec::new();
    };
    let chapter = expected.chapter.unwrap_or(0);
    match case.outcome.as_str() {
        "list" => case
            .verses
            .iter()
            .map(|&v| Segment::verses(chapter, v, v))
            .collect(),
        "cross_chapter" => {
            let book = expected
                .book
                .as_deref()
                .and_then(|code| structure.book(code));
            let first = book.and_then(|b| b.verse_count(chapter)).unwrap_or(0);
            let end = case.end_chapter.unwrap_or(chapter);
            let mut segments = vec![Segment::verses(
                chapter,
                expected.verse_start.unwrap_or(1),
                first,
            )];
            segments.extend((chapter + 1..end).map(Segment::whole));
            segments.push(Segment::verses(end, 1, expected.verse_end.unwrap_or(1)));
            segments
        }
        _ => vec![Segment {
            chapter,
            verse_start: expected.verse_start,
            verse_end: expected.verse_end,
        }],
    }
}

fn alternative_problems(
    case: &CorpusCase,
    parsed: &ParsedQuery,
    got: &[&str],
    structure: &Versification,
) -> Vec<String> {
    let mut problems = Vec::new();
    let fitting: Vec<&str> = case
        .alternatives
        .iter()
        .map(String::as_str)
        .filter(|code| got.contains(code))
        .collect();
    if got.get(..fitting.len()) != Some(fitting.as_slice()) {
        problems.push(format!(
            "alternatives {got:?} do not start with {fitting:?}"
        ));
    }
    for code in &case.alternatives {
        if !parsed.candidates.iter().any(|c| &c.code == code) {
            problems.push(format!("{code} is not a parse candidate"));
        }
        let fits = parsed
            .spec
            .as_ref()
            .zip(structure.book(code))
            .is_some_and(|(spec, book)| bind(spec, book).is_some());
        if fits && !got.contains(&code.as_str()) {
            problems.push(format!("{code} fits but is missing from the alternatives"));
        }
    }
    problems
}

fn book_problem(case: &CorpusCase, code: &str) -> Option<String> {
    let want = case.expected.as_ref().and_then(|e| e.book.as_deref());
    (Some(code) != want).then(|| format!("book {code} != {want:?}"))
}

fn passage_problems(
    case: &CorpusCase,
    passage: &Passage,
    structure: &Versification,
) -> Vec<String> {
    let mut problems: Vec<String> = book_problem(case, &passage.book.code).into_iter().collect();
    let want = expected_segments(case, structure);
    if passage.segments != want {
        problems.push(format!("segments {:?} != {want:?}", passage.segments));
    }
    problems
}

fn out_of_range_problems(
    case: &CorpusCase,
    book: &BookRef,
    requested: &Segment,
    hint: &str,
    structure: &Versification,
) -> Vec<String> {
    let want = expected_segments(case, structure);
    let mut problems: Vec<String> = book_problem(case, &book.code).into_iter().collect();
    if want.first() != Some(requested) {
        problems.push(format!("requested {requested:?} != {want:?}"));
    }
    if Some(hint) != case.hint.as_deref() {
        problems.push(format!("hint {hint:?} != {:?}", case.hint));
    }
    problems
}

fn resolution_problems(
    case: &CorpusCase,
    parsed: &ParsedQuery,
    resolution: &Resolution,
    structure: &Versification,
) -> Vec<String> {
    let whole_chapter_book_only =
        case.outcome == "book_only" && case.expected.as_ref().and_then(|e| e.chapter).is_some();
    match (case.outcome.as_str(), resolution) {
        (
            "resolved" | "list" | "cross_chapter" | "book_only",
            Resolution::Passage {
                passage,
                alternatives,
            },
        )
        | (
            "ambiguous",
            Resolution::Ambiguous {
                default: passage,
                alternatives,
            },
        ) if case.outcome != "book_only" || whole_chapter_book_only => {
            let mut problems = passage_problems(case, passage, structure);
            problems.extend(alternative_problems(
                case,
                parsed,
                &codes(alternatives),
                structure,
            ));
            problems
        }
        (
            "out_of_range",
            Resolution::OutOfRange {
                book,
                requested,
                hint,
                ..
            },
        ) => out_of_range_problems(case, book, requested, hint, structure),
        ("book_only", Resolution::BookOnly { book, alternatives }) => {
            let got: Vec<&str> = alternatives.iter().map(|b| b.code.as_str()).collect();
            let mut problems: Vec<String> = book_problem(case, &book.code).into_iter().collect();
            problems.extend(alternative_problems(case, parsed, &got, structure));
            problems
        }
        ("incomplete", Resolution::Incomplete { ordinal, books })
            if *ordinal == 1
                && !books.is_empty()
                && books.iter().all(|b| b.code.starts_with('1')) =>
        {
            Vec::new()
        }
        ("no_book", Resolution::NoBook { segments, hint })
            if *segments == expected_segments(case, structure) && hint == &case.hint =>
        {
            Vec::new()
        }
        (outcome, other) => vec![format!("outcome {outcome}, got {other:?}")],
    }
}

fn case_problems(case: &CorpusCase, structure: &Versification) -> Vec<String> {
    let parsed = parse(&case.input);
    let resolution = resolve(&parsed, structure);
    let mut problems = resolution_problems(case, &parsed, &resolution, structure);
    let first_kind = parsed.candidates.first().map(|c| c.kind);
    if case.match_kind.as_deref() == Some("fuzzy") && first_kind != Some(MatchKind::Fuzzy) {
        problems.push(format!("not a fuzzy match: {first_kind:?}"));
    }
    problems
        .into_iter()
        .map(|problem| format!("#{} {:?}: {problem}", case.id, case.input))
        .collect()
}

#[test]
fn corpus_100_of_100_on_slk_seb() {
    let structure = seb();
    let cases = corpus();
    assert_eq!(cases.len(), 100);
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|case| case_problems(case, &structure))
        .collect();
    assert!(
        failures.is_empty(),
        "{} corpus problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_17_incident_spellings_of_hebrews_11_1_resolve_to_heb() {
    let structure = seb();
    for case in corpus().iter().take(17) {
        let book = match lookup(&case.input, &structure) {
            Resolution::Passage { passage, .. } => passage.book.code,
            Resolution::OutOfRange { book, .. } => book.code,
            other => panic!("{:?} → {other:?}", case.input),
        };
        assert_eq!(book, "HEB", "{:?}", case.input);
    }
}

#[test]
fn zid_11_1_shows_the_stored_name_and_the_matched_form() {
    let Resolution::Passage {
        passage,
        alternatives,
    } = lookup("zid 11 1", &seb())
    else {
        panic!("zid 11 1 must be a passage");
    };
    assert_eq!(
        (passage.book.name.as_str(), passage.book.number),
        ("Hebrejom", 58)
    );
    assert_eq!(passage.matched_form, "Žid");
    assert_eq!(passage.segments, [Segment::verses(11, 1, 1)]);
    assert!(alternatives.is_empty());
    assert_eq!(
        segment_label(&passage.book.name, &passage.segments[0]),
        "Hebrejom 11:1"
    );
}
```

`resolver/tests/api.rs`:

```rust
//! The public API end to end (#850 spec §5.1, §5.7): `lookup`, the name and
//! translation lookups, stored names at runtime, partial main translations.

use super::{seb, sevp};
use crate::bible::resolver::{
    lookup, parse, resolve_book_name, resolve_translation_code, segment_label, Passage, Resolution,
    Segment, Versification, MISSING_BOOK_HINT,
};

fn codes(passages: &[Passage]) -> Vec<&str> {
    passages.iter().map(|p| p.book.code.as_str()).collect()
}

#[test]
fn main_translation_decides_the_versification_and_the_names() {
    let Resolution::OutOfRange { hint, .. } = lookup("mal 4:5", &seb()) else {
        panic!("Malachi has 3 chapters in SEB");
    };
    assert_eq!(hint, "Kniha má len 3 kapitoly");
    let Resolution::Ambiguous {
        default,
        alternatives,
    } = lookup("J 3,16", &sevp())
    else {
        panic!("J 3,16 is ambiguous (Jer/Job 3:16 exist too)");
    };
    assert_eq!(default.book.name, "Evanjelium podľa Jána");
    assert_eq!(codes(&alternatives)[..4], ["JER", "JOB", "JAS", "JOS"]);
    let Resolution::Passage {
        passage,
        alternatives,
    } = lookup("1K 13,4", &sevp())
    else {
        panic!("1K 13,4 is 1 Korinťanom");
    };
    assert_eq!(passage.book.name, "Prvýlist apoštola Pavla Korintským");
    assert_eq!(codes(&alternatives), ["1KI", "1CH"]);
}

#[test]
fn the_structure_filter_picks_the_book_that_has_the_verse() {
    for (input, code) in [
        ("pt 5:7", "1PE"),
        ("ab 2:4", "HAB"),
        ("fil 4:13", "PHP"),
        ("jud 2:5", "JDG"),
    ] {
        let Resolution::Passage { passage, .. } = lookup(input, &seb()) else {
            panic!("{input} must be one passage");
        };
        assert_eq!(passage.book.code, code, "{input}");
    }
}

#[test]
fn out_of_range_fix_labels() {
    for (input, label) in [
        ("zid 14", "Hebrejom 13"),
        ("zid 11:41", "Hebrejom 11:40"),
        ("jud 30", "Júda 1:25"),
    ] {
        let Resolution::OutOfRange {
            book,
            fix: Some(fix),
            ..
        } = lookup(input, &seb())
        else {
            panic!("{input} must be out of range with a fix");
        };
        assert_eq!(
            segment_label(&book.name, &fix.segments[0]),
            label,
            "{input}"
        );
    }
}

#[test]
fn incomplete_and_no_book_inputs() {
    let structure = seb();
    for input in ["1", "1.", "I.", "prvý"] {
        let Resolution::Incomplete { ordinal, books } = lookup(input, &structure) else {
            panic!("{input} must be incomplete");
        };
        assert_eq!(ordinal, 1, "{input}");
        assert!(books.iter().any(|b| b.code == "1CO"), "{input}");
        assert!(books.iter().all(|b| b.code.starts_with('1')), "{input}");
    }
    let no_book = |segments: Vec<Segment>, hint: Option<&str>| Resolution::NoBook {
        segments,
        hint: hint.map(str::to_string),
    };
    assert_eq!(
        lookup("11:1", &structure),
        no_book(vec![Segment::verses(11, 1, 1)], Some("Napíš aj knihu"))
    );
    assert_eq!(
        lookup("viera je zárukou", &structure),
        no_book(Vec::new(), None)
    );
    assert_eq!(lookup("", &structure), no_book(Vec::new(), None));
}

#[test]
fn a_book_missing_from_a_partial_main_translation_says_so() {
    let gospels = Versification::new(
        seb()
            .books()
            .iter()
            .filter(|b| ["MAT", "MRK", "LUK", "JHN"].contains(&b.code.as_str()))
            .cloned()
            .collect(),
    );
    assert_eq!(
        lookup("zid 11 1", &gospels),
        Resolution::NoBook {
            segments: vec![Segment::verses(11, 1, 1)],
            hint: Some(MISSING_BOOK_HINT.to_string()),
        }
    );
    assert!(matches!(
        lookup("jan 3 16", &gospels),
        Resolution::Passage { .. }
    ));
}

#[test]
fn stored_names_of_the_main_translation_match_at_runtime() {
    let mut books = seb().books().to_vec();
    if let Some(heb) = books.iter_mut().find(|b| b.code == "HEB") {
        heb.name = "Hebräer".to_string();
    }
    let Resolution::Passage { passage, .. } = lookup("hebräer 11 1", &Versification::new(books))
    else {
        panic!("a stored name must resolve without the table");
    };
    assert_eq!(passage.book.code, "HEB");
    assert_eq!(passage.matched_form, "Hebräer");
}

#[test]
fn parse_keeps_the_typed_parts() {
    let parsed = parse("zid 11:1 viera");
    assert_eq!(
        (parsed.book_text.as_str(), parsed.key.as_str()),
        ("zid", "zid")
    );
    assert_eq!(parsed.leftover, "viera");
    assert_eq!(parsed.candidates[0].code, "HEB");
    assert_eq!(parse("PŠ 2,1").candidates[0].code, "SNG");
}

#[test]
fn book_name_and_translation_code_lookups() {
    let code = |name: &str| resolve_book_name(name).map(|c| c.code);
    assert_eq!(code("Hebr").as_deref(), Some("HEB"));
    assert_eq!(code("Židom").as_deref(), Some("HEB"));
    assert_eq!(code("John").as_deref(), Some("JHN"));
    assert_eq!(code(""), None);
    let installed = ["eng-kjv", "slk-seb", "slk-roh", "slk-sevp", "eng-nlt"];
    let translation = |input: &str| resolve_translation_code(input, &installed);
    assert_eq!(translation("KJV").as_deref(), Some("eng-kjv"));
    assert_eq!(translation("seb").as_deref(), Some("slk-seb"));
    assert_eq!(translation("SLK-SEVP").as_deref(), Some("slk-sevp"));
    assert_eq!(translation(" nlt ").as_deref(), Some("eng-nlt"));
    assert_eq!(translation("xyz"), None);
    assert_eq!(translation(""), None);
    assert_eq!(
        resolve_translation_code("seb", &["slk-seb", "ces-seb"]),
        None
    );
}
```

- [ ] **Step 3: How it is verified.** CI `test` job: `corpus_100_of_100_on_slk_seb` collects every problem of every case into one failure message (one CI run shows all of them). The plan's mirror of these exact rules scores 100/100 under the same checks (`python3 -I $SCRATCH/plan-probe/mirror.py $SCRATCH/plan-probe/repo $SCRATCH` → `100/100`); if CI reports a case, port the fix to the mirror first, then to Rust.

- [ ] **Step 4: Implement** — `resolver/mod.rs` final content:

```rust
//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5): [`parse`] turns text into book candidates and a
//! chapter/verse part, [`resolve`] binds them to the MAIN translation's
//! structure, [`lookup`] does both. The name table is generated
//! (`scripts/dev/gen_bible_book_names.py`); the importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
mod grammar;
mod hint;
#[rustfmt::skip]
pub mod names_table;
mod rank;
mod types;
mod versification;

pub use fold::{fold_book_text, FoldedBook};
pub use hint::{
    chapters_hint, counted, segment_label, verses_hint, MISSING_BOOK_HINT, NO_BOOK_HINT,
};
pub use types::{
    BookCandidate, BookRef, MatchKind, ParsedQuery, Passage, RawSpec, Resolution, Segment, Tier,
};
pub use versification::{BookStructure, Step, Versification};

/// Pure text analysis: book candidates (best first), chapter/verse part, leftover words.
pub fn parse(input: &str) -> ParsedQuery {
    let split = grammar::split_input(input);
    let folded = fold_book_text(&split.book_text);
    let candidates = match rank::no_fold_candidate(&split.book_text) {
        Some(candidate) => vec![candidate],
        None => rank::rank(&folded, &[]),
    };
    ParsedQuery {
        input: input.to_string(),
        book_text: split.book_text,
        ordinal: folded.ordinal,
        key: folded.key,
        bare_ordinal: split.bare_ordinal,
        candidates,
        spec: split.spec,
        leftover: split.leftover,
    }
}

/// `parse` + `resolve`.
pub fn lookup(input: &str, structure: &Versification) -> Resolution {
    resolve(&parse(input), structure)
}

/// The best book for a name alone, no structure (the server choke point, §5.8).
pub fn resolve_book_name(name: &str) -> Option<BookCandidate> {
    parse(name).candidates.into_iter().next()
}

/// `KJV` → `eng-kjv`, `seb` → `slk-seb`: an exact installed code (any case),
/// else the one installed code whose part after the last `-` matches.
pub fn resolve_translation_code(input: &str, installed: &[&str]) -> Option<String> {
    let wanted = input.trim();
    if wanted.is_empty() {
        return None;
    }
    if let Some(code) = installed
        .iter()
        .find(|code| code.eq_ignore_ascii_case(wanted))
    {
        return Some((*code).to_string());
    }
    let mut by_short = installed.iter().filter(|code| {
        code.rsplit('-')
            .next()
            .is_some_and(|short| short.eq_ignore_ascii_case(wanted))
    });
    let first = by_short.next()?;
    by_short.next().is_none().then(|| (*first).to_string())
}

/// Bind a parsed input to the MAIN translation's structure (spec §5.6, §5.7).
pub fn resolve(query: &ParsedQuery, structure: &Versification) -> Resolution {
    let typed_ordinal = if query.key.is_empty() {
        query.ordinal
    } else {
        None
    };
    if let Some(ordinal) = query.bare_ordinal.or(typed_ordinal) {
        return incomplete(ordinal, structure);
    }
    if query.key.is_empty() && query.candidates.is_empty() {
        let hint = query.spec.as_ref().map(|_| NO_BOOK_HINT.to_string());
        return no_book(query, hint);
    }
    let books = candidates_in(query, structure);
    let Some(((best, best_book), others)) = books.split_first() else {
        let hint = (!query.candidates.is_empty()).then(|| MISSING_BOOK_HINT.to_string());
        return no_book(query, hint);
    };
    let Some(spec) = &query.spec else {
        return Resolution::BookOnly {
            book: book_ref(best_book),
            alternatives: others.iter().map(|(_, book)| book_ref(book)).collect(),
        };
    };
    let fits: Vec<(&BookCandidate, Passage)> = books
        .iter()
        .filter_map(|(candidate, book)| {
            let segments = versification::bind(spec, book)?;
            Some((candidate, passage(candidate, book, segments)))
        })
        .collect();
    let Some(((default, default_passage), rest)) = fits.split_first() else {
        let failure = versification::out_of_range(spec, best_book);
        let fix = (!failure.fix.is_empty()).then(|| passage(best, best_book, failure.fix));
        return Resolution::OutOfRange {
            book: book_ref(best_book),
            requested: failure.requested,
            hint: failure.hint,
            fix,
        };
    };
    let alternatives: Vec<Passage> = rest.iter().map(|(_, p)| p.clone()).collect();
    let ties = rest
        .iter()
        .any(|(other, _)| (other.tier, other.kind) <= (default.tier, default.kind));
    if ties {
        Resolution::Ambiguous {
            default: default_passage.clone(),
            alternatives,
        }
    } else {
        Resolution::Passage {
            passage: default_passage.clone(),
            alternatives,
        }
    }
}

/// Candidates re-ranked with the structure's stored names (a new translation's
/// own names work without regenerating the table), kept only when the main
/// translation has the book.
fn candidates_in<'s>(
    query: &ParsedQuery,
    structure: &'s Versification,
) -> Vec<(BookCandidate, &'s BookStructure)> {
    let folded = FoldedBook {
        ordinal: query.ordinal,
        key: query.key.clone(),
    };
    let ranked = match rank::no_fold_candidate(&query.book_text) {
        Some(candidate) => vec![candidate],
        None => rank::rank(&folded, structure.stored_names()),
    };
    ranked
        .into_iter()
        .filter_map(|candidate| {
            let book = structure.book(&candidate.code)?;
            Some((candidate, book))
        })
        .collect()
}

fn incomplete(ordinal: u8, structure: &Versification) -> Resolution {
    let prefix = char::from_digit(u32::from(ordinal), 10);
    Resolution::Incomplete {
        ordinal,
        books: structure
            .books()
            .iter()
            .filter(|book| prefix.is_some_and(|digit| book.code.starts_with(digit)))
            .map(book_ref)
            .collect(),
    }
}

fn no_book(query: &ParsedQuery, hint: Option<String>) -> Resolution {
    Resolution::NoBook {
        segments: query
            .spec
            .as_ref()
            .map(RawSpec::unbound_segments)
            .unwrap_or_default(),
        hint,
    }
}

fn book_ref(book: &BookStructure) -> BookRef {
    BookRef {
        code: book.code.clone(),
        number: book.number,
        name: book.name.clone(),
    }
}

fn passage(candidate: &BookCandidate, book: &BookStructure, segments: Vec<Segment>) -> Passage {
    Passage {
        book: book_ref(book),
        matched_form: candidate.matched_form.clone(),
        segments,
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 5: Local gates** — `cargo fmt --all -- --check`; `QC_TARGETS=$(ls crates/presenter-core/src/bible/resolver/*.rs) python3 scripts/dev/fn_length_check.py .` (no warning; `resolve` = 57 lines); `for f in crates/presenter-core/src/bible/resolver/*.rs; do echo "$f $(bash scripts/dev/count_prod_lines.sh $f)"; done`; `rg -n 'unwrap\(\)|\.expect\(|panic!' crates/presenter-core/src/bible/resolver --glob '!tests/**'` → nothing.

- [ ] **Step 6: Commit** — paths `crates/presenter-core/src/bible/resolver/mod.rs crates/presenter-core/src/bible/resolver/testdata/corpus.json crates/presenter-core/src/bible/resolver/tests/api.rs crates/presenter-core/src/bible/resolver/tests/corpus.rs crates/presenter-core/src/bible/resolver/tests/mod.rs`, message:

```text
feat(bible): parse / resolve / lookup — the reference resolver API (#850 A1b)

parse is pure text, resolve binds to the MAIN translation's structure (the
structure filter runs last), lookup does both; resolve_book_name and
resolve_translation_code serve the server's choke point (A2a). The 100-case
corpus passes on slk-seb with alternatives order, hints and segments
checked; the 17 incident spellings of Hebrews 11:1 resolve to HEB.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 18: Playbook, ship A1b, mutation sweep

**Files:**
- Modify: `.claude/rules/bible-resolver.md` (append the section below)
- Modify: `CLAUDE.md` (the bible-resolver row from Task 11: append "; A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule")

**Interfaces:** none.

- [ ] **Step 1: Extend the rule** — append to `.claude/rules/bible-resolver.md`:

```markdown

## Resolution rules the corpus pins (A1b)

- `parse` is pure text (no structure); `resolve` binds to the MAIN translation's `Versification`
  and drops candidates whose chapter/verse does not exist — the best disambiguator (`pt 5:7` →
  1 Pt, `ab 2:4` → Hab, `jud 2:5` → Sudcov). EVERY segment is checked (`mt 5:3.99.7`,
  `gn 50:1-51:2`), not only the last verse as the prototype did.
- Order: (tier, kind, penalty), then the family's citation rank, then the code. Keys of at most
  two letters are reordered by the curated `SHORT_DEFAULTS`, which is why `Ambiguous` uses `<=`:
  an alternative whose (tier, kind) is as good as OR BETTER than the default makes it a guess
  (`jd 1`).
- Corpus `alternatives` = what a picker offers. The corpus test requires the FITTING ones to open
  `Resolution::alternatives` in corpus order and the non-fitting ones (2 Ján has no chapter 3) to
  be parse candidates that never fit; `Resolution::alternatives` holds fitting books only.
- Typing in progress is normal input: trailing `:` `,` `-` `až` are ignored, words after the
  numbers go to `leftover`, at most 64 chapter/verse tokens are parsed.
- Single-chapter books (OBA PHM 2JN 3JN JUD): a lone `n > 1` and `n-m` are verses, `1` is the book.
- Rust tests run in CI only (Tier-0). When a rule or the corpus changes, check it first in a
  throwaway Python mirror of these rules (the planning session's `plan-probe/mirror.py` did this
  for 100/100), then port to Rust.
```

Commit — paths `.claude/rules/bible-resolver.md CLAUDE.md`, message `docs(playbook): resolver resolution rules the corpus pins (#850 A1b) [no-test: docs only]` + body line + Co-Authored-By.

- [ ] **Step 2: Unit gates** — Task 5 Step 2 commands, `(cd crates/presenter-ui && cargo fmt --check)`, the generator unittest + `--check`, `git status --short`.

- [ ] **Step 3: Push, CI, PR, merge, deploy, release** — as Task 11 Step 3 with `Release v0.4.318: Bible reference resolver in presenter-core — grammar, ranking, versification, 100/100 corpus (#850 A1b)`; SNV/PP verification = version 0.4.318 in `/healthz` and the footer DOM plus the Bible page range hint (counted moved: open `/ui/operator/bible`, type a chapter past the end, read `data-role="bible-range-hint"` = "Kniha má len N kapitol").

- [ ] **Step 4: Mutation sweep (spec §10).** After the release: `gh workflow run mutation-full.yml --ref main`, monitor to terminal, download the report; every surviving mutant under `crates/presenter-core/src/bible/resolver/` is A1 work — kill it with a test on `dev` in the next commits (not a follow-up ticket).

- [ ] **Step 5:** `playbook-review`, update #850 (A1 done; resolver API ready for A2a/B2a).

---

## Self-review against the spec

**§5.1 Module and API** — module path `bible::resolver` (Task 10), every type and function of the listing (Tasks 13-17); `Versification::new` / `from_chapter_summaries` / `step` (Task 16); stored names matched at runtime as tier-1 keys (Task 15 `StoredName`, Task 16 `Versification::new`, test `stored_names_of_the_main_translation_match_at_runtime`). Additions: `RawSpec`, `ParsedQuery::bare_ordinal`, `Segment::{whole, verses}`, `BookStructure::{chapter_count, verse_count}`, `Versification::{books, book}`, the hint helpers and `segment_label`. Test files sit in `resolver/tests/*.rs` (the `bible/tests/` layout, file-size exempt) instead of one `tests.rs`.

**§5.2 Name table** — sources (Task 7: three BPRP files + `bprp-eng-preferred.yaml` + LICENSE, biblia-sk, presenter-stored, curated with all five keys), generator from `build_inventory.py` with `--check` + unittest in `quality` (Task 9), canonical arrays read from canonical.rs and refused unless 66 (Task 9 unittest), `@generated` header, key-sorted table with binary search / `partition_point` (Task 15 `rank`), `NO_FOLD`, `SHORT_DEFAULTS`, `FREQUENCY_RANK: [u8; 66]`, 768 keys; `Ab`→OBA removed (Task 8) and kept out by `curated.remove`; the importer stays exact (Task 8 touches one alias only); `THIRD_PARTY_NOTICES.md` (Task 7).

**§5.3 Folding** — Task 10 (NFD, ordinals 1–5 incl. `prvý/prvá (kniha|list)`, `1 k.`, roman with `.`/space, fillers incl. the prototype's English ones, `[a-z0-9]`), `NO_FOLD` before folding (Task 15 `no_fold_candidate`); the three collisions + `ps` pinned (Task 9 unittest, Task 10 `slovak_1k_and_2k_beat_english_kings`, Task 15 `fold_collisions_and_the_no_fold_list`).

**§5.4 Grammar** — Task 14: split, precedence, ranges incl. `až`/`az`, cross-chapter `1:1-2:3` and `1,1-2,3`, chapter range, lists `5:3.5.7` / `3:16,18`; single-chapter books in Task 16 `bind`; `normalize_reference` untouched (no file of `ai/` is modified).

**§5.5 Ranking** — Task 15 tiers / kinds / frequency, fuzzy limits, `SHORT_DEFAULTS` for ≤ 2-letter keys, ordinal-1 family default; the structure filter last (Task 17 `resolve`), `Ambiguous` vs `Passage` with fitting lower-ranked books as alternatives (`jan 3 16` → Ján + 1 Ján, corpus #18).

**§5.6 Validation** — UI/server constructors (Task 16); hints reuse `bible_range.rs` wording through the moved `counted` (Task 13); fixes "Hebrejom 13" / "Hebrejom 11:40" (Task 16/17 tests).

**§5.7 Kinds → corpus** — all seven kinds mapped in `corpus.rs` (Task 17).

**§5.9 rows that belong to A1** — none of the 18 call sites changes in A1; A1 delivers what they consume: `resolve_book_name` (rows 7-13, 17 — A2a), `resolve_translation_code` (rows 8, 9 — A2a), `lookup` + `Resolution::OutOfRange` (rows 1-6 — B2a/B2c); invariants kept: row 16 (`ai/bible_validator.rs` untouched), row 18 (importer exact; only the wrong `Ab` alias goes).

**§8 Observability** — `TraceLayer` with DEBUG span + DEBUG `on_response` in ms, client IP by the `live.rs`/#793 rule (Task 1); drop-ins removed in deploy.yml + release.yml (Task 3); `presenter::bible::show` with source/device/client_ip/reference/secondary from `trigger_bible_slide_output` (Task 2); journal volume measured on SNV (Task 5 Step 7). `presenter::bible::lookup` (B2a/A2a) and `presenter::bible::name` (A2a) are not A1 scope.

**§9 Access** — reconcile mode, dry run default, `--apply`, ids/names only, pure `desired_apps` / `plan` unit-tested without network, example config with placeholders, real config outside git, token via `secret exec --env CF_TOKEN`, adding a person = one e-mail + `--apply` (Task 4, rule in Task 5).

**§10 Unit tests (presenter-core)** — corpus 100/100 on `structure-slk-seb.json` + mapping rules (Task 17); `structure-eng-kjv.json` Malachi (Tasks 16/17); `structure-slk-sevp.json` `J 3,16`, `1K 13,4` (Task 17); fold table (Task 10); grammar table (Task 14); ranking collisions / `SHORT_DEFAULTS` / fuzzy limits (Task 15); table invariants (Task 10); `resolve_translation_code` + `step` across a chapter and a book end (Tasks 16/17); generator unittest (Task 9). `rstest` is not needed — plain table loops keep each `#[test]` self-asserting for the quality job's assertion check.

**Contradictions and gaps found while planning**

1. §11 sizes A1b at "400 + 200" LoC under the ≤ 600 rule; the planned (rustfmt-formatted, documented) code is ≈ 1,325 production + ≈ 945 test lines (A1a ≈ 600 + 265 plus the 827-line generated table, O1 ≈ 415 + 315). A clean split of A1b is not possible without dead code (`StoredName` is built only by `Versification`), so it ships as one PR — the owner may want to waive the cap for it.
2. §5.1 `step` "crosses chapter + book bounds" vs §7.2 "disabled at the end of the book": the plan keeps §5.1 and leaves stopping to the console (B2b).
3. §5.2's 3-tuple `BOOK_KEYS` cannot produce §5.8's `"matched": "Žid"` / §7.1's "· Žid ·": a display-form column is added (+ ≈ 12 KiB static data on wasm32: 6.2 KB of strings + 768 slice headers; dead-code-eliminated from the WASM until B2a calls the resolver).
4. §5.5 says `SHORT_DEFAULTS` "fixes the 14 sk/cs forms", but the Finding-6 table and the corpus keep `g` → GEN (SEP "G" = Galatians), `jo` → JOB (Czech "Jo" = Joel), `na` → LAM, `za` → PSA; only Czech `Jd` (→ JUD) and the `2 P` family (→ 2PE) are re-pointed, the rest stay reachable as alternatives.
5. §5.4 "a leading 1–3 … is an ordinal" vs SEP `4M`/`5M` and the fold's 1–5: the grammar uses 1–5 for the split and 1–3 for a bare `Incomplete` ordinal (no book code starts with 4 or 5).
6. §10 "no duplicate (key, code)": the merged sources legitimately repeat (key, code) across ordinals (`kralov` → 1KI, `kgs`, `peter` …); the invariant tested is (key, ordinal, code).
7. Corpus `alternatives` list books that cannot fit (cases 18, 30, 31, 74, 77, 87) while §5.1 defines alternatives as fitting books, and cases 30/31 order them HEB, ZEC, REV where the Finding-6 table and the prototype say REV, HEB, ZEC; the corpus test checks order only among fitting books and membership for the rest. The prototype never compared order or hints at all.
8. §9 "the real config lives in the dev2 vault next to the token": the airuleset vault hook refuses hand writes/reads under the vault directory and `secret` has no agent-side store, so the non-secret config goes to `~/.config/presenter/presenter-access.json` (0600); the token stays in the vault.
9. §8 "measure on dev first": dev2's `presenter-dev.service.d/gst-debug.conf` sets `tower_http=warn` and no global `info`, so dev shows neither request lines nor `presenter::bible::show`; the measurement moves to SNV (Task 5 Step 7) — removing that dev drop-in is outside the approved scope.
10. Prototype bugs not ported: `I.` → Izaiáš (dots stripped before folding), chapter ranges and end chapters unchecked, BPRP multi-book entries attributed to the previous OSIS id by its regex parser.
11. Outside this plan: `scripts/dev/quality-check.sh` check 10 (unwrap/expect/panic) never fires — its pattern `\.expect\\(` does not parse, rg exits 2 and the `if` is false. Worth its own ticket.
