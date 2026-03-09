use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::types::{ClipTarget, LaneTarget, SlotKind};
use super::{BibleUpdate, ResolumeConnectionSnapshot, StageUpdate, TimerFrame};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::warn;

use super::driver::TRIGGER_DELAY;

pub(super) fn translation_short_code(code: &str) -> String {
    code.rsplit('-').next().unwrap_or(code).to_uppercase()
}

#[derive(Clone, Copy, Debug)]
pub(super) enum MetadataSlot {
    SongName,
    BandName,
}

impl HostDriver {
    pub(super) async fn handle_stage(
        &mut self,
        update: StageUpdate,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            let main_lane = self.lane_state.current(SlotKind::Main);
            let translation_lane = self.lane_state.current(SlotKind::Translation);

            let mut to_trigger = Vec::new();
            let mut main_lane_filled = false;
            if let Some(ref main_text) = update.current_main {
                let mut main_targets = self
                    .update_lane_text(
                        main_lane,
                        &mapping.main_a,
                        &mapping.main_b,
                        Some(main_text),
                        status,
                    )
                    .await?;
                if !main_targets.is_empty() {
                    to_trigger.append(&mut main_targets);
                    main_lane_filled = true;
                }
            }

            let mut translation_lane_filled = false;
            if let Some(ref translation_text) = update.current_translation {
                let mut translation_targets = self
                    .update_lane_text(
                        translation_lane,
                        &mapping.translation_a,
                        &mapping.translation_b,
                        Some(translation_text),
                        status,
                    )
                    .await?;
                if !translation_targets.is_empty() {
                    to_trigger.append(&mut translation_targets);
                    translation_lane_filled = true;
                }
            }

            if let Some(ref song_name) = update.song_name {
                if mapping.song_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #song-name clip"
                    );
                } else {
                    self.update_metadata_targets(
                        &mapping.song_name,
                        song_name,
                        MetadataSlot::SongName,
                        status,
                    )
                    .await?;
                }
            } else {
                self.last_song_name_payload = None;
            }

            if let Some(ref band_name) = update.band_name {
                if mapping.band_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #band-name clip"
                    );
                } else {
                    self.update_metadata_targets(
                        &mapping.band_name,
                        band_name,
                        MetadataSlot::BandName,
                        status,
                    )
                    .await?;
                }
            } else {
                self.last_band_name_payload = None;
            }

            if !to_trigger.is_empty() {
                if TRIGGER_DELAY.as_millis() > 0 {
                    sleep(TRIGGER_DELAY).await;
                }
                self.trigger_clips(&to_trigger).await?;
            }

            if main_lane_filled {
                self.lane_state.flip(SlotKind::Main);
                if !translation_lane_filled
                    && !mapping.translation_a.is_empty()
                    && !mapping.translation_b.is_empty()
                {
                    self.lane_state.flip(SlotKind::Translation);
                }
            }

            if translation_lane_filled {
                self.lane_state.flip(SlotKind::Translation);
            }
        }
        self.mark_connected(status).await;
        Ok(())
    }

    pub(super) async fn handle_bible(
        &mut self,
        update: BibleUpdate,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            let bible_lane = self.lane_state.current(SlotKind::Bible);
            let bible_translation_lane = self.lane_state.current(SlotKind::BibleTranslation);
            let mut to_trigger = Vec::new();

            // Prefer slide_output (single source of truth) over legacy passage
            let (bible_lane_filled, bible_translation_lane_filled) =
                if let Some(ref output) = update.slide_output {
                    // New path: use BibleSlideOutput directly (no transformations)
                    self.handle_bible_slide_output(
                        output,
                        &mapping,
                        bible_lane,
                        bible_translation_lane,
                        &mut to_trigger,
                        status,
                    )
                    .await?
                } else if let Some(ref passage) = update.passage {
                    // Legacy path: use BibleBroadcast with derived values
                    self.handle_bible_legacy(
                        passage,
                        &update,
                        &mapping,
                        bible_lane,
                        bible_translation_lane,
                        &mut to_trigger,
                        status,
                    )
                    .await?
                } else {
                    // Clear path
                    self.handle_bible_clear(
                        &mapping,
                        bible_lane,
                        bible_translation_lane,
                        &mut to_trigger,
                        status,
                    )
                    .await?
                };

            if !to_trigger.is_empty() {
                if TRIGGER_DELAY.as_millis() > 0 {
                    sleep(TRIGGER_DELAY).await;
                }
                self.trigger_clips(&to_trigger).await?;
            }
            if bible_lane_filled {
                self.lane_state.flip(SlotKind::Bible);
            }
            if bible_translation_lane_filled {
                self.lane_state.flip(SlotKind::BibleTranslation);
            }
        }
        self.mark_connected(status).await;
        Ok(())
    }

    /// Handle Bible slide using the new single-source-of-truth output.
    /// Uses the exact values from the slide without any transformation.
    async fn handle_bible_slide_output(
        &mut self,
        output: &presenter_core::BibleSlideOutput,
        mapping: &ClipMapping,
        bible_lane: LaneTarget,
        bible_translation_lane: LaneTarget,
        to_trigger: &mut Vec<ClipTarget>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<(bool, bool)> {
        // Send main verse text to #bible-a/b
        let bible_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_a,
                &mapping.bible_b,
                Some(&output.main_text),
                status,
            )
            .await?;
        let bible_lane_filled = !bible_targets.is_empty();
        if bible_lane_filled {
            to_trigger.extend(bible_targets);
        }

        // Send main reference to #bible-reference-a/b (exact value from slide)
        let bible_ref_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_reference_a,
                &mapping.bible_reference_b,
                Some(&output.main_reference),
                status,
            )
            .await?;
        if !bible_ref_targets.is_empty() {
            to_trigger.extend(bible_ref_targets);
        }

        // Send secondary translation text to #bible-translate-a/b
        let bible_translation_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translation_a,
                &mapping.bible_translation_b,
                Some(&output.secondary_text),
                status,
            )
            .await?;
        let bible_translation_lane_filled = !bible_translation_targets.is_empty();
        if bible_translation_lane_filled {
            to_trigger.extend(bible_translation_targets);
        }

        // Send secondary reference to #bible-translate-reference-a/b (exact value from slide)
        let sec_ref_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translate_reference_a,
                &mapping.bible_translate_reference_b,
                Some(&output.secondary_reference),
                status,
            )
            .await?;
        if !sec_ref_targets.is_empty() {
            to_trigger.extend(sec_ref_targets);
        }

        Ok((bible_lane_filled, bible_translation_lane_filled))
    }

    /// Handle Bible update using the legacy BibleBroadcast path (deprecated).
    async fn handle_bible_legacy(
        &mut self,
        passage: &presenter_core::BibleBroadcast,
        update: &BibleUpdate,
        mapping: &ClipMapping,
        bible_lane: LaneTarget,
        bible_translation_lane: LaneTarget,
        to_trigger: &mut Vec<ClipTarget>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<(bool, bool)> {
        let verse_text = passage.passage.text.clone();
        let translation_code = passage.passage.translation.code.clone();
        let reference = passage.passage.reference.to_human_readable();
        let short_code = translation_short_code(&translation_code);
        let reference_with_code = format!("{reference} ({short_code})");

        // Send verse text to #bible-a/b
        let bible_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_a,
                &mapping.bible_b,
                Some(&verse_text),
                status,
            )
            .await?;
        let bible_lane_filled = !bible_targets.is_empty();
        if bible_lane_filled {
            to_trigger.extend(bible_targets);
        }

        // Send reference to #bible-reference-a/b (same lane as bible)
        let bible_ref_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_reference_a,
                &mapping.bible_reference_b,
                Some(&reference_with_code),
                status,
            )
            .await?;
        if !bible_ref_targets.is_empty() {
            to_trigger.extend(bible_ref_targets);
        }

        // Send secondary translation text to #bible-translate-a/b
        let sec_text = update.secondary_text.as_deref().unwrap_or("").to_string();
        let bible_translation_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translation_a,
                &mapping.bible_translation_b,
                Some(&sec_text),
                status,
            )
            .await?;
        let bible_translation_lane_filled = !bible_translation_targets.is_empty();
        if bible_translation_lane_filled {
            to_trigger.extend(bible_translation_targets);
        }

        // Send secondary translation reference to #bible-translate-reference-a/b
        let sec_ref = if let Some(ref sec_code) = update.secondary_translation_code {
            let sec_short = translation_short_code(sec_code);
            format!("{reference} ({sec_short})")
        } else {
            String::new()
        };
        let sec_ref_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translate_reference_a,
                &mapping.bible_translate_reference_b,
                Some(&sec_ref),
                status,
            )
            .await?;
        if !sec_ref_targets.is_empty() {
            to_trigger.extend(sec_ref_targets);
        }

        Ok((bible_lane_filled, bible_translation_lane_filled))
    }

    /// Handle clearing all Bible clips.
    async fn handle_bible_clear(
        &mut self,
        mapping: &ClipMapping,
        bible_lane: LaneTarget,
        bible_translation_lane: LaneTarget,
        to_trigger: &mut Vec<ClipTarget>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<(bool, bool)> {
        let blank = String::new();
        let bible_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_a,
                &mapping.bible_b,
                Some(&blank),
                status,
            )
            .await?;
        let bible_lane_filled = !bible_targets.is_empty();
        if bible_lane_filled {
            to_trigger.extend(bible_targets);
        }

        // Clear bible reference clips
        let bible_ref_targets = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_reference_a,
                &mapping.bible_reference_b,
                Some(&blank),
                status,
            )
            .await?;
        if !bible_ref_targets.is_empty() {
            to_trigger.extend(bible_ref_targets);
        }

        let bible_translation_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translation_a,
                &mapping.bible_translation_b,
                Some(&blank),
                status,
            )
            .await?;
        let bible_translation_lane_filled = !bible_translation_targets.is_empty();
        if bible_translation_lane_filled {
            to_trigger.extend(bible_translation_targets);
        }

        // Clear secondary translation reference clips
        let sec_ref_targets = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translate_reference_a,
                &mapping.bible_translate_reference_b,
                Some(&blank),
                status,
            )
            .await?;
        if !sec_ref_targets.is_empty() {
            to_trigger.extend(sec_ref_targets);
        }

        to_trigger.extend(mapping.bible_clear.iter().cloned());
        Ok((bible_lane_filled, bible_translation_lane_filled))
    }

    pub(super) async fn handle_timer(
        &mut self,
        frame: TimerFrame,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            if mapping.timer.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    "Resolume mapping missing #timer clip"
                );
                return Ok(());
            }

            let text = frame.formatted;
            if self.last_timer_payload.as_deref() == Some(text.as_str()) {
                return Ok(());
            }

            let endpoint = self.endpoint().await?;
            let mut latency_recorded = None;
            for target in &mapping.timer {
                if let Some(duration) = self.update_clip_text(target, &text, &endpoint).await? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
            self.last_timer_payload = Some(text);
        }
        Ok(())
    }

    pub(super) async fn update_lane_text(
        &mut self,
        lane: LaneTarget,
        lane_a: &[ClipTarget],
        lane_b: &[ClipTarget],
        text: Option<&String>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<Vec<ClipTarget>> {
        let (primary, alternate) = super::types::select_lane_targets(lane, lane_a, lane_b);
        if primary.is_empty() {
            if !alternate.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume lane missing clips; skipping update"
                );
            } else {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume has no clips configured for lane"
                );
            }
            return Ok(Vec::new());
        }
        let selected = primary;

        if let Some(payload) = text {
            let endpoint = self.endpoint().await?;
            let mut latency_recorded = None;
            for target in selected {
                if let Some(duration) = self.update_clip_text(target, payload, &endpoint).await? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
        }

        Ok(selected.to_vec())
    }

    pub(super) async fn update_metadata_targets(
        &mut self,
        targets: &[ClipTarget],
        text: &str,
        slot: MetadataSlot,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if targets.is_empty() {
            return Ok(());
        }

        let already_sent = match slot {
            MetadataSlot::SongName => self.last_song_name_payload.as_deref() == Some(text),
            MetadataSlot::BandName => self.last_band_name_payload.as_deref() == Some(text),
        };

        if already_sent {
            return Ok(());
        }

        let endpoint = self.endpoint().await?;
        let mut latency_recorded = None;
        for target in targets {
            if let Some(duration) = self.update_clip_text(target, text, &endpoint).await? {
                if latency_recorded.is_none() {
                    latency_recorded = Some(duration);
                }
            }
        }
        if let Some(latency) = latency_recorded {
            self.note_latency(status, latency).await;
        }
        match slot {
            MetadataSlot::SongName => self.last_song_name_payload = Some(text.to_string()),
            MetadataSlot::BandName => self.last_band_name_payload = Some(text.to_string()),
        }
        Ok(())
    }
}
