//! #807: the Bible CLEAR path for one Resolume host.
//!
//! A clear must not trigger the blanked Bible lane clips and the `#bible-clear`
//! clip in one concurrent batch. When `#bible-clear` shares a layer with a lane
//! clip (SNV: `#bible-reference-a/b` and `#bible-clear` in layer 29), the two
//! `/connect`s race inside that layer and whichever Resolume processes last stays
//! live, so the clear clip showed only ~50% of the time. A clear therefore runs
//! in two strictly sequential phases:
//!
//! 1. blank every Bible lane's text (as before), then trigger the blanked lane
//!    clips EXCEPT those in a layer that also holds a `#bible-clear` clip — the
//!    clear clip replaces that layer's content anyway, so triggering the blank
//!    clip there would only race it (or show a blank cut first);
//! 2. once every phase-1 connect has completed, trigger the `#bible-clear` clips.

use super::clip_map::ClipMapping;
use super::driver::{duration_ms, HostDriver, TRIGGER_DELAY};
use super::types::{ClipTarget, LaneTarget};
use super::ResolumeConnectionSnapshot;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info};

/// The two-phase trigger plan for a Bible clear (#807).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct BibleClearTriggers {
    /// Phase 1: blanked lane clips in layers that hold no `#bible-clear` clip.
    pub(super) lanes: Vec<ClipTarget>,
    /// Blanked lane clips NOT triggered: their layer holds a `#bible-clear`
    /// clip, which replaces them.
    pub(super) skipped: Vec<ClipTarget>,
    /// Phase 2: the `#bible-clear` clips, triggered only after phase 1 is done.
    pub(super) clear: Vec<ClipTarget>,
}

/// Split the blanked lane clips of a clear into the phase-1 triggers and the
/// clips skipped because they share a layer with a `#bible-clear` clip. Pure,
/// so the layer rule is unit-tested without a mock Resolume.
pub(super) fn plan_bible_clear_triggers(
    blanked: Vec<ClipTarget>,
    clear: &[ClipTarget],
) -> BibleClearTriggers {
    let clear_layers: HashSet<usize> = clear.iter().map(|target| target.layer_index).collect();
    let (skipped, lanes): (Vec<ClipTarget>, Vec<ClipTarget>) = blanked
        .into_iter()
        .partition(|target| clear_layers.contains(&target.layer_index));
    BibleClearTriggers {
        lanes,
        skipped,
        clear: clear.to_vec(),
    }
}

/// Clip ids of `targets`, for the phase log lines.
pub(super) fn clip_ids(targets: &[ClipTarget]) -> Vec<i64> {
    targets.iter().map(|target| target.clip_id).collect()
}

impl HostDriver {
    /// Clear every Bible lane on this host and trigger the result in two
    /// phases (see the module docs). Returns whether the bible and the
    /// bible-translation lanes were blanked, which drives `handle_bible`'s A/B
    /// lane flip exactly as before #807.
    pub(super) async fn handle_bible_clear(
        &mut self,
        mapping: &ClipMapping,
        bible_lane: LaneTarget,
        bible_translation_lane: LaneTarget,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<(bool, bool)> {
        let blank = String::new();
        let bible = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_a,
                &mapping.bible_b,
                Some(&blank),
                status,
            )
            .await?;
        let reference = self
            .update_lane_text(
                bible_lane,
                &mapping.bible_reference_a,
                &mapping.bible_reference_b,
                Some(&blank),
                status,
            )
            .await?;
        let translation = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translation_a,
                &mapping.bible_translation_b,
                Some(&blank),
                status,
            )
            .await?;
        let translate_reference = self
            .update_lane_text(
                bible_translation_lane,
                &mapping.bible_translate_reference_a,
                &mapping.bible_translate_reference_b,
                Some(&blank),
                status,
            )
            .await?;

        let lanes_blanked = (!bible.is_empty(), !translation.is_empty());
        let blanked = [bible, reference, translation, translate_reference].concat();
        let triggers = plan_bible_clear_triggers(blanked, &mapping.bible_clear);
        self.trigger_bible_clear(&triggers).await?;
        Ok(lanes_blanked)
    }

    /// Phase 1 (lane clips), then phase 2 (`#bible-clear`). `trigger_clips`
    /// returns only after every connect of its batch completed, so phase 2 can
    /// never race a phase-1 connect.
    async fn trigger_bible_clear(&mut self, triggers: &BibleClearTriggers) -> anyhow::Result<()> {
        if triggers.lanes.is_empty() && triggers.clear.is_empty() {
            return Ok(());
        }
        // The same text-settle delay every trigger waits (0 in test builds).
        sleep(TRIGGER_DELAY).await;

        let phase1_start = Instant::now();
        self.trigger_clips(&triggers.lanes).await?;
        let t_phase1_ms = duration_ms(phase1_start.elapsed());
        debug!(
            host = %self.config.host,
            lane_clip_ids = ?clip_ids(&triggers.lanes),
            t_phase1_ms,
            "resolume bible clear: phase 1 (lane clips) complete, triggering #bible-clear"
        );

        let phase2_start = Instant::now();
        self.trigger_clips(&triggers.clear).await?;
        let t_phase2_ms = duration_ms(phase2_start.elapsed());
        info!(
            host = %self.config.host,
            lane_clip_ids = ?clip_ids(&triggers.lanes),
            skipped_same_layer_clip_ids = ?clip_ids(&triggers.skipped),
            clear_clip_ids = ?clip_ids(&triggers.clear),
            t_phase1_ms,
            t_phase2_ms,
            "resolume bible clear: lane clips triggered, then #bible-clear"
        );
        Ok(())
    }
}
