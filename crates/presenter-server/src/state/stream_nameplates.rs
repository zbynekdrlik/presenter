//! Lower-third nameplate runtime show-state (#779, epic #718).
//!
//! [`NameplateManager`] holds, per output, WHICH plate is currently on air (an
//! [`ActiveNameplate`]) plus a monotonic `seq` counter. This is IN-MEMORY only —
//! a server restart must never pop a plate back on air (ADR §3, same rationale as
//! the countdown/overlay show-state), and it is NOT a config write so it never
//! bumps `config_revision`. Every show/hide broadcasts
//! [`LiveEvent::StreamNameplate`] so the output page animates and Companion turns
//! its feedback on/off.
//!
//! # Auto-hide
//! Showing a plate optionally schedules a server-side hide after the max
//! `auto_hide_s` over the output's `lower_third` elements (0 = stay until
//! hidden). The task is guarded by the plate's `seq`: a newer show (or an
//! explicit hide) bumps the active seq, so the older auto-hide finds a mismatch
//! and no-ops. Running it server-side (not client-side) means Companion feedback
//! turns off too when the plate auto-hides.
//!
//! # Lock discipline
//! One `RwLock` (the per-output slot map). It is NEVER held across a repository
//! or stage-snapshot `await`: texts are resolved FIRST (outside the guard), then
//! the lock is taken only to mutate the slot, then dropped before publishing.

use super::AppState;
use crate::live::LiveEvent;
use presenter_core::{ActiveNameplate, NameplateSource, StreamElementProps};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// State-layer refusal for a nameplate show (no DB touched) — a wrong-layer
/// dependency on `RepositoryError` is avoided per `repository-error-pattern.md`;
/// the router downcasts this to a 409.
#[derive(Debug, thiserror::Error)]
pub(crate) enum NameplateError {
    #[error("žiadna pieseň nie je práve aktívna — najprv spusti pieseň")]
    EmptySong,
}

/// One output's plate show-state slot.
#[derive(Clone, Default)]
struct NameplateSlot {
    active: Option<ActiveNameplate>,
    /// Monotonic generation for the next show — guards the auto-hide task.
    next_seq: u64,
}

/// In-memory per-output nameplate show-state cache (own lock, ADR §3).
#[derive(Clone)]
pub(crate) struct NameplateManager {
    slots: Arc<RwLock<HashMap<String, NameplateSlot>>>,
}

impl NameplateManager {
    pub(crate) fn new() -> Self {
        Self {
            slots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Allocate a fresh `seq` for `slug` (bumps the per-output counter).
    async fn allocate_seq(&self, slug: &str) -> u64 {
        let mut slots = self.slots.write().await;
        let slot = slots.entry(slug.to_string()).or_default();
        let seq = slot.next_seq;
        slot.next_seq += 1;
        seq
    }

    /// Store the active plate (or clear it) for `slug`.
    async fn set_active(&self, slug: &str, active: Option<ActiveNameplate>) {
        let mut slots = self.slots.write().await;
        slots.entry(slug.to_string()).or_default().active = active;
    }

    /// The current on-air plate for `slug` (None = nothing on air).
    async fn active(&self, slug: &str) -> Option<ActiveNameplate> {
        self.slots
            .read()
            .await
            .get(slug)
            .and_then(|s| s.active.clone())
    }

    /// Drop an output's slot — called when the output is deleted.
    async fn evict(&self, slug: &str) {
        self.slots.write().await.remove(slug);
    }
}

impl AppState {
    /// Show a PERSON plate: resolve its texts from the repository row, activate,
    /// broadcast, and schedule auto-hide. Returns the new active state.
    pub(crate) async fn stream_nameplate_show_person(
        &self,
        slug: &str,
        nameplate_id: i64,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        // Resolve BEFORE taking any lock (repository await).
        let plate = self.repository.get_stream_nameplate(nameplate_id).await?;
        self.activate_nameplate(
            slug,
            NameplateSource::Person,
            Some(nameplate_id),
            plate.primary_text,
            plate.secondary_text,
        )
        .await
    }

    /// Show the virtual SONG plate: resolve the current song title + library from
    /// the live stage snapshot. An empty song is a 409 ([`NameplateError::EmptySong`]).
    pub(crate) async fn stream_nameplate_show_song(
        &self,
        slug: &str,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        let (primary, secondary) = self.resolve_song_texts().await?;
        self.activate_nameplate(slug, NameplateSource::Song, None, primary, secondary)
            .await
    }

    /// Hide whatever plate is on air (explicit hide / Companion hide). Bumps the
    /// active seq so a pending auto-hide from the last show no-ops, then broadcasts.
    pub(crate) async fn stream_nameplate_hide(
        &self,
        slug: &str,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        // Bump the seq so any in-flight auto-hide sees a mismatch.
        let _ = self.nameplates.allocate_seq(slug).await;
        self.nameplates.set_active(slug, None).await;
        self.publish_nameplate(slug, None);
        Ok(None)
    }

    /// Toggle a PERSON plate: hide it if it is the one on air, else show it.
    pub(crate) async fn stream_nameplate_toggle_person(
        &self,
        slug: &str,
        nameplate_id: i64,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        if self.nameplate_person_on_air(slug, nameplate_id).await {
            self.stream_nameplate_hide(slug).await
        } else {
            self.stream_nameplate_show_person(slug, nameplate_id).await
        }
    }

    /// Toggle the SONG plate: hide it if it is on air, else show it.
    pub(crate) async fn stream_nameplate_toggle_song(
        &self,
        slug: &str,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        let song_on_air = matches!(
            self.nameplates.active(slug).await,
            Some(ActiveNameplate {
                source: NameplateSource::Song,
                ..
            })
        );
        if song_on_air {
            self.stream_nameplate_hide(slug).await
        } else {
            self.stream_nameplate_show_song(slug).await
        }
    }

    /// The current on-air plate for cold OBS load / editor + Companion sync.
    pub(crate) async fn stream_nameplate_active(&self, slug: &str) -> Option<ActiveNameplate> {
        self.nameplates.active(slug).await
    }

    /// Broadcast [`LiveEvent::StreamNameplatesChanged`] after a plate-list CRUD
    /// so the editor + Companion refetch the list (choices / variables / presets).
    pub(crate) fn stream_nameplates_changed(&self, slug: &str) {
        self.live_hub.publish(LiveEvent::StreamNameplatesChanged {
            output: slug.to_string(),
        });
    }

    /// Drop an output's cached plate show-state (delete-output path).
    pub(crate) async fn stream_nameplate_evict(&self, slug: &str) {
        self.nameplates.evict(slug).await;
    }

    // ---- internals --------------------------------------------------------

    /// Common show path: allocate a seq, store, broadcast, schedule auto-hide.
    async fn activate_nameplate(
        &self,
        slug: &str,
        source: NameplateSource,
        nameplate_id: Option<i64>,
        primary: String,
        secondary: String,
    ) -> anyhow::Result<Option<ActiveNameplate>> {
        let seq = self.nameplates.allocate_seq(slug).await;
        let active = ActiveNameplate {
            source,
            nameplate_id,
            primary,
            secondary,
            seq,
        };
        self.nameplates.set_active(slug, Some(active.clone())).await;
        self.publish_nameplate(slug, Some(active.clone()));
        self.schedule_auto_hide(slug, seq).await;
        Ok(Some(active))
    }

    /// Broadcast the current plate show-state for one output.
    fn publish_nameplate(&self, slug: &str, active: Option<ActiveNameplate>) {
        self.live_hub.publish(LiveEvent::StreamNameplate {
            output: slug.to_string(),
            active,
        });
    }

    /// True when a specific PERSON plate is the one currently on air.
    async fn nameplate_person_on_air(&self, slug: &str, nameplate_id: i64) -> bool {
        matches!(
            self.nameplates.active(slug).await,
            Some(ActiveNameplate {
                source: NameplateSource::Person,
                nameplate_id: Some(id),
                ..
            }) if id == nameplate_id
        )
    }

    /// Resolve the song plate's texts (primary = song/presentation title,
    /// secondary = library) from the layout-independent camera-crew stage
    /// snapshot (the same worship content the stream lyrics element consumes).
    async fn resolve_song_texts(&self) -> anyhow::Result<(String, String)> {
        let snapshot = self.stage_display_snapshot("camera-crew").await?;
        let snapshot = snapshot.ok_or(NameplateError::EmptySong)?;
        let primary = snapshot
            .song_name
            .filter(|s| !s.trim().is_empty())
            .or(snapshot.presentation_name)
            .unwrap_or_default();
        if primary.trim().is_empty() {
            return Err(NameplateError::EmptySong.into());
        }
        let secondary = snapshot.library_name.unwrap_or_default();
        Ok((primary, secondary))
    }

    /// Schedule the server-side auto-hide for a just-shown plate. The delay is
    /// the max `auto_hide_s` over the output's `lower_third` elements; 0 = no
    /// auto-hide. The spawned task hides ONLY if the active seq still matches.
    async fn schedule_auto_hide(&self, slug: &str, seq: u64) {
        let auto_hide_s = self.max_auto_hide_s(slug).await;
        if auto_hide_s == 0 {
            return;
        }
        let state = self.clone();
        let slug = slug.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(auto_hide_s as u64)).await;
            state.auto_hide_if_current(&slug, seq).await;
        });
    }

    /// Hide the plate iff it still carries `seq` (a newer show / explicit hide
    /// bumped the seq, so this older auto-hide is a no-op).
    async fn auto_hide_if_current(&self, slug: &str, seq: u64) {
        let still_current = self
            .nameplates
            .active(slug)
            .await
            .map(|a| a.seq == seq)
            .unwrap_or(false);
        if still_current {
            self.nameplates.set_active(slug, None).await;
            self.publish_nameplate(slug, None);
        }
    }

    /// The max `auto_hide_s` over the output's `lower_third` elements (0 when the
    /// output has none, or all are 0 = stay until hidden). A failed def read
    /// degrades to 0 (no auto-hide) rather than failing the show.
    async fn max_auto_hide_s(&self, slug: &str) -> u32 {
        let Ok(def) = self.repository.load_output_def(slug).await else {
            return 0;
        };
        def.scenes
            .iter()
            .flat_map(|s| &s.elements)
            .filter_map(|e| match &e.props {
                StreamElementProps::LowerThird { auto_hide_s, .. } => Some(*auto_hide_s),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{
        AnimationPreset, Frame, SceneKind, StreamElementProps, TextAlign, TextStyle,
    };

    fn text_style() -> TextStyle {
        TextStyle {
            font_family: "Inter".to_string(),
            size_pct: 6.0,
            color: "#ffffff".to_string(),
            weight: 700,
            align: TextAlign::Left,
            line_height: 1.1,
            shadow: None,
        }
    }

    fn lower_third(auto_hide_s: u32) -> StreamElementProps {
        StreamElementProps::LowerThird {
            frame: Frame {
                x_pct: 5.0,
                y_pct: 70.0,
                w_pct: 40.0,
                h_pct: 15.0,
            },
            bar_color: "#101828".to_string(),
            bar_opacity: 0.9,
            accent_color: "#38bdf8".to_string(),
            accent_width_pct: 1.5,
            primary_style: text_style(),
            secondary_style: text_style(),
            padding_pct: 2.0,
            animation: AnimationPreset::SlideLeft,
            in_ms: 400,
            out_ms: 300,
            auto_hide_s,
        }
    }

    async fn seed_output_with_plate(state: &AppState, slug: &str) -> i64 {
        state
            .repository()
            .create_stream_output(slug, "Out")
            .await
            .unwrap();
        state
            .repository()
            .create_stream_nameplate(slug, "Ján Novák", "pastor")
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn show_person_activates_and_publishes() {
        let state = AppState::in_memory().await.unwrap();
        let id = seed_output_with_plate(&state, "np-show").await;
        let mut rx = state.live_hub().subscribe();

        let active = state
            .stream_nameplate_show_person("np-show", id)
            .await
            .unwrap()
            .expect("active");
        assert_eq!(active.source, NameplateSource::Person);
        assert_eq!(active.nameplate_id, Some(id));
        assert_eq!(active.primary, "Ján Novák");
        assert_eq!(active.secondary, "pastor");

        match rx.try_recv().expect("one event") {
            LiveEvent::StreamNameplate { output, active } => {
                assert_eq!(output, "np-show");
                assert_eq!(active.unwrap().nameplate_id, Some(id));
            }
            other => panic!("expected StreamNameplate, got {other:?}"),
        }
        // Cold-read returns the same active plate.
        assert_eq!(
            state
                .stream_nameplate_active("np-show")
                .await
                .unwrap()
                .primary,
            "Ján Novák"
        );
    }

    #[tokio::test]
    async fn hide_clears_active() {
        let state = AppState::in_memory().await.unwrap();
        let id = seed_output_with_plate(&state, "np-hide").await;
        state
            .stream_nameplate_show_person("np-hide", id)
            .await
            .unwrap();
        let hidden = state.stream_nameplate_hide("np-hide").await.unwrap();
        assert!(hidden.is_none());
        assert!(state.stream_nameplate_active("np-hide").await.is_none());
    }

    #[tokio::test]
    async fn toggle_person_shows_then_hides() {
        let state = AppState::in_memory().await.unwrap();
        let id = seed_output_with_plate(&state, "np-toggle").await;
        let shown = state
            .stream_nameplate_toggle_person("np-toggle", id)
            .await
            .unwrap();
        assert!(shown.is_some());
        let hidden = state
            .stream_nameplate_toggle_person("np-toggle", id)
            .await
            .unwrap();
        assert!(hidden.is_none());
    }

    #[tokio::test]
    async fn show_song_with_no_live_song_is_conflict() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .create_stream_output("np-song", "Out")
            .await
            .unwrap();
        // No presentation triggered → empty song → typed EmptySong (→ 409).
        let err = state
            .stream_nameplate_show_song("np-song")
            .await
            .expect_err("empty song must refuse");
        assert!(
            err.downcast_ref::<NameplateError>().is_some(),
            "expected NameplateError::EmptySong, got {err:?}"
        );
    }

    #[tokio::test]
    async fn auto_hide_fires_after_delay_and_a_newer_show_cancels_the_old_one() {
        let state = AppState::in_memory().await.unwrap();
        let id = seed_output_with_plate(&state, "np-auto").await;
        // A base scene with a lower_third element carrying a 1 s auto-hide.
        let scene = state
            .repository()
            .create_stream_scene("np-auto", "Base", SceneKind::Base)
            .await
            .unwrap();
        state
            .repository()
            .create_stream_element(scene.id, lower_third(1))
            .await
            .unwrap();

        state
            .stream_nameplate_show_person("np-auto", id)
            .await
            .unwrap();
        assert!(state.stream_nameplate_active("np-auto").await.is_some());
        // After > 1 s the auto-hide task clears it.
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(
            state.stream_nameplate_active("np-auto").await.is_none(),
            "auto-hide should have cleared the plate"
        );
    }

    #[tokio::test]
    async fn no_lower_third_element_means_no_auto_hide() {
        let state = AppState::in_memory().await.unwrap();
        let id = seed_output_with_plate(&state, "np-noauto").await;
        // No lower_third element → max_auto_hide_s == 0 → plate stays on air.
        state
            .stream_nameplate_show_person("np-noauto", id)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(state.stream_nameplate_active("np-noauto").await.is_some());
    }
}
