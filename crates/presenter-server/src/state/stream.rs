//! Stream-graphics show-state manager (epic #718, ADR 0009 §3/§6; PR-3 #706).
//!
//! [`StreamManager`] is the in-memory cache of each output's ACTIVATION
//! snapshot — the exclusive base scene plus the independent set of active
//! overlays — mirroring what the #705 repository persists in
//! `stream_outputs.active_scene_id` + `stream_scenes.is_active`. The DB is the
//! source of truth; the cache is a warm read-through so the WASM output page
//! and the Companion variable text (#7/#11) can read the current show-state
//! without a DB round-trip, and so a cold OBS load / server restart restores
//! the last look with zero Companion action (ADR decision #3).
//!
//! # Lock discipline
//! The manager owns ONE `RwLock` (its cache map). It is NEVER held across a
//! repository `await`: every mutation persists through the repository FIRST
//! (outside the guard), then takes the lock only to insert the fresh snapshot,
//! then drops it before publishing. No nesting with any other `AppState` lock —
//! see the lock inventory in `state/mod.rs`.
//!
//! # Event split (`.claude/rules/stream-graphics.md`)
//! Activation (`set/overlay/clear`) publishes [`LiveEvent::StreamState`] — it
//! does NOT bump `config_revision`, so clients apply it in place. CONFIG writes
//! (output/scene/element CRUD + reorder) bump `config_revision` and publish
//! [`LiveEvent::StreamConfigChanged`] via [`AppState::stream_config_write_notify`],
//! prompting a full def refetch.

use super::AppState;
use crate::live::LiveEvent;
use presenter_core::StreamShowState;
use presenter_persistence::Repository;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory per-output show-state cache with its own lock (ADR §3).
#[derive(Clone)]
pub(crate) struct StreamManager {
    cache: Arc<RwLock<HashMap<String, StreamShowState>>>,
}

impl StreamManager {
    pub(crate) fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Read-through getter: return the cached snapshot, else hydrate it from
    /// the repository (cold start / a fresh manager after a simulated restart)
    /// and cache it. The lock is never held across the repository await.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn show_state(
        &self,
        repo: &Repository,
        slug: &str,
    ) -> anyhow::Result<StreamShowState> {
        {
            let cache = self.cache.read().await;
            if let Some(state) = cache.get(slug) {
                return Ok(state.clone());
            }
        }
        let state = repo.get_stream_show_state(slug).await?;
        self.cache
            .write()
            .await
            .insert(slug.to_string(), state.clone());
        Ok(state)
    }

    /// Re-read the authoritative snapshot from the repository after a persisted
    /// change and refresh the cache. Returns the fresh snapshot so the caller
    /// can publish it. The repository await happens before the lock is taken.
    async fn refresh(&self, repo: &Repository, slug: &str) -> anyhow::Result<StreamShowState> {
        let state = repo.get_stream_show_state(slug).await?;
        self.cache
            .write()
            .await
            .insert(slug.to_string(), state.clone());
        Ok(state)
    }

    /// Drop an output's cached snapshot — called when the output is deleted so
    /// a later read hydrate-misses (and surfaces the repository's NotFound)
    /// instead of returning a stale entry.
    async fn evict(&self, slug: &str) {
        self.cache.write().await.remove(slug);
    }
}

impl AppState {
    /// Exclusive base activation (`scene_id = None` clears the base). Persists
    /// via the repository (which validates base-kind + output membership),
    /// refreshes the cache, and publishes [`LiveEvent::StreamState`].
    pub(crate) async fn stream_activate_scene(
        &self,
        slug: &str,
        scene_id: Option<i64>,
    ) -> anyhow::Result<StreamShowState> {
        self.repository.set_active_scene(slug, scene_id).await?;
        self.publish_stream_state(slug).await
    }

    /// Turn one overlay scene on/off (independent of the base). Persists,
    /// refreshes the cache, publishes [`LiveEvent::StreamState`].
    pub(crate) async fn stream_set_overlay(
        &self,
        slug: &str,
        scene_id: i64,
        active: bool,
    ) -> anyhow::Result<StreamShowState> {
        self.repository
            .set_overlay_active(slug, scene_id, active)
            .await?;
        self.publish_stream_state(slug).await
    }

    /// Clear the whole output — base to none and every overlay off. Persists,
    /// refreshes the cache, publishes [`LiveEvent::StreamState`].
    pub(crate) async fn stream_clear(&self, slug: &str) -> anyhow::Result<StreamShowState> {
        self.repository.clear_stream_output(slug).await?;
        self.publish_stream_state(slug).await
    }

    /// Refresh the cache from the DB and broadcast the current show-state.
    async fn publish_stream_state(&self, slug: &str) -> anyhow::Result<StreamShowState> {
        let state = self.stream.refresh(&self.repository, slug).await?;
        self.live_hub.publish(LiveEvent::StreamState {
            output: slug.to_string(),
            active_scene_id: state.active_scene_id,
            active_overlay_ids: state.active_overlay_ids.clone(),
            config_revision: state.config_revision,
        });
        Ok(state)
    }

    /// Called by the #707 config-mutating handlers AFTER they persist an
    /// output/scene/element change (which bumped `config_revision`): refresh the
    /// cache and publish [`LiveEvent::StreamConfigChanged`] so clients refetch
    /// the full def.
    pub(crate) async fn stream_config_write_notify(
        &self,
        slug: &str,
    ) -> anyhow::Result<StreamShowState> {
        let state = self.stream.refresh(&self.repository, slug).await?;
        self.live_hub.publish(LiveEvent::StreamConfigChanged {
            output: slug.to_string(),
            config_revision: state.config_revision,
        });
        Ok(state)
    }

    /// Current show-state for an output (read-through cache). Foundation for the
    /// Companion variable text (#7) and the WASM output page (#11); consumed by
    /// the #706 tests in this PR.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn stream_show_state(&self, slug: &str) -> anyhow::Result<StreamShowState> {
        self.stream.show_state(&self.repository, slug).await
    }

    /// Evict an output's cached show-state — called by the delete-output
    /// handler after the repository row is gone, so the cache never serves a
    /// stale snapshot for a deleted output.
    pub(crate) async fn stream_evict_output(&self, slug: &str) {
        self.stream.evict(slug).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::SceneKind;

    /// Create a fresh, uniquely-slugged output. The process-shared in-memory
    /// DB (`Repository::connect_in_memory` = `sqlite::memory:?cache=shared`,
    /// `.claude/rules/stream-graphics.md`) means every test must own its slug
    /// rather than share the seeded `stream` output, or activations race.
    async fn seed_output(state: &AppState, slug: &str) {
        state
            .repository()
            .create_stream_output(slug, "Test")
            .await
            .expect("create output");
    }

    /// Create a fresh output plus a base + an overlay scene on it.
    /// Returns `(base_id, overlay_id)`.
    async fn seed_scenes(state: &AppState, slug: &str) -> (i64, i64) {
        seed_output(state, slug).await;
        let base = state
            .repository()
            .create_stream_scene(slug, "Base", SceneKind::Base)
            .await
            .expect("create base scene");
        let overlay = state
            .repository()
            .create_stream_scene(slug, "Overlay", SceneKind::Overlay)
            .await
            .expect("create overlay scene");
        (base.id, overlay.id)
    }

    #[tokio::test]
    async fn activate_base_is_exclusive() {
        let state = AppState::in_memory().await.unwrap();
        seed_output(&state, "s706-exclusive").await;
        let base_a = state
            .repository()
            .create_stream_scene("s706-exclusive", "Base A", SceneKind::Base)
            .await
            .unwrap();
        let base_b = state
            .repository()
            .create_stream_scene("s706-exclusive", "Base B", SceneKind::Base)
            .await
            .unwrap();

        let after_a = state
            .stream_activate_scene("s706-exclusive", Some(base_a.id))
            .await
            .unwrap();
        assert_eq!(after_a.active_scene_id, Some(base_a.id));

        // Activating a second base REPLACES the first (exclusive).
        let after_b = state
            .stream_activate_scene("s706-exclusive", Some(base_b.id))
            .await
            .unwrap();
        assert_eq!(after_b.active_scene_id, Some(base_b.id));
        assert_eq!(
            state
                .stream_show_state("s706-exclusive")
                .await
                .unwrap()
                .active_scene_id,
            Some(base_b.id)
        );
    }

    #[tokio::test]
    async fn overlays_are_independent_of_base() {
        let state = AppState::in_memory().await.unwrap();
        let (base, overlay) = seed_scenes(&state, "s706-overlays").await;

        state
            .stream_activate_scene("s706-overlays", Some(base))
            .await
            .unwrap();
        let with_overlay = state
            .stream_set_overlay("s706-overlays", overlay, true)
            .await
            .unwrap();
        assert_eq!(with_overlay.active_scene_id, Some(base));
        assert_eq!(with_overlay.active_overlay_ids, vec![overlay]);

        // Turning the overlay off leaves the base untouched.
        let without = state
            .stream_set_overlay("s706-overlays", overlay, false)
            .await
            .unwrap();
        assert_eq!(without.active_scene_id, Some(base));
        assert!(without.active_overlay_ids.is_empty());
    }

    #[tokio::test]
    async fn clear_resets_base_and_overlays() {
        let state = AppState::in_memory().await.unwrap();
        let (base, overlay) = seed_scenes(&state, "s706-clear").await;
        state
            .stream_activate_scene("s706-clear", Some(base))
            .await
            .unwrap();
        state
            .stream_set_overlay("s706-clear", overlay, true)
            .await
            .unwrap();

        let cleared = state.stream_clear("s706-clear").await.unwrap();
        assert_eq!(cleared.active_scene_id, None);
        assert!(cleared.active_overlay_ids.is_empty());
    }

    #[tokio::test]
    async fn state_survives_a_simulated_restart() {
        let state = AppState::in_memory().await.unwrap();
        let (base, overlay) = seed_scenes(&state, "s706-restart").await;
        state
            .stream_activate_scene("s706-restart", Some(base))
            .await
            .unwrap();
        state
            .stream_set_overlay("s706-restart", overlay, true)
            .await
            .unwrap();

        // A brand-new manager (empty cache) over the SAME DB hydrates the
        // persisted show-state — the cold-OBS-load / restart guarantee.
        let fresh = StreamManager::new();
        let restored = fresh
            .show_state(state.repository(), "s706-restart")
            .await
            .unwrap();
        assert_eq!(restored.active_scene_id, Some(base));
        assert_eq!(restored.active_overlay_ids, vec![overlay]);
    }

    #[tokio::test]
    async fn activation_publishes_exactly_one_stream_state_event() {
        let state = AppState::in_memory().await.unwrap();
        let (base, _overlay) = seed_scenes(&state, "s706-oneevent").await;
        // Snapshot the revision AFTER the scene creates (which each bumped it);
        // activation must leave it unchanged.
        let before = state
            .stream_show_state("s706-oneevent")
            .await
            .unwrap()
            .config_revision;
        let mut rx = state.live_hub().subscribe();

        state
            .stream_activate_scene("s706-oneevent", Some(base))
            .await
            .unwrap();

        match rx.try_recv().expect("exactly one event") {
            LiveEvent::StreamState {
                output,
                active_scene_id,
                config_revision,
                ..
            } => {
                assert_eq!(output, "s706-oneevent");
                assert_eq!(active_scene_id, Some(base));
                // Activation must NOT bump config_revision (stream-graphics rule).
                assert_eq!(config_revision, before);
            }
            other => panic!("expected StreamState, got {other:?}"),
        }
        // No second event queued.
        assert!(
            rx.try_recv().is_err(),
            "activation must publish exactly one event"
        );
    }

    #[tokio::test]
    async fn config_write_notify_publishes_config_changed() {
        let state = AppState::in_memory().await.unwrap();
        seed_output(&state, "s706-cfg").await;
        let mut rx = state.live_hub().subscribe();

        // A scene create bumps config_revision (repository side); the notify
        // publishes StreamConfigChanged carrying the new revision.
        state
            .repository()
            .create_stream_scene("s706-cfg", "Base", SceneKind::Base)
            .await
            .unwrap();
        state.stream_config_write_notify("s706-cfg").await.unwrap();

        match rx.try_recv().expect("one event") {
            LiveEvent::StreamConfigChanged {
                output,
                config_revision,
            } => {
                assert_eq!(output, "s706-cfg");
                assert!(
                    config_revision >= 1,
                    "a config write must have bumped the revision"
                );
            }
            other => panic!("expected StreamConfigChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wrong_kind_activation_surfaces_typed_error_without_panic() {
        let state = AppState::in_memory().await.unwrap();
        let (_base, overlay) = seed_scenes(&state, "s706-wrongkind").await;

        // Activating an OVERLAY scene as the base is a 422 Invalid, not a panic.
        let err = state
            .stream_activate_scene("s706-wrongkind", Some(overlay))
            .await
            .expect_err("activating an overlay as base must be refused");
        let repo_err = err
            .downcast_ref::<presenter_persistence::RepositoryError>()
            .expect("typed RepositoryError");
        assert!(
            matches!(repo_err, presenter_persistence::RepositoryError::Invalid(_)),
            "wrong-kind activation must be RepositoryError::Invalid, got {repo_err:?}"
        );
    }

    #[tokio::test]
    async fn evict_drops_cached_show_state() {
        let state = AppState::in_memory().await.unwrap();
        let (base, _overlay) = seed_scenes(&state, "s706-evict").await;
        // Cache the output's show-state via an activation.
        state
            .stream_activate_scene("s706-evict", Some(base))
            .await
            .unwrap();
        // Delete the output and evict the cache (the delete_output handler path).
        state
            .repository()
            .delete_stream_output("s706-evict")
            .await
            .unwrap();
        state.stream_evict_output("s706-evict").await;
        // A read now hydrate-misses and surfaces the repository's NotFound —
        // never a stale cached snapshot.
        let err = state
            .stream_show_state("s706-evict")
            .await
            .expect_err("a deleted output must not read from a stale cache");
        assert!(
            err.downcast_ref::<presenter_persistence::RepositoryError>()
                .is_some(),
            "stale-cache read must surface the repository's typed NotFound"
        );
    }

    #[tokio::test]
    async fn foreign_output_activation_surfaces_typed_error() {
        let state = AppState::in_memory().await.unwrap();
        // A scene on a SECOND output must not be activatable on `stream`.
        state
            .repository()
            .create_stream_output("other", "Other")
            .await
            .unwrap();
        let foreign = state
            .repository()
            .create_stream_scene("other", "Base", SceneKind::Base)
            .await
            .unwrap();

        let err = state
            .stream_activate_scene("stream", Some(foreign.id))
            .await
            .expect_err("activating a foreign output's scene must be refused");
        assert!(
            err.downcast_ref::<presenter_persistence::RepositoryError>()
                .is_some(),
            "must surface a typed RepositoryError, not a bare anyhow"
        );
    }
}
