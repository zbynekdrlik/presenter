//! Stage-page live-event application (#793).
//!
//! The stage WebSocket hands EVERY live event to [`apply_stage_event`]
//! synchronously (no single latest-value slot a burst could coalesce), and a
//! `Stage` snapshot for another selected layout switches the display to it
//! instead of being dropped — so a lost `StageLayout` event self-heals on the
//! next snapshot. The page additionally re-reads layout + snapshot on every WS
//! (re)connect (`pages/stage.rs`), covering events published during a gap.

use leptos::prelude::*;
use presenter_core::{LiveEvent, StageDisplaySnapshot};
use std::cell::Cell;
use std::rc::Rc;

use super::stage::{ndi_activation_resets_gate, set_global_string};
use crate::state::stage::StageContext;

/// The camera-crew layout: its snapshot is published on EVERY stage broadcast
/// (for `/ui/camera`) and it is never operator-selectable, so it must never
/// switch a `/stage` display.
pub(crate) const CAMERA_CREW_LAYOUT: &str = "camera-crew";

/// True for the live events that make an in-flight reconnect resync's HTTP
/// answer stale: a `StageLayout` or a `Stage` snapshot is NEWER than whatever
/// the fetch will return.
pub(crate) fn event_invalidates_resync(event: &LiveEvent) -> bool {
    matches!(
        event,
        LiveEvent::Stage { .. } | LiveEvent::StageLayout { .. }
    )
}

/// Generation counter of applied layout/snapshot live events (#793). The
/// reconnect resync records it before each HTTP fetch and discards the answer
/// when it moved, so a slow fetch never overwrites a newer live event (with
/// snapshot-driven layout adoption that would flip the display back).
#[derive(Clone, Default)]
pub(crate) struct StageSyncGeneration(Rc<Cell<u64>>);

impl StageSyncGeneration {
    pub(crate) fn current(&self) -> u64 {
        self.0.get()
    }

    /// Bump the generation when `event` invalidates an in-flight resync.
    pub(crate) fn note(&self, event: &LiveEvent) {
        if event_invalidates_resync(event) {
            self.0.set(self.0.get().wrapping_add(1));
        }
    }
}

/// What the stage page does with an incoming `Stage` snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotAction {
    /// Snapshot for the display's current layout — show it.
    Apply,
    /// Snapshot for ANOTHER layout. The server publishes only the selected
    /// layout's snapshot (plus camera-crew; the api snapshot only while api is
    /// selected), so this layout IS the active one — switch to it, then show it.
    AdoptLayout,
    /// The always-published camera-crew snapshot — not for `/stage`.
    Ignore,
}

/// Decide how to handle a `Stage` snapshot for `snapshot_layout` while the
/// display shows `current_layout`. No I/O.
pub(crate) fn stage_snapshot_action(current_layout: &str, snapshot_layout: &str) -> SnapshotAction {
    if snapshot_layout == current_layout {
        SnapshotAction::Apply
    } else if snapshot_layout == CAMERA_CREW_LAYOUT {
        SnapshotAction::Ignore
    } else {
        SnapshotAction::AdoptLayout
    }
}

/// Switch the display to `code` (no-op when already there) and mirror it into
/// the `__presenterStageLayout` test global.
pub(crate) fn apply_layout_code(ctx: &StageContext, code: &str) {
    let current = ctx.layout_code.get_untracked();
    if current != code {
        leptos::logging::log!("stage layout {current} -> {code}");
        ctx.layout_code.set(code.to_string());
    }
    set_global_string("__presenterStageLayout", code);
}

/// Apply a `Stage` snapshot per [`stage_snapshot_action`].
pub(crate) fn apply_stage_snapshot(ctx: &StageContext, snapshot: StageDisplaySnapshot) {
    match stage_snapshot_action(&ctx.layout_code.get_untracked(), &snapshot.layout.code) {
        SnapshotAction::Apply => ctx.snapshot.set(Some(snapshot)),
        SnapshotAction::AdoptLayout => {
            apply_layout_code(ctx, &snapshot.layout.code);
            ctx.snapshot.set(Some(snapshot));
        }
        SnapshotAction::Ignore => {}
    }
}

/// Apply one live event to the stage page state. Called by the stage WS for
/// every event, in arrival order.
pub(crate) fn apply_stage_event(ctx: &StageContext, event: LiveEvent) {
    match event {
        LiveEvent::Stage { snapshot } => apply_stage_snapshot(ctx, snapshot),
        LiveEvent::StageLayout { code } => apply_layout_code(ctx, &code),
        LiveEvent::BibleSlide { output } => ctx.bible_overlay.set(Some(output)),
        LiveEvent::BibleCleared => ctx.bible_overlay.set(None),
        LiveEvent::BroadcastLive { enabled } => ctx.broadcast_live.set(enabled),
        LiveEvent::Timers { overview } => {
            ctx.snapshot.update(|snap| {
                if let Some(s) = snap {
                    s.timers = overview;
                }
            });
        }
        LiveEvent::NdiSourceActivated { source_id, .. } => {
            // #757: reset the neutral-cover / frames gate ONLY when the source
            // actually CHANGED — a same-source re-activation must leave the
            // live frames gate alone, or `mark_frames_live` (transition-guarded
            // on the still-`true` per-session Cell) never re-emits `true` and
            // the video stays stuck dormant while frames flow. Mirrors the
            // `sync_ndi_source_state` guard (`ndi_active_source_id != incoming id`).
            let resets_gate = ndi_activation_resets_gate(
                &source_id,
                ctx.ndi_active_source_id.get_untracked().as_deref(),
            );
            ctx.ndi_active.set(true);
            ctx.ndi_active_source_id.set(Some(source_id));
            if resets_gate {
                ctx.ndi_status.set("connecting".to_string());
                // #500: a freshly-activated (new/changed) source has no frames
                // yet — the neutral cover must show until the WHEP video decodes.
                ctx.ndi_frames_live.set(false);
            }
        }
        LiveEvent::NdiSourceDeactivated => {
            ctx.ndi_active.set(false);
            ctx.ndi_active_source_id.set(None);
            ctx.ndi_status.set(String::new());
            // #500: no source → no frames; clear the live-frames flag.
            ctx.ndi_frames_live.set(false);
        }
        LiveEvent::NdiConnectionStatus { status } => ctx.ndi_status.set(status),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        event_invalidates_resync, snapshot_invalidates_resync, stage_snapshot_action,
        SnapshotAction, StageSyncGeneration,
    };
    use presenter_core::LiveEvent;

    // ─────────────────────────────────────────────────────────────────────
    // #793 — a `Stage` snapshot for a layout other than the display's own
    // used to be DROPPED, so a lost `StageLayout` event left the display on
    // the old layout forever (SNV 2026-09-27: SD3 + one other on `timer`,
    // the rest on `worship-snv`). The server only publishes the SELECTED
    // layout's snapshot plus the always-on camera-crew one, so any other
    // snapshot layout IS the active layout → adopt it.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_for_the_current_layout_is_applied() {
        assert_eq!(
            stage_snapshot_action("worship-snv", "worship-snv"),
            SnapshotAction::Apply
        );
    }

    #[test]
    fn snapshot_for_another_selected_layout_adopts_it() {
        assert_eq!(
            stage_snapshot_action("worship-snv", "timer"),
            SnapshotAction::AdoptLayout
        );
        assert_eq!(
            stage_snapshot_action("timer", "api"),
            SnapshotAction::AdoptLayout,
            "the api snapshot is only published while api is selected",
        );
    }

    #[test]
    fn camera_crew_snapshot_never_flips_a_stage_display() {
        assert_eq!(
            stage_snapshot_action("worship-snv", "camera-crew"),
            SnapshotAction::Ignore,
            "camera-crew is published on every broadcast, never selected",
        );
    }

    // #793 review: a reconnect resync's HTTP answer must not overwrite a
    // NEWER live layout/snapshot event applied while the fetch was in flight —
    // but only events the display actually APPLIES count: the camera-crew
    // snapshot (published on every broadcast, the ONLY one while api is
    // selected) is ignored by /stage and must not void the resync.
    #[test]
    fn only_applied_layout_or_snapshot_events_invalidate_an_in_flight_resync() {
        assert!(event_invalidates_resync(
            &LiveEvent::StageLayout {
                code: "timer".into()
            },
            "worship-snv"
        ));
        assert!(!event_invalidates_resync(
            &LiveEvent::BroadcastLive { enabled: true },
            "worship-snv"
        ));
        assert!(!event_invalidates_resync(
            &LiveEvent::BibleCleared,
            "worship-snv"
        ));
        assert!(!event_invalidates_resync(
            &LiveEvent::NdiSourceDeactivated,
            "worship-snv"
        ));
    }

    #[test]
    fn camera_crew_snapshot_does_not_invalidate_a_resync() {
        assert!(!snapshot_invalidates_resync("worship-snv", "camera-crew"));
        assert!(!snapshot_invalidates_resync("api", "camera-crew"));
        assert!(snapshot_invalidates_resync("worship-snv", "worship-snv"));
        assert!(snapshot_invalidates_resync("worship-snv", "timer"));
    }

    #[test]
    fn generation_moves_only_on_invalidating_events() {
        let generation = StageSyncGeneration::default();
        assert_eq!(generation.current(), 0);
        generation.note(&LiveEvent::BibleCleared, "worship-snv");
        assert_eq!(
            generation.current(),
            0,
            "an unrelated event keeps the resync valid"
        );
        generation.note(
            &LiveEvent::StageLayout {
                code: "timer".into(),
            },
            "worship-snv",
        );
        assert_eq!(
            generation.current(),
            1,
            "a layout event voids an in-flight resync"
        );
        let shared = generation.clone();
        shared.note(
            &LiveEvent::StageLayout {
                code: "preach".into(),
            },
            "timer",
        );
        assert_eq!(generation.current(), 2, "clones share one counter");
    }
}
