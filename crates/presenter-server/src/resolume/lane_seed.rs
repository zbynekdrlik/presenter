//! Seed the A/B lanes from the side Resolume shows connected (#841 review).
//!
//! The lane state starts at side A on every start and host config change. When
//! side A is still the connected clip (a verse, then a clear, then a restart),
//! the first push wrote into that live clip and connected it again, so the text
//! popped in without the layer crossfade. Since #841 the Bible clear also
//! leaves the reference clip connected at opacity 0, so its restore popped the
//! reference in too. After the first composition fetch that lists lane clips,
//! each slot whose live side is known starts on the OTHER side.

use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::types::{ClipTarget, LaneTarget, SlotKind};
use tracing::info;

const SLOTS: [SlotKind; 4] = [
    SlotKind::Main,
    SlotKind::Translation,
    SlotKind::Bible,
    SlotKind::BibleTranslation,
];

/// A slot side without reference clips.
const NO_CLIPS: &[ClipTarget] = &[];

/// One side of a slot: its text clips and its reference clips.
type Side<'a> = [&'a [ClipTarget]; 2];

fn any_connected(side: &Side<'_>) -> bool {
    side.iter()
        .flat_map(|clips| clips.iter())
        .any(|clip| clip.connected)
}

impl ClipMapping {
    /// The A and B sides of one slot (a Bible slot also drives its reference
    /// clips, which flip with it).
    fn slot_sides(&self, slot: SlotKind) -> [Side<'_>; 2] {
        match slot {
            SlotKind::Main => [
                [self.main_a.as_slice(), NO_CLIPS],
                [self.main_b.as_slice(), NO_CLIPS],
            ],
            SlotKind::Translation => [
                [self.translation_a.as_slice(), NO_CLIPS],
                [self.translation_b.as_slice(), NO_CLIPS],
            ],
            SlotKind::Bible => [
                [self.bible_a.as_slice(), self.bible_reference_a.as_slice()],
                [self.bible_b.as_slice(), self.bible_reference_b.as_slice()],
            ],
            SlotKind::BibleTranslation => [
                [
                    self.bible_translation_a.as_slice(),
                    self.bible_translate_reference_a.as_slice(),
                ],
                [
                    self.bible_translation_b.as_slice(),
                    self.bible_translate_reference_b.as_slice(),
                ],
            ],
        }
    }

    /// The lane the next push of `slot` should fill: the side that is NOT
    /// connected when exactly one side is, else `None` (unknown: neither side
    /// or both sides connected).
    pub(super) fn next_lane_from_connected(&self, slot: SlotKind) -> Option<LaneTarget> {
        let [a, b] = self.slot_sides(slot);
        match (any_connected(&a), any_connected(&b)) {
            (true, false) => Some(LaneTarget::B),
            (false, true) => Some(LaneTarget::A),
            _ => None,
        }
    }

    /// Whether the mapping lists any A/B lane clip. A composition fetched while
    /// Arena is still loading lists none, and must not count as the seed.
    fn has_lane_clips(&self) -> bool {
        SLOTS.iter().any(|&slot| {
            self.slot_sides(slot)
                .iter()
                .flatten()
                .any(|clips| !clips.is_empty())
        })
    }
}

impl HostDriver {
    /// Seed each slot's next lane from the clips' connected state, once after
    /// a start or host config change (the lane state resets with them).
    pub(super) fn seed_lanes(&mut self, mapping: &ClipMapping) {
        if self.lane_state.seeded || !mapping.has_lane_clips() {
            return;
        }
        self.lane_state.seeded = true;
        let mut seeded = Vec::new();
        for slot in SLOTS {
            if let Some(lane) = mapping.next_lane_from_connected(slot) {
                self.lane_state.set(slot, lane);
                seeded.push(format!("{slot:?}={}", lane.label()));
            }
        }
        info!(
            host = %self.config.host,
            ?seeded,
            "resolume lanes seeded from the connected clips (unlisted slots start on A)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use presenter_core::{ResolumeHost, ResolumeHostId};
    use reqwest::Client;
    use serde_json::{json, Value};

    fn clip(id: i64, name: &str, connected: &str) -> Value {
        json!({
            "id": id,
            "name": { "value": name },
            "connected": { "value": connected },
            "video": { "sourceparams": { "text": { "valuetype": "ParamText", "id": id + 1000 } } },
        })
    }

    fn mapping(clips: Vec<Value>) -> ClipMapping {
        ClipMapping::from_composition(&json!({ "layers": [{ "clips": clips }] })).expect("mapping")
    }

    fn driver() -> HostDriver {
        let now = Utc::now();
        let config = ResolumeHost::new(
            ResolumeHostId::new(),
            "Mock".into(),
            "127.0.0.1".into(),
            8090,
            true,
            now,
            now,
        );
        HostDriver::new(Client::new(), config)
    }

    #[test]
    fn the_connected_state_is_parsed_from_the_composition() {
        let mapping = mapping(vec![
            clip(1, "#bible-a", "Connected"),
            clip(2, "#bible-b", "Disconnected"),
            clip(3, "#main-a", "Connected & previewing"),
            clip(4, "#main-b", "Previewing"),
            clip(5, "#translate-a", "Empty"),
        ]);
        assert!(mapping.bible_a[0].connected);
        assert!(!mapping.bible_b[0].connected);
        assert!(mapping.main_a[0].connected);
        assert!(!mapping.main_b[0].connected);
        assert!(!mapping.translation_a[0].connected);
    }

    #[test]
    fn the_next_lane_is_the_side_that_is_not_connected() {
        // SNV after a clear: the verse text clip A is connected, and the
        // reference layer shows the old blank clip.
        let snv = mapping(vec![
            clip(1, "#bible-a", "Connected"),
            clip(2, "#bible-b", "Disconnected"),
            clip(3, "#bible-reference-a", "Disconnected"),
            clip(4, "#bible-reference-b", "Disconnected"),
            clip(5, "#main-b", "Connected"),
            clip(6, "#main-a", "Disconnected"),
        ]);
        assert_eq!(
            snv.next_lane_from_connected(SlotKind::Bible),
            Some(LaneTarget::B)
        );
        assert_eq!(
            snv.next_lane_from_connected(SlotKind::Main),
            Some(LaneTarget::A)
        );
        assert_eq!(snv.next_lane_from_connected(SlotKind::Translation), None);

        let reference_only = mapping(vec![
            clip(1, "#bible-a", "Disconnected"),
            clip(2, "#bible-reference-b", "Connected"),
        ]);
        assert_eq!(
            reference_only.next_lane_from_connected(SlotKind::Bible),
            Some(LaneTarget::A),
            "a slot's reference clips count for its side"
        );

        let both = mapping(vec![
            clip(1, "#bible-translate-a", "Connected"),
            clip(2, "#bible-translate-b", "Connected"),
        ]);
        assert_eq!(
            both.next_lane_from_connected(SlotKind::BibleTranslation),
            None
        );
    }

    #[test]
    fn lanes_are_seeded_once_per_start_and_again_after_a_config_change() {
        let mut driver = driver();
        // Arena still loading: no lane clips, no seed yet.
        driver.seed_lanes(&mapping(vec![clip(9, "#timer", "Connected")]));
        assert!(!driver.lane_state.seeded);

        driver.seed_lanes(&mapping(vec![
            clip(1, "#bible-a", "Connected"),
            clip(2, "#bible-b", "Disconnected"),
        ]));
        assert_eq!(driver.lane_state.current(SlotKind::Bible), LaneTarget::B);
        assert_eq!(driver.lane_state.current(SlotKind::Main), LaneTarget::A);

        // A later fetch never re-seeds: the lanes now follow the pushes.
        driver.seed_lanes(&mapping(vec![
            clip(1, "#bible-a", "Disconnected"),
            clip(2, "#bible-b", "Connected"),
        ]));
        assert_eq!(driver.lane_state.current(SlotKind::Bible), LaneTarget::B);

        let config = driver.config.clone();
        driver.update_config(config);
        assert!(!driver.lane_state.seeded, "a config change resets the seed");
        assert_eq!(driver.lane_state.current(SlotKind::Bible), LaneTarget::A);
        driver.seed_lanes(&mapping(vec![
            clip(1, "#bible-a", "Connected"),
            clip(2, "#bible-b", "Disconnected"),
        ]));
        assert!(driver.lane_state.seeded);
        assert_eq!(driver.lane_state.current(SlotKind::Bible), LaneTarget::B);
    }
}
