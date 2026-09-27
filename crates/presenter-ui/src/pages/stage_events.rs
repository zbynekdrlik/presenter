//! Stage-page live-event application (#793).

#[cfg(test)]
mod tests {
    use super::{stage_snapshot_action, SnapshotAction};

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
}
