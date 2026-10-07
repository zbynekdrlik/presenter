use super::types::{ClipTarget, LaneTarget, TextTransform};
use anyhow::{anyhow, Result};
use serde_json::Value;

/// The lyric lanes a stage push writes, named as in the missing-clip list.
pub(super) const MAIN_KINDS: [&str; 2] = ["#main-a", "#main-b"];
/// The translation lanes a stage push writes.
pub(super) const TRANSLATION_KINDS: [&str; 2] = ["#translate-a", "#translate-b"];
/// Every Bible lane a Bible push writes: verse, reference and their
/// translations.
pub(super) const BIBLE_LANE_KINDS: [&str; 8] = [
    "#bible-a",
    "#bible-b",
    "#bible-reference-a",
    "#bible-reference-b",
    "#bible-translate-a",
    "#bible-translate-b",
    "#bible-translate-reference-a",
    "#bible-translate-reference-b",
];
/// The clip a Bible clear triggers.
pub(super) const BIBLE_CLEAR_KIND: &str = "#bible-clear";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipMapping {
    pub main_a: Vec<ClipTarget>,
    pub main_b: Vec<ClipTarget>,
    pub translation_a: Vec<ClipTarget>,
    pub translation_b: Vec<ClipTarget>,
    pub bible_a: Vec<ClipTarget>,
    pub bible_b: Vec<ClipTarget>,
    pub bible_reference_a: Vec<ClipTarget>,
    pub bible_reference_b: Vec<ClipTarget>,
    pub bible_translation_a: Vec<ClipTarget>,
    pub bible_translation_b: Vec<ClipTarget>,
    pub bible_translate_reference_a: Vec<ClipTarget>,
    pub bible_translate_reference_b: Vec<ClipTarget>,
    pub bible_clear: Vec<ClipTarget>,
    pub timer: Vec<ClipTarget>,
    pub song_name: Vec<ClipTarget>,
    pub band_name: Vec<ClipTarget>,
    missing_tokens: Vec<&'static str>,
}

impl ClipMapping {
    pub fn from_composition(value: &Value) -> Result<Self> {
        let layers = value
            .get("layers")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("composition has no layers"))?;

        let mut mapping = ClipMapping::default();
        for (layer_index, layer) in layers.iter().enumerate() {
            if let Some(clips) = layer.get("clips").and_then(Value::as_array) {
                for clip in clips {
                    ingest_clip(&mut mapping, clip, layer_index);
                }
            }
        }
        mapping.missing_tokens = compute_missing_tokens(&mapping);
        Ok(mapping)
    }

    pub fn missing_tokens(&self) -> &[&'static str] {
        &self.missing_tokens
    }

    /// The destination kinds this mapping has clips for (#808: compared
    /// between fetches to spot a composition fetched while Arena was still
    /// loading it).
    pub(super) fn destination_kinds(&self) -> Vec<&'static str> {
        self.destinations()
            .into_iter()
            .filter(|(_, clips)| !clips.is_empty())
            .map(|(kind, _)| kind)
            .collect()
    }

    /// Every recognized destination kind with its clips, in the order of the
    /// missing-clip list. The one place that names the kinds.
    fn destinations(&self) -> [(&'static str, &[ClipTarget]); 16] {
        [
            (MAIN_KINDS[0], self.main_a.as_slice()),
            (MAIN_KINDS[1], self.main_b.as_slice()),
            (TRANSLATION_KINDS[0], self.translation_a.as_slice()),
            (TRANSLATION_KINDS[1], self.translation_b.as_slice()),
            (BIBLE_LANE_KINDS[0], self.bible_a.as_slice()),
            (BIBLE_LANE_KINDS[1], self.bible_b.as_slice()),
            (BIBLE_LANE_KINDS[2], self.bible_reference_a.as_slice()),
            (BIBLE_LANE_KINDS[3], self.bible_reference_b.as_slice()),
            (BIBLE_LANE_KINDS[4], self.bible_translation_a.as_slice()),
            (BIBLE_LANE_KINDS[5], self.bible_translation_b.as_slice()),
            (
                BIBLE_LANE_KINDS[6],
                self.bible_translate_reference_a.as_slice(),
            ),
            (
                BIBLE_LANE_KINDS[7],
                self.bible_translate_reference_b.as_slice(),
            ),
            (BIBLE_CLEAR_KIND, self.bible_clear.as_slice()),
            ("#timer", self.timer.as_slice()),
            ("#song-name", self.song_name.as_slice()),
            ("#band-name", self.band_name.as_slice()),
        ]
    }

    /// Returns the sorted set of `#timer` clip text-param IDs for stable
    /// equality comparison across mapping refreshes.
    pub(super) fn timer_param_ids(&self) -> Vec<i64> {
        sorted_text_param_ids(&self.timer)
    }
}

/// The sorted text-param ids of one clip kind, for comparing two mappings
/// (#267 timer dedup, #808 song/band dedup).
pub(super) fn sorted_text_param_ids(targets: &[ClipTarget]) -> Vec<i64> {
    let mut ids: Vec<i64> = targets.iter().filter_map(|t| t.text_param_id).collect();
    ids.sort_unstable();
    ids
}

/// Total number of clips across all layers in a Resolume `/composition` body —
/// the composition "size" logged on every fetch (#483).
pub(super) fn count_clips(body: &Value) -> usize {
    body.get("layers")
        .and_then(|layers| layers.as_array())
        .map(|layers| {
            layers
                .iter()
                .map(|layer| {
                    layer
                        .get("clips")
                        .and_then(|clips| clips.as_array())
                        .map(|clips| clips.len())
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

fn ingest_clip(mapping: &mut ClipMapping, clip: &Value, layer_index: usize) {
    let clip_id = clip.get("id").and_then(Value::as_i64);
    let Some(clip_id) = clip_id else {
        return;
    };

    let name = clip
        .get("name")
        .and_then(|v| v.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let name_lower = name.to_ascii_lowercase();
    let text_param = extract_text_param_id(clip);

    for fragment in name_lower.split_whitespace() {
        if let Some(start) = fragment.find('#') {
            let mut tag = &fragment[start..];
            tag = tag
                .trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '#'));
            if tag.len() < 2 {
                continue;
            }

            for destination in parse_clip_destinations(tag, clip_id, text_param, layer_index) {
                match destination {
                    ClipDestination::Main(lane, target) => match lane {
                        LaneTarget::A => mapping.main_a.push(target),
                        LaneTarget::B => mapping.main_b.push(target),
                    },
                    ClipDestination::Translation(lane, target) => match lane {
                        LaneTarget::A => mapping.translation_a.push(target),
                        LaneTarget::B => mapping.translation_b.push(target),
                    },
                    ClipDestination::Bible(lane, target) => match lane {
                        LaneTarget::A => mapping.bible_a.push(target),
                        LaneTarget::B => mapping.bible_b.push(target),
                    },
                    ClipDestination::BibleReference(lane, target) => match lane {
                        LaneTarget::A => mapping.bible_reference_a.push(target),
                        LaneTarget::B => mapping.bible_reference_b.push(target),
                    },
                    ClipDestination::BibleTranslation(lane, target) => match lane {
                        LaneTarget::A => mapping.bible_translation_a.push(target),
                        LaneTarget::B => mapping.bible_translation_b.push(target),
                    },
                    ClipDestination::BibleTranslateReference(lane, target) => match lane {
                        LaneTarget::A => mapping.bible_translate_reference_a.push(target),
                        LaneTarget::B => mapping.bible_translate_reference_b.push(target),
                    },
                    ClipDestination::BibleClear(target) => mapping.bible_clear.push(target),
                    ClipDestination::Timer(target) => mapping.timer.push(target),
                    ClipDestination::SongName(target) => mapping.song_name.push(target),
                    ClipDestination::BandName(target) => mapping.band_name.push(target),
                }
            }
        }
    }
}

enum ClipDestination {
    Main(LaneTarget, ClipTarget),
    Translation(LaneTarget, ClipTarget),
    Bible(LaneTarget, ClipTarget),
    BibleReference(LaneTarget, ClipTarget),
    BibleTranslation(LaneTarget, ClipTarget),
    BibleTranslateReference(LaneTarget, ClipTarget),
    BibleClear(ClipTarget),
    Timer(ClipTarget),
    SongName(ClipTarget),
    BandName(ClipTarget),
}

#[derive(Debug, Clone, Copy)]
enum ClipKind {
    Main,
    Translation,
    Bible,
    BibleReference,
    BibleTranslation,
    BibleTranslateReference,
    BibleClear,
    Timer,
    SongName,
    BandName,
}

fn parse_clip_destinations(
    name: &str,
    clip_id: i64,
    text_param_id: Option<i64>,
    layer_index: usize,
) -> Vec<ClipDestination> {
    let mut result = Vec::new();
    if !name.starts_with('#') {
        return result;
    }

    let tokens: Vec<&str> = name
        .split('-')
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return result;
    }

    let Some((kind, mut index)) = parse_clip_kind(&tokens) else {
        return result;
    };

    let transforms_start;
    let lane = match kind {
        ClipKind::BibleClear | ClipKind::Timer | ClipKind::SongName | ClipKind::BandName => {
            transforms_start = index;
            None
        }
        _ => {
            let token = tokens.get(index).copied();
            let lane = match token {
                Some("a") => Some(LaneTarget::A),
                Some("b") => Some(LaneTarget::B),
                _ => None,
            };
            if lane.is_none() {
                return result;
            }
            index += 1;
            transforms_start = index;
            lane
        }
    };

    let transforms = parse_transforms(&tokens[transforms_start..]);
    // Every destination shares one target; #807 records the clip's layer so the
    // Bible clear path can tell which lane clips share a layer with `#bible-clear`.
    let target = ClipTarget {
        clip_id,
        text_param_id,
        transforms,
        layer_index,
    };

    let destination = match (kind, lane) {
        (ClipKind::Main, Some(lane)) => ClipDestination::Main(lane, target),
        (ClipKind::Translation, Some(lane)) => ClipDestination::Translation(lane, target),
        (ClipKind::Bible, Some(lane)) => ClipDestination::Bible(lane, target),
        (ClipKind::BibleReference, Some(lane)) => ClipDestination::BibleReference(lane, target),
        (ClipKind::BibleTranslation, Some(lane)) => ClipDestination::BibleTranslation(lane, target),
        (ClipKind::BibleTranslateReference, Some(lane)) => {
            ClipDestination::BibleTranslateReference(lane, target)
        }
        // A clear clip is only ever triggered, never written to.
        (ClipKind::BibleClear, _) => ClipDestination::BibleClear(ClipTarget {
            text_param_id: None,
            ..target
        }),
        (ClipKind::Timer, _) => ClipDestination::Timer(target),
        (ClipKind::SongName, _) => ClipDestination::SongName(target),
        (ClipKind::BandName, _) => ClipDestination::BandName(target),
        _ => return result,
    };
    result.push(destination);
    result
}

/// The clip kind named by a clip tag's leading tokens (`#bible-translate-…`),
/// plus the index of the first token after them — the lane letter, or the
/// first transform for lane-less kinds. `None` for an unknown tag.
fn parse_clip_kind(tokens: &[&str]) -> Option<(ClipKind, usize)> {
    let mut index = 1;
    let kind = match *tokens.first()? {
        "#main" => ClipKind::Main,
        "#translate" | "#translation" => ClipKind::Translation,
        "#bible" => {
            if tokens.get(index) == Some(&"translate") {
                index += 1;
                if tokens.get(index) == Some(&"reference") {
                    index += 1;
                    ClipKind::BibleTranslateReference
                } else {
                    ClipKind::BibleTranslation
                }
            } else if tokens.get(index) == Some(&"reference") {
                index += 1;
                ClipKind::BibleReference
            } else if tokens.get(index) == Some(&"clear") {
                index += 1;
                ClipKind::BibleClear
            } else {
                ClipKind::Bible
            }
        }
        "#bibleclear" => ClipKind::BibleClear,
        "#timer" => ClipKind::Timer,
        "#song" if tokens.get(index) == Some(&"name") => {
            index += 1;
            ClipKind::SongName
        }
        "#band" if tokens.get(index) == Some(&"name") => {
            index += 1;
            ClipKind::BandName
        }
        _ => return None,
    };
    Some((kind, index))
}

fn parse_transforms(tokens: &[&str]) -> Vec<TextTransform> {
    let mut transforms = Vec::new();
    for token in tokens {
        match *token {
            "u" | "upper" if !transforms.contains(&TextTransform::Uppercase) => {
                transforms.push(TextTransform::Uppercase);
            }
            "re" | "noenter" | "singleline"
                if !transforms.contains(&TextTransform::RemoveLineBreaks) =>
            {
                transforms.push(TextTransform::RemoveLineBreaks);
            }
            _ => {}
        }
    }
    transforms
}

fn compute_missing_tokens(mapping: &ClipMapping) -> Vec<&'static str> {
    mapping
        .destinations()
        .into_iter()
        .filter(|(_, clips)| clips.is_empty())
        .map(|(kind, _)| kind)
        .collect()
}

fn extract_text_param_id(clip: &Value) -> Option<i64> {
    let sourceparams = clip.get("video")?.get("sourceparams")?.as_object()?;
    for param in sourceparams.values() {
        let valuetype = param.get("valuetype").and_then(Value::as_str)?;
        if valuetype.eq_ignore_ascii_case("paramtext") {
            if let Some(id) = param.get("id").and_then(Value::as_i64) {
                return Some(id);
            }
        }
    }
    None
}
