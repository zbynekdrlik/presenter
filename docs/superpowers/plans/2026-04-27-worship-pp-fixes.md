# Worship-PP Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three regressions on the worship-pp stage and operator playlist after PR #268: stage layout overlap, invisible active-song highlight, empty operator playlist entry names.

**Architecture:** Three independent fixes shipped together. (1) Wrap the six worship-pp slide regions inside the existing `.stage-pp__slides-area` so the absolute positioning anchors to the left 70% column instead of the page. (2) Replace the faint `.stage-pp__playlist-entry--active` styling with a high-contrast pill for projector visibility. (3) Add `presentation_name: Option<String>` to `PlaylistEntryKind::Presentation` in `presenter-core`; the server enriches it on read using the existing `fetch_presentation_names_for_playlist`; the WASM operator reads it directly and the now-unused `rebuild_playlist_presentations_with_signal` helper is deleted.

**Tech Stack:** Rust (axum, sea-orm), Leptos 0.7 WASM (`presenter-ui`), CSS, Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-27-worship-pp-fixes-design.md` (commit `01f6703`).

---

## Context

`presenter-ui` is excluded from the root workspace and has a separate `Cargo.lock`. Tests for it run via `cargo test -p presenter-ui --target x86_64-unknown-linux-gnu` (native target since most tests don't need wasm32) or `cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu`. Clippy: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings -W clippy::all`.

Local Rust builds are allowed on this dev machine. Run fmt + clippy + tests locally before pushing.

`Playlist` and `PlaylistEntryKind` live in `presenter-core` and are used by both the server (storage + serialization) and the WASM client (deserialization). Adding a field means it flows through both with one change.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/presenter-core/src/playlist.rs` | Domain types — add `presentation_name` to `PlaylistEntryKind::Presentation` |
| `crates/presenter-persistence/src/repository/util.rs` | Set `presentation_name: None` when reading entries from DB |
| `crates/presenter-server/src/router/playlists.rs` | Apply `enrich_playlist_with_names` to all `Json(playlist)` and `Json(playlists)` returns |
| `crates/presenter-server/src/state/presentations.rs` | New helper `enrich_playlist_with_names` and `enrich_playlists_with_names` (callers in router and broadcast paths) |
| `crates/presenter-ui/src/components/presentation_list.rs` | Replace `ctx.presentations.get()` lookup with `entry.kind` direct read |
| `crates/presenter-ui/src/pages/operator.rs` | Remove the `rebuild_playlist_presentations_with_signal` block at line 494-508 |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Wrap six regions in `<div class="stage-pp__slides-area">` |
| `crates/presenter-ui/styles/stage.css` | Replace `.stage-pp__playlist-entry--active` rule with high-contrast styling |
| `tests/e2e/wasm-playlist-operations.spec.ts` | Add operator playlist-name visibility check |
| `tests/e2e/stage-worship-pp.spec.ts` | New E2E for layout no-overlap + active highlight |
| `Cargo.toml` (workspace), `crates/presenter-ui/Cargo.toml` | Version bump |

---

## Task 1: Bump version

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package]`)
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Run version check**

```bash
git fetch origin
gh release list --limit 1
grep '^version' Cargo.toml | head -1
grep '^version' crates/presenter-ui/Cargo.toml | head -1
```

Expected: workspace at `0.4.36`, latest release v0.4.26. Bump to `0.4.37`.

- [ ] **Step 2: Bump root workspace**

In `Cargo.toml`, find `[workspace.package]` block and change `version = "0.4.36"` to `version = "0.4.37"`.

- [ ] **Step 3: Bump presenter-ui**

In `crates/presenter-ui/Cargo.toml`, find `[package]` `version` and bump to next patch (e.g. `0.1.5` → `0.1.6`). Keep workspace coordination.

- [ ] **Step 4: Refresh Cargo.lock**

```bash
cargo check -p presenter-server 2>&1 | tail -5
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -5 && cd ../..
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.37 (#worship-pp-fixes)"
```

---

## Task 2: Add `presentation_name` field to `PlaylistEntryKind::Presentation`

**Files:**
- Modify: `crates/presenter-core/src/playlist.rs:42-53`
- Modify: `crates/presenter-persistence/src/repository/util.rs:106-109`

- [ ] **Step 1: Write failing serde round-trip test**

In `crates/presenter-core/src/playlist.rs`, add at the very bottom of the file (or in the existing `#[cfg(test)] mod tests` block if present — verify by running `grep -n "mod tests" crates/presenter-core/src/playlist.rs`; if absent, append the block):

```rust
#[cfg(test)]
mod presentation_name_tests {
    use super::*;
    use crate::id::{PlaylistEntryId, PresentationId};

    #[test]
    fn presentation_entry_round_trips_presentation_name() {
        let id = PlaylistEntryId::new();
        let pres_id = PresentationId::new();
        let entry = PlaylistEntry {
            id,
            kind: PlaylistEntryKind::Presentation {
                presentation_id: pres_id,
                midi_binding: None,
                presentation_name: Some("My Song".to_string()),
            },
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(
            json.contains("\"presentationName\":\"My Song\""),
            "expected camelCase field; got: {json}"
        );
        let parsed: PlaylistEntry = serde_json::from_str(&json).expect("deserialize");
        match parsed.kind {
            PlaylistEntryKind::Presentation {
                presentation_name, ..
            } => assert_eq!(presentation_name.as_deref(), Some("My Song")),
            _ => panic!("expected Presentation"),
        }
    }

    #[test]
    fn presentation_entry_omits_name_when_none() {
        let entry = PlaylistEntry {
            id: PlaylistEntryId::new(),
            kind: PlaylistEntryKind::Presentation {
                presentation_id: PresentationId::new(),
                midi_binding: None,
                presentation_name: None,
            },
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(
            !json.contains("presentationName"),
            "None should be skip_serializing_if; got: {json}"
        );
    }

    #[test]
    fn deserializing_legacy_payload_without_name_works() {
        // Backwards compat: old DB rows / clients that don't include the field.
        let json = r#"{"id":"00000000-0000-0000-0000-000000000001","type":"presentation","presentationId":"00000000-0000-0000-0000-000000000002"}"#;
        let parsed: PlaylistEntry = serde_json::from_str(json).expect("deserialize");
        match parsed.kind {
            PlaylistEntryKind::Presentation {
                presentation_name, ..
            } => assert_eq!(presentation_name, None),
            _ => panic!("expected Presentation"),
        }
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cargo test -p presenter-core presentation_name_tests 2>&1 | tail -15
```

Expected: compile error (field `presentation_name` doesn't exist) or test failures.

- [ ] **Step 3: Add field to `PlaylistEntryKind::Presentation`**

In `crates/presenter-core/src/playlist.rs`, replace lines 42-53:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PlaylistEntryKind {
    Presentation {
        presentation_id: PresentationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        midi_binding: Option<MidiBinding>,
        /// Display name resolved by the server when serializing the playlist
        /// for the client. Stored as None internally; populated only on the
        /// response path. See `enrich_playlist_with_names` in the server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presentation_name: Option<String>,
    },
    Separator {
        name: String,
    },
}
```

- [ ] **Step 4: Update persistence conversion**

In `crates/presenter-persistence/src/repository/util.rs`, find the `PlaylistEntryKind::Presentation { presentation_id, midi_binding }` construction at line 106-109 and add `presentation_name: None`:

```rust
            PlaylistEntryKind::Presentation {
                presentation_id,
                midi_binding,
                presentation_name: None,
            }
```

- [ ] **Step 5: Update any other constructors of `PlaylistEntryKind::Presentation`**

Find every site that constructs the variant:

```bash
grep -rn "PlaylistEntryKind::Presentation {" crates/ 2>&1 | head -20
```

For each, ensure it sets `presentation_name: None` (or `Some(..)` if it actually has the name to set). Likely sites: `presenter-server/src/router/playlists.rs` (replace_playlist_entries), `presenter-server/src/state/stage.rs`, anywhere in tests.

For `presenter-server/src/router/playlists.rs` around line 153, change to:

```rust
                Ok(PlaylistEntry {
                    id,
                    kind: PlaylistEntryKind::Presentation {
                        presentation_id: PresentationId::from_uuid(presentation_id),
                        midi_binding: binding,
                        presentation_name: None,
                    },
                })
```

For any test fixtures, set `presentation_name: None`.

- [ ] **Step 6: Run tests — expect PASS**

```bash
cargo test -p presenter-core presentation_name_tests 2>&1 | tail -10
cargo build --workspace 2>&1 | tail -10
```

Expected: the 3 new tests pass; the workspace still builds.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/playlist.rs crates/presenter-persistence/src/repository/util.rs crates/presenter-server/src/router/playlists.rs
# Add any other files that needed updating in step 5:
git status -s
git add -A
git commit -m "feat(core): add presentation_name field to playlist Presentation entry (#worship-pp-fixes)

Field is None internally and populated on the server response path
via fetch_presentation_names_for_playlist. Skip-serialize-if-none
keeps storage and intra-server flows untouched. Backwards-compatible
deserialization for legacy clients that don't send the field."
```

---

## Task 3: Server enrichment helper + apply to all response sites

**Files:**
- Modify: `crates/presenter-server/src/state/presentations.rs`
- Modify: `crates/presenter-server/src/router/playlists.rs`
- Modify: `crates/presenter-server/src/router/tests.rs`

- [ ] **Step 1: Write failing integration test**

In `crates/presenter-server/src/router/tests.rs`, add (or find the existing `playlist` test module and add to it):

```rust
#[tokio::test]
async fn get_playlist_response_includes_presentation_name() {
    let state = AppState::in_memory().await.unwrap();
    // Seed: create one library + presentation
    let library = state
        .create_library("Test Library", true)
        .await
        .expect("create library");
    let presentation = state
        .create_presentation(library.id, "My Song")
        .await
        .expect("create presentation");
    // Create playlist with that presentation as an entry
    let playlist = state
        .create_playlist("Test Playlist", true)
        .await
        .expect("create playlist");
    let entries = vec![presenter_core::playlist::PlaylistEntry {
        id: presenter_core::PlaylistEntryId::new(),
        kind: presenter_core::playlist::PlaylistEntryKind::Presentation {
            presentation_id: presentation.id,
            midi_binding: None,
            presentation_name: None,
        },
    }];
    state
        .replace_playlist_entries(playlist.id, entries)
        .await
        .expect("replace entries");

    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/playlists/{}", playlist.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap(),
    )
    .unwrap();
    let entry = &body["entries"][0];
    assert_eq!(entry["type"], "presentation");
    assert_eq!(
        entry["presentationName"], "My Song",
        "presentation_name must be present in playlist GET response"
    );
}

#[tokio::test]
async fn list_playlists_response_includes_presentation_names() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Lib", true).await.unwrap();
    let presentation = state
        .create_presentation(library.id, "Track One")
        .await
        .unwrap();
    let playlist = state.create_playlist("PL", true).await.unwrap();
    state
        .replace_playlist_entries(
            playlist.id,
            vec![presenter_core::playlist::PlaylistEntry {
                id: presenter_core::PlaylistEntryId::new(),
                kind: presenter_core::playlist::PlaylistEntryKind::Presentation {
                    presentation_id: presentation.id,
                    midi_binding: None,
                    presentation_name: None,
                },
            }],
        )
        .await
        .unwrap();
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body[0]["entries"][0]["presentationName"], "Track One");
}
```

If the existing tests use a different helper-function shape (e.g., `state.create_library` vs another method), check `crates/presenter-server/src/router/tests.rs` for the actual API surface used by sibling tests and match it. Run `grep -n "create_library\|create_presentation\|create_playlist" crates/presenter-server/src/state/` to find the actual method names. The test names and assertions stay the same; only the seed-data calls adapt.

- [ ] **Step 2: Run test — expect FAIL**

```bash
cargo test -p presenter-server router::tests::get_playlist_response_includes_presentation_name 2>&1 | tail -10
```

Expected: assertion fails — `presentationName` is not in the response.

- [ ] **Step 3: Add `enrich_playlist_with_names` helper**

In `crates/presenter-server/src/state/presentations.rs`, add after the existing `get_playlist` method:

```rust
    /// Populate `presentation_name` on each `PlaylistEntryKind::Presentation`
    /// entry. Idempotent — overwrites whatever was there.
    pub async fn enrich_playlist_with_names(
        &self,
        mut playlist: presenter_core::Playlist,
    ) -> anyhow::Result<presenter_core::Playlist> {
        let names = self
            .repository
            .fetch_presentation_names_for_playlist(&playlist)
            .await?;
        for entry in playlist.entries.iter_mut() {
            if let presenter_core::playlist::PlaylistEntryKind::Presentation {
                presentation_id,
                presentation_name,
                ..
            } = &mut entry.kind
            {
                *presentation_name = names.get(presentation_id).cloned();
            }
        }
        Ok(playlist)
    }

    pub async fn enrich_playlists_with_names(
        &self,
        playlists: Vec<presenter_core::Playlist>,
    ) -> anyhow::Result<Vec<presenter_core::Playlist>> {
        let mut out = Vec::with_capacity(playlists.len());
        for pl in playlists {
            out.push(self.enrich_playlist_with_names(pl).await?);
        }
        Ok(out)
    }
```

- [ ] **Step 4: Apply enrichment in router handlers**

In `crates/presenter-server/src/router/playlists.rs`:

Replace `list_playlists` body (lines 57-62):

```rust
#[instrument(skip_all)]
pub(super) async fn list_playlists(
    State(state): State<AppState>,
) -> Result<Json<Vec<Playlist>>, AppError> {
    let playlists = state.playlists().await?;
    let enriched = state.enrich_playlists_with_names(playlists).await?;
    Ok(Json(enriched))
}
```

Replace `get_playlist` body (lines 80-89):

```rust
#[instrument(skip_all)]
pub(super) async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Playlist>, AppError> {
    let playlist = state
        .get_playlist(PlaylistId::from_uuid(id))
        .await?
        .ok_or_else(|| AppError::not_found("playlist not found"))?;
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
}
```

Replace `update_playlist` final lines (110-117):

```rust
    let updated = state
        .playlists()
        .await?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .ok_or_else(|| AppError::not_found("playlist not found"))?;
    let enriched = state.enrich_playlist_with_names(updated).await?;
    Ok(Json(enriched))
```

Replace `replace_playlist_entries` final lines (178-181):

```rust
    let playlist = state
        .replace_playlist_entries(PlaylistId::from_uuid(id), entries)
        .await?;
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
```

Replace `create_playlist` final lines (73-77):

```rust
    let playlist = state
        .create_playlist(name, payload.show_in_dashboard)
        .await?;
    // Just-created has no entries, but enrich for response shape consistency.
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
```

- [ ] **Step 5: Run tests — expect PASS**

```bash
cargo test -p presenter-server router::tests::get_playlist_response_includes_presentation_name router::tests::list_playlists_response_includes_presentation_names 2>&1 | tail -10
cargo test -p presenter-server 2>&1 | tail -10
```

Expected: 2 new tests pass; all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/router/playlists.rs crates/presenter-server/src/state/presentations.rs crates/presenter-server/src/router/tests.rs
git commit -m "feat(server): enrich playlist responses with presentation_name (#worship-pp-fixes)

Server uses existing fetch_presentation_names_for_playlist to fill
presentation_name on each Presentation entry before serializing.
Applies to GET /playlists, GET /playlists/{id}, POST /playlists,
PATCH /playlists/{id}, PUT /playlists/{id}/entries.
Operator can now read the entry display name directly without
maintaining a parallel client-side index of empty-name summaries."
```

---

## Task 4: WASM operator reads `presentation_name` from entry; remove rebuild helper

**Files:**
- Modify: `crates/presenter-ui/src/components/presentation_list.rs`
- Modify: `crates/presenter-ui/src/pages/operator.rs:486-514`

- [ ] **Step 1: Update entry-name lookup in `presentation_list.rs`**

In `crates/presenter-ui/src/components/presentation_list.rs`, replace lines 351-356:

OLD:
```rust
                                        // Look up presentation name from presentations list or index
                                        let presentations = ctx.presentations.get();
                                        let pres_name = presentations.iter()
                                            .find(|p| p.id.to_string() == id)
                                            .map(|p| p.name.clone())
                                            .unwrap_or_default();
```

NEW:
```rust
                                        // Read presentation name directly from the playlist entry
                                        // (server enriches it via fetch_presentation_names_for_playlist).
                                        let pres_name = match &entry.kind {
                                            presenter_core::playlist::PlaylistEntryKind::Presentation {
                                                presentation_name, ..
                                            } => presentation_name.clone().unwrap_or_default(),
                                            _ => String::new(),
                                        };
```

Note: `entry` is the loop variable from `playlist.entries.iter().enumerate()` at line 176 — verify it's in scope at line 351 (it is — this is inside the `Presentation` arm of the match on `&entry.kind`).

- [ ] **Step 2: Remove `rebuild_playlist_presentations_with_signal` from operator.rs**

In `crates/presenter-ui/src/pages/operator.rs`, find the `spawn_local` block at lines 486-514 (the one that fetches a playlist on selection and rebuilds `presentations`). Replace the whole block:

OLD (lines 488-510, approximately):
```rust
        leptos::task::spawn_local(async move {
            // Fetch full playlist for entry rendering
            if let Ok(pl) = crate::api::playlists::get_playlist(&pl_id).await {
                context_title.set(pl.name.clone());
                let summaries: Vec<presenter_core::PresentationSummary> = pl
                    .entries
                    .iter()
                    .filter_map(|e| match &e.kind {
                        presenter_core::playlist::PlaylistEntryKind::Presentation {
                            presentation_id,
                            ..
                        } => Some(presenter_core::PresentationSummary::new(
                            *presentation_id,
                            String::new(),
                        )),
                        _ => None,
                    })
                    .collect();
                presentations.set(summaries);
                selected_playlist.set(Some(pl));
            }
            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                playlists.set(pls);
            }
        });
```

NEW:
```rust
        leptos::task::spawn_local(async move {
            // Fetch full playlist for entry rendering. The response now
            // includes presentation_name on each entry, so the operator
            // no longer needs to fake a presentations summary list.
            if let Ok(pl) = crate::api::playlists::get_playlist(&pl_id).await {
                context_title.set(pl.name.clone());
                selected_playlist.set(Some(pl));
            }
            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                playlists.set(pls);
            }
        });
```

- [ ] **Step 3: Remove `rebuild_playlist_presentations_with_signal` helper**

In `crates/presenter-ui/src/components/presentation_list.rs`, delete lines 605-623 (the `fn rebuild_playlist_presentations_with_signal(...)` definition). Then remove every CALL to it. Find calls:

```bash
grep -n "rebuild_playlist_presentations_with_signal" crates/presenter-ui/src/components/presentation_list.rs
```

Each call follows a `replace_entries` success and looks like:
```rust
    if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
        selected_playlist.set(Some(updated.clone()));
        rebuild_playlist_presentations_with_signal(presentations, &updated);  // ← delete this line
    }
```

Remove that line at every callsite (line 114, 235, 288, 321, 420, 478 — verify with grep). Also remove `let presentations = ctx.presentations;` lines that exist ONLY to feed the helper (look for unused `presentations` bindings in those closures and remove them too — the compiler will flag unused bindings with `-D warnings`).

- [ ] **Step 4: Build and run**

```bash
cd crates/presenter-ui
cargo build --target wasm32-unknown-unknown 2>&1 | tail -10
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -15
cargo test --target x86_64-unknown-linux-gnu 2>&1 | tail -10
cd ../..
```

Expected: clean build, clippy passes, all `presenter-ui` tests still pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/presentation_list.rs crates/presenter-ui/src/pages/operator.rs
git commit -m "fix(ui): operator reads presentation_name from playlist entry (#worship-pp-fixes)

Removes the rebuild_playlist_presentations_with_signal helper that
faked a presentations list with empty names every time a playlist
was selected. The lookup that depended on it returned an empty
string, so playlist entries rendered with no visible name.
Now the entry's presentation_name (server-enriched) is used directly."
```

---

## Task 5: Wrap worship-pp slide regions in `.stage-pp__slides-area`

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_pp.rs:132-194`

- [ ] **Step 1: Wrap the six regions in slides-area**

In `crates/presenter-ui/src/components/stage/worship_pp.rs`, replace the whole `view! { ... }` block (lines 132-195). The change: insert `<div class="stage-pp__slides-area">` around the six regions, before `.stage-pp__playlist-sidebar`. StatusBar stays as the last child of `.stage-container`.

```rust
    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage-pp__slides-area">
                <div class="stage__current-group">
                    <span class="stage__debug-label">"current-group"</span>
                    <div node_ref=current_group_ref class="stage__group-pill" style=current_group_style>
                        {current_group_text}
                    </div>
                </div>

                <div class="stage__current-song">
                    <span class="stage__debug-label">"current-song"</span>
                    <div node_ref=current_song_ref class="stage__song-name-text">
                        {current_song_text}
                    </div>
                </div>

                <div class="stage__current-slide">
                    <span class="stage__debug-label">"current-slide"</span>
                    <div node_ref=current_text_ref class="stage__slide-text">
                        {current_text}
                    </div>
                </div>

                <div class="stage__next-group">
                    <span class="stage__debug-label">"next-group"</span>
                    <div node_ref=next_group_ref class="stage__group-pill" style=next_group_style>
                        {next_group_text}
                    </div>
                </div>

                <div class="stage__next-song">
                    <span class="stage__debug-label">"next-song"</span>
                    <div node_ref=next_song_ref class="stage__song-name-text">
                        {next_song_text}
                    </div>
                </div>

                <div class="stage__next-slide">
                    <span class="stage__debug-label">"next-slide"</span>
                    <div node_ref=next_text_ref class="stage__slide-text">
                        {next_text}
                    </div>
                </div>
            </div>

            <div class="stage-pp__playlist-sidebar">
                <span class="stage__debug-label">"playlist-sidebar"</span>
                <For
                    each=playlist_entries
                    key=|entry| entry.name.clone()
                    children=move |entry| {
                        let class = if entry.is_active {
                            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                        } else {
                            "stage-pp__playlist-entry"
                        };
                        let display_name = clean_song_name(&entry.name);
                        view! { <div class=class>{display_name}</div> }
                    }
                />
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
```

- [ ] **Step 2: Build the WASM crate**

```bash
cd crates/presenter-ui
cargo build --target wasm32-unknown-unknown 2>&1 | tail -5
cd ../..
```

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/worship_pp.rs
git commit -m "fix(stage): wrap worship-pp regions in slides-area to stop sidebar overlap (#worship-pp-fixes)

The .stage-pp__slides-area CSS rule already constrained to the left
70% but the markup never used it as a wrapper, so the six slide
regions used full-page absolute positioning and the sidebar overlaid
them. Wrapping the regions makes their absolute positioning anchor
to slides-area instead, leaving a clean 30% column on the right
for the playlist."
```

---

## Task 6: High-contrast active-song highlight CSS

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css:364-368`

- [ ] **Step 1: Update the active-entry rule**

In `crates/presenter-ui/styles/stage.css`, replace lines 364-368:

OLD:
```css
.stage-pp__playlist-entry--active {
    background: rgba(56, 189, 248, 0.15);
    color: #f8fafc;
    font-weight: 700;
}
```

NEW:
```css
.stage-pp__playlist-entry--active {
    background: #38bdf8;
    color: #0f172a;
    font-weight: 700;
    border-left: 4px solid #0ea5e9;
    /* Compensate for the 4px border so text alignment with non-active
       rows stays consistent. Original padding-left from .stage-pp__playlist-entry
       is 0.6rem; subtract the border width. */
    padding-left: calc(0.6rem - 4px);
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "fix(stage): high-contrast active-song highlight on worship-pp playlist (#worship-pp-fixes)

The 15%-opacity tint was invisible from projector distance. Now
uses a solid sky-blue pill with dark text and a 4px accent bar
on the left edge — pops at a glance even on a dim screen."
```

---

## Task 7: E2E tests for operator playlist names + worship-pp layout & highlight

**Files:**
- Modify: `tests/e2e/wasm-playlist-operations.spec.ts`
- Create: `tests/e2e/stage-worship-pp.spec.ts`

- [ ] **Step 1: Extend operator drag-drop test to assert visible name**

In `tests/e2e/wasm-playlist-operations.spec.ts`, find the test `"drop a presentation onto a playlist row appends an entry"` (line 327). Right before the final `expect(consoleErrors).toEqual([]);` at the end of that test, INSERT:

```typescript
    // Click the playlist to make it the selected playlist, so its entries
    // are shown in the presentation column.
    const playlistRow = page.locator(
      `[data-role="playlist-list"] [data-playlist-id="${targetPlaylistId}"] [data-role="playlist-item"]`,
    );
    await playlistRow.click();

    // The first presentation entry must render with a non-empty visible name.
    // Regression guard: previously the operator rebuilt presentations summaries
    // with empty strings, so the name span was blank.
    const firstEntryName = await page
      .locator(
        '[data-role="presentation-item"][data-type="presentation"] > span:first-child',
      )
      .first()
      .textContent();
    expect(
      firstEntryName?.trim(),
      "playlist entry must show a non-empty presentation name",
    ).toBeTruthy();
    expect(firstEntryName?.trim().length ?? 0).toBeGreaterThan(0);
```

The selector `[data-role="presentation-item"] > span:first-child` targets the `<span>{pres_name}</span>` at presentation_list.rs:433.

- [ ] **Step 2: Create new stage E2E test**

Create `tests/e2e/stage-worship-pp.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import { startTestServer, type TestServer } from "./helpers/test-server";

let server: TestServer;
let baseURL: string;

test.beforeAll(async () => {
  server = await startTestServer();
  baseURL = server.baseURL;
});

test.afterAll(async () => {
  await server?.stop();
});

test.describe("Stage worship-pp layout", () => {
  test("slides-area and playlist-sidebar do not overlap", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon")) consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // 1. Switch the active stage layout to worship-pp via the operator's
    //    select control (which uses the WASM API path the user would).
    await page.goto(new URL("/ui/operator", baseURL).toString());
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );
    await page.evaluate(() => {
      const sel = document.querySelector(
        '[data-role="stage-layout-select"]',
      ) as HTMLSelectElement | null;
      if (!sel) throw new Error("stage-layout-select not found");
      sel.value = "worship-pp";
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });

    // 2. Open the stage page in the same context.
    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(
      () =>
        document.body.dataset.wasmReady === "true" &&
        document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );

    // 3. Both wrapper boxes exist.
    const slidesArea = page.locator(".stage-pp__slides-area");
    const sidebar = page.locator(".stage-pp__playlist-sidebar");
    await expect(slidesArea).toBeVisible();
    await expect(sidebar).toBeVisible();

    // 4. Slides-area's right edge must be ≤ sidebar's left edge.
    const overlap = await page.evaluate(() => {
      const a = document
        .querySelector(".stage-pp__slides-area")
        ?.getBoundingClientRect();
      const b = document
        .querySelector(".stage-pp__playlist-sidebar")
        ?.getBoundingClientRect();
      if (!a || !b) return { error: "missing rect" };
      return { aRight: a.right, bLeft: b.left, overlap: a.right > b.left };
    });
    expect(
      overlap.overlap,
      `slides-area right=${overlap.aRight} sidebar left=${overlap.bLeft}`,
    ).toBe(false);

    // 5. The six slide regions live INSIDE slides-area (not as siblings).
    for (const cls of [
      ".stage__current-group",
      ".stage__current-song",
      ".stage__current-slide",
      ".stage__next-group",
      ".stage__next-song",
      ".stage__next-slide",
    ]) {
      const inside = await page.evaluate((selector) => {
        const el = document.querySelector(selector);
        return !!el?.closest(".stage-pp__slides-area");
      }, cls);
      expect(inside, `${cls} should be inside slides-area`).toBe(true);
    }

    expect(consoleErrors).toEqual([]);
  });

  test("active playlist entry has high-contrast background distinct from inactive", async ({
    page,
  }) => {
    if (!server) throw new Error("test server not started");

    // Seed: create a playlist with one entry, set its presentation as active on stage.
    const presResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = await presResp.json();
    const presentation = libs[0]?.presentations?.[0];
    if (!presentation) {
      test.skip(true, "test fixture has no presentations");
      return;
    }
    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      { data: { name: "Highlight Test", showInDashboard: true } },
    );
    const playlist = await playlistResp.json();
    await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [
            { type: "presentation", presentationId: presentation.id },
          ],
        },
      },
    );
    // Trigger the presentation on stage so it becomes the active one.
    await page.request.post(
      new URL("/state/stage/trigger-presentation", baseURL).toString(),
      { data: { presentationId: presentation.id, playlistId: playlist.id } },
    );

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );

    // Wait for the active row to appear with the active class.
    const active = page.locator(".stage-pp__playlist-entry--active").first();
    await expect(active).toBeVisible({ timeout: 15_000 });

    // Background of active row must NOT match background of an inactive row
    // (or the absence of one — if there's only one entry, just check it has
    // a non-transparent background).
    const colors = await page.evaluate(() => {
      const a = document.querySelector(
        ".stage-pp__playlist-entry--active",
      ) as HTMLElement | null;
      const inactive = Array.from(
        document.querySelectorAll(".stage-pp__playlist-entry"),
      ).find(
        (e) => !e.classList.contains("stage-pp__playlist-entry--active"),
      ) as HTMLElement | null;
      return {
        active: a ? getComputedStyle(a).backgroundColor : null,
        inactive: inactive ? getComputedStyle(inactive).backgroundColor : null,
      };
    });
    expect(colors.active).toBeTruthy();
    // Active should not be fully transparent (rgba(0, 0, 0, 0)).
    expect(colors.active).not.toBe("rgba(0, 0, 0, 0)");
    expect(colors.active).not.toBe("transparent");
    if (colors.inactive) {
      expect(colors.active).not.toBe(colors.inactive);
    }

    // Cleanup
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );
  });
});
```

If the seed data setup or the trigger endpoint shape differs (verify by running `grep -rn "trigger-presentation\|trigger_presentation" crates/presenter-server/src/router/` to find the actual endpoint), adapt the seed-section calls. The visibility, overlap, and background-color assertions stay as-is.

If `tests/e2e/helpers/test-server.ts` doesn't expose `startTestServer`/`TestServer`, mirror the import pattern used by `tests/e2e/wasm-playlist-operations.spec.ts` (run `head -20 tests/e2e/wasm-playlist-operations.spec.ts` to see the actual imports and fixtures used).

- [ ] **Step 3: Run E2E tests locally to confirm they would pass**

Run only the new and updated tests in shard mode:

```bash
npm run test:playwright -- --grep "drop a presentation onto a playlist row|Stage worship-pp layout" 2>&1 | tail -40
```

Expected: both tests pass against the local dev server. If the server isn't running, start it: `cargo run -p presenter-server &` then re-run.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/wasm-playlist-operations.spec.ts tests/e2e/stage-worship-pp.spec.ts
git commit -m "test(e2e): playlist names + worship-pp layout & active highlight (#worship-pp-fixes)

Three regression guards for #worship-pp-fixes:
- After drag-drop, click the playlist row and assert the rendered
  entry shows a non-empty presentation name
- Stage worship-pp: assert slides-area and playlist-sidebar do not
  overlap and the six regions live inside slides-area
- Stage worship-pp: assert the active entry has a non-transparent
  background distinct from inactive entries"
```

---

## Task 8: Local checks + push + monitor pipeline CI

- [ ] **Step 1: Run local lint/format/test**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -15
cargo test --workspace 2>&1 | tail -10
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -15 && cargo test --target x86_64-unknown-linux-gnu 2>&1 | tail -10 && cd ../..
```

Expected: every step exits 0, clippy emits no warnings.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor pipeline**

Find the pipeline run for the latest SHA:

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status,conclusion --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
```

Wait for completion using a single background poll (per `ci-monitoring.md` — do NOT poll repeatedly):

```bash
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Expected: ALL jobs `success` (Branch Sync, Format, Clippy, Test, Build, Code Coverage, Mutation Testing, Playwright E2E 1/3 + 2/3 + 3/3, Merge E2E Reports, Deploy to Dev). If anything failed, fetch the failed job log (`gh run view <RUN_ID> --log-failed`), fix the root cause in ONE commit, push, monitor again.

---

## Task 9: Verify on dev + open PR

- [ ] **Step 1: Verify dev deploy**

```bash
curl -s http://10.77.8.134:8080/healthz | head
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.37"}`.

Then verify the three fixes in a real browser via Playwright MCP (or by hand):

- Open `http://10.77.8.134:8080/ui/operator`. In a freshly created playlist, drag any presentation into it. Click the playlist row. Expected: presentation name visible (not empty span).
- Open `http://10.77.8.134:8080/stage`. Switch operator to worship-pp layout. Expected: slides on the left don't overlap the playlist sidebar on the right. Active song row in sidebar is a solid blue pill with a left accent bar (clearly distinct from the others).
- Browser console on both pages: zero errors / warnings.

- [ ] **Step 2: Open PR**

```bash
gh pr create --base main --head dev --title "fix(stage): worship-pp regressions — overlap, highlight, operator names" --body "$(cat <<'EOF'
## Summary
Three regressions reported on the worship-pp stage and operator playlist after PR #268:

- **Stage layout overlap** — slides and playlist sidebar overlapped. Fixed by wrapping the six slide regions in `.stage-pp__slides-area` so their absolute positioning anchors to the left 70% column.
- **Active-song highlight invisible from projector** — replaced the 15%-opacity tint with a solid sky-blue pill, dark text, and a 4px accent bar on the left edge.
- **Operator playlist entries showed empty names** — server now enriches the playlist response with `presentation_name` on each `Presentation` entry (using the existing `fetch_presentation_names_for_playlist` helper). Operator reads it directly. Removed the obsolete `rebuild_playlist_presentations_with_signal` helper and its callsites.

## Test plan
- [ ] CI green on push and PR runs (incl. mutation testing)
- [ ] Operator drag-drop E2E asserts presentation name visible
- [ ] Stage worship-pp E2E asserts no overlap + active highlight has distinct background
- [ ] 3 new core/serde unit tests for `presentation_name`
- [ ] 2 new server integration tests for response enrichment
- [ ] Manual: open dev stage with worship-pp + active playlist, verify visible highlight at projector distance
- [ ] Browser console: zero errors / warnings on operator and stage

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Wait for PR pipeline (separate from push pipeline)**

```bash
gh run list --branch dev --limit 5 --json databaseId,name,event,headSha,status --jq '.[] | select(.name == "Pipeline" and .event == "pull_request")' | head -1
```

Monitor the PR run with the same `sleep 1500 && gh run view` pattern. Wait for ALL jobs green incl. mutation testing.

- [ ] **Step 4: Verify PR is mergeable + clean**

```bash
gh pr view <PR_NUMBER> --json mergeable,mergeStateStatus,url
```

Expected: `{"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN", ...}`.

- [ ] **Step 5: Provide PR URL to user, wait for explicit "merge it"**

Per `pr-merge-policy.md`, do NOT merge. Send the completion report with the PR URL and wait.

---

## Task 10: Post-merge production verification (after user merges)

- [ ] **Step 1: Watch the post-merge main pipeline + production deploy**

```bash
gh run list --branch main --limit 3 --json databaseId,name,event,status,conclusion --jq '.[] | select(.event == "push")' | head -1
```

Background-poll: `sleep 1500 && gh run view <RUN_ID> ...`. Wait for ALL jobs green incl. `Deploy to Production`.

- [ ] **Step 2: Verify production**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"channel":"release","status":"ok","version":"0.4.37"}`.

Then re-do the same Playwright/manual checks from Task 9 step 1 against `http://10.77.9.205/...`. Capture: drag-drop adds entry with visible name; stage worship-pp shows no overlap and obvious active highlight.

- [ ] **Step 3: Final completion report**

Send the user a short report (per `completion-report.md`): URLs, audits clean, what shipped, no follow-up question.

---

## Verification Summary

| Check | How |
|-------|-----|
| Field round-trips through serde | `cargo test -p presenter-core presentation_name_tests` (3 tests) |
| Server response carries name | `cargo test -p presenter-server router::tests::get_playlist_response_includes_presentation_name list_playlists_response_includes_presentation_names` |
| Operator entries show names | E2E `wasm-playlist-operations.spec.ts` after-drop assertion |
| Stage layout no overlap | E2E `stage-worship-pp.spec.ts` `getBoundingClientRect` comparison |
| Active highlight visible | E2E `stage-worship-pp.spec.ts` `getComputedStyle` background-color check |
| Console clean | All E2E tests assert `consoleErrors === []` |
| Worship-snv unaffected | Existing worship-snv E2E tests still pass |
| Mutation testing | CI mutation job stays green |
