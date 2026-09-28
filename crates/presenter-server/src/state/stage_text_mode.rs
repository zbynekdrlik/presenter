//! Persisted stage text mode for the API layouts (#799).
//!
//! The operator picks which lyric text(s) the `api` / `api-ambient` layouts
//! show — original, translation, or both. The mode lives in a lock-free
//! atomic cell so it can be read while the stage-layout lock is held (the
//! api snapshot is stamped under that lock, see `api_stage.rs`), and it is
//! persisted in `app_settings` like the stage layout itself (#384) so it
//! survives a restart/deploy.

use super::AppState;
use crate::live::LiveEvent;
use presenter_core::StageTextMode;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// `app_settings` key of the persisted text mode. Same generic k/v mechanism
/// (and the same no-audit rationale) as `STAGE_LAYOUT_KEY`.
pub(crate) const STAGE_TEXT_MODE_KEY: &str = "feature.stage.text_mode";

/// Shared, lock-free holder of the current [`StageTextMode`]. `Arc` so every
/// `AppState` clone sees the same value.
#[derive(Clone)]
pub(crate) struct StageTextModeCell(Arc<AtomicU8>);

impl StageTextModeCell {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU8::new(StageTextMode::default().to_u8())))
    }

    pub(crate) fn get(&self) -> StageTextMode {
        StageTextMode::from_u8(self.0.load(Ordering::SeqCst))
    }

    /// Store `mode`, returning the previous value.
    pub(crate) fn replace(&self, mode: StageTextMode) -> StageTextMode {
        StageTextMode::from_u8(self.0.swap(mode.to_u8(), Ordering::SeqCst))
    }
}

impl AppState {
    /// The current text mode of the API stage layouts.
    pub fn stage_text_mode(&self) -> StageTextMode {
        self.stage_text_mode.get()
    }

    /// Read the persisted text mode, falling back to the default (`both`)
    /// when none is stored, the stored value is unknown, or the read fails.
    /// Pure read — never writes (second-startup-no-audit invariant).
    pub(crate) async fn load_persisted_stage_text_mode(&self) -> StageTextMode {
        match self.repository().get_app_setting(STAGE_TEXT_MODE_KEY).await {
            Ok(Some(stored)) => stored.parse::<StageTextMode>().unwrap_or_else(|err| {
                tracing::warn!(%err, "persisted stage text mode unknown — using default");
                StageTextMode::default()
            }),
            Ok(None) => StageTextMode::default(),
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "failed to load persisted stage text mode — using default"
                );
                StageTextMode::default()
            }
        }
    }

    /// Seed the in-memory mode from the database on startup.
    pub(crate) async fn restore_stage_text_mode(&self) {
        let mode = self.load_persisted_stage_text_mode().await;
        self.stage_text_mode.replace(mode);
    }

    /// Change the text mode: store it, persist it (best-effort — the live
    /// value already changed), announce it with `LiveEvent::StageTextMode`
    /// for operator surfaces, and re-publish the api snapshot (which carries
    /// the mode) when an API layout is selected so displays switch live.
    /// Re-selecting the current mode is a no-op.
    pub async fn set_stage_text_mode(&self, mode: StageTextMode) {
        let previous = self.stage_text_mode.replace(mode);
        if previous == mode {
            return;
        }
        tracing::info!(
            target: "presenter::stage::text_mode",
            from = %previous,
            to = %mode,
            "stage text mode switched"
        );
        if let Err(err) = self
            .repository()
            .set_app_setting(STAGE_TEXT_MODE_KEY, mode.as_str())
            .await
        {
            tracing::warn!(
                ?err,
                %mode,
                "failed to persist stage text mode — it will reset on next restart"
            );
        }
        self.live_hub.publish(LiveEvent::StageTextMode { mode });
        self.republish_api_snapshot().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn default_mode_is_both_and_nothing_is_persisted() {
        let state = AppState::in_memory().await.unwrap();
        assert_eq!(state.stage_text_mode(), StageTextMode::Both);
        assert_eq!(
            state.load_persisted_stage_text_mode().await,
            StageTextMode::Both
        );
        let stored = state
            .repository()
            .get_app_setting(STAGE_TEXT_MODE_KEY)
            .await
            .unwrap();
        assert!(stored.is_none(), "a pure read must not write the default");
    }

    #[tokio::test]
    async fn set_mode_persists_and_restores_on_a_fresh_cell() {
        let state = AppState::in_memory().await.unwrap();
        state.set_stage_text_mode(StageTextMode::Translation).await;
        assert_eq!(state.stage_text_mode(), StageTextMode::Translation);
        assert_eq!(
            state
                .repository()
                .get_app_setting(STAGE_TEXT_MODE_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("translation")
        );

        // Simulate a restart: reset the live cell, then restore from the DB.
        state.stage_text_mode.replace(StageTextMode::Both);
        state.restore_stage_text_mode().await;
        assert_eq!(state.stage_text_mode(), StageTextMode::Translation);
    }

    #[tokio::test]
    async fn unknown_persisted_value_falls_back_to_default() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .set_app_setting(STAGE_TEXT_MODE_KEY, "klingon")
            .await
            .unwrap();
        assert_eq!(
            state.load_persisted_stage_text_mode().await,
            StageTextMode::Both
        );
    }

    #[tokio::test]
    async fn set_mode_publishes_event_and_republishes_api_ambient_snapshot() {
        let state = AppState::in_memory().await.unwrap();
        state
            .set_stage_layout_code("api-ambient")
            .await
            .expect("api-ambient is operator-selectable");
        let mut rx = state.live_hub().subscribe();

        state.set_stage_text_mode(StageTextMode::Original).await;

        let mut saw_mode_event = false;
        let mut snapshot = None;
        let collect = async {
            while snapshot.is_none() || !saw_mode_event {
                match rx.recv().await {
                    Ok(LiveEvent::StageTextMode { mode }) => {
                        assert_eq!(mode, StageTextMode::Original);
                        saw_mode_event = true;
                    }
                    Ok(LiveEvent::Stage { snapshot: s }) => snapshot = Some(s),
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        };
        timeout(Duration::from_millis(500), collect)
            .await
            .expect("mode event + api snapshot within timeout");
        let snapshot = snapshot.expect("api snapshot republished");
        assert!(saw_mode_event);
        assert_eq!(snapshot.layout.code, "api-ambient");
        assert_eq!(snapshot.text_mode, Some(StageTextMode::Original));
    }

    #[tokio::test]
    async fn reselecting_the_same_mode_publishes_nothing() {
        let state = AppState::in_memory().await.unwrap();
        let mut rx = state.live_hub().subscribe();
        state.set_stage_text_mode(StageTextMode::Both).await;
        // Drain unrelated events (heartbeats) for a short window; neither a
        // mode event nor a snapshot may arrive for an unchanged mode.
        let saw_mode_traffic = async {
            loop {
                match rx.recv().await {
                    Ok(LiveEvent::StageTextMode { .. } | LiveEvent::Stage { .. }) => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        };
        let got = timeout(Duration::from_millis(150), saw_mode_traffic).await;
        assert!(got.is_err(), "no event for an unchanged mode");
    }
}
