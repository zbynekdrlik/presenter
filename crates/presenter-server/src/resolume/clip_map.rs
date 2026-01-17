use super::types::{ClipTarget, LaneTarget, TextTransform};
use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipMapping {
    pub main_a: Vec<ClipTarget>,
    pub main_b: Vec<ClipTarget>,
    pub translation_a: Vec<ClipTarget>,
    pub translation_b: Vec<ClipTarget>,
    pub bible_a: Vec<ClipTarget>,
    pub bible_b: Vec<ClipTarget>,
    pub bible_translation_a: Vec<ClipTarget>,
    pub bible_translation_b: Vec<ClipTarget>,
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
        for layer in layers {
            if let Some(clips) = layer.get("clips").and_then(Value::as_array) {
                for clip in clips {
                    ingest_clip(&mut mapping, clip);
                }
            }
        }
        mapping.missing_tokens = compute_missing_tokens(&mapping);
        Ok(mapping)
    }

    pub fn missing_tokens(&self) -> &[&'static str] {
        &self.missing_tokens
    }
}

fn ingest_clip(mapping: &mut ClipMapping, clip: &Value) {
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

            for destination in parse_clip_destinations(tag, clip_id, text_param) {
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
                    ClipDestination::BibleTranslation(lane, target) => match lane {
                        LaneTarget::A => mapping.bible_translation_a.push(target),
                        LaneTarget::B => mapping.bible_translation_b.push(target),
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
    BibleTranslation(LaneTarget, ClipTarget),
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
    BibleTranslation,
    BibleClear,
    Timer,
    SongName,
    BandName,
}

fn parse_clip_destinations(
    name: &str,
    clip_id: i64,
    text_param_id: Option<i64>,
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

    let mut index = 1;
    let kind = match tokens[0] {
        "#main" => ClipKind::Main,
        "#translate" | "#translation" => ClipKind::Translation,
        "#bible" => {
            if tokens.get(index) == Some(&"translate") {
                index += 1;
                ClipKind::BibleTranslation
            } else if tokens.get(index) == Some(&"clear") {
                index += 1;
                ClipKind::BibleClear
            } else {
                ClipKind::Bible
            }
        }
        "#bibleclear" => ClipKind::BibleClear,
        "#timer" => ClipKind::Timer,
        "#song" => {
            if tokens.get(index) == Some(&"name") {
                index += 1;
                ClipKind::SongName
            } else {
                return result;
            }
        }
        "#band" => {
            if tokens.get(index) == Some(&"name") {
                index += 1;
                ClipKind::BandName
            } else {
                return result;
            }
        }
        _ => return result,
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

    match (kind, lane) {
        (ClipKind::Main, Some(lane)) => {
            result.push(ClipDestination::Main(
                lane,
                ClipTarget {
                    clip_id,
                    text_param_id,
                    transforms,
                },
            ));
        }
        (ClipKind::Translation, Some(lane)) => {
            result.push(ClipDestination::Translation(
                lane,
                ClipTarget {
                    clip_id,
                    text_param_id,
                    transforms,
                },
            ));
        }
        (ClipKind::Bible, Some(lane)) => {
            result.push(ClipDestination::Bible(
                lane,
                ClipTarget {
                    clip_id,
                    text_param_id,
                    transforms,
                },
            ));
        }
        (ClipKind::BibleTranslation, Some(lane)) => {
            result.push(ClipDestination::BibleTranslation(
                lane,
                ClipTarget {
                    clip_id,
                    text_param_id,
                    transforms,
                },
            ));
        }
        (ClipKind::BibleClear, _) => {
            result.push(ClipDestination::BibleClear(ClipTarget {
                clip_id,
                text_param_id: None,
                transforms,
            }));
        }
        (ClipKind::Timer, _) => {
            result.push(ClipDestination::Timer(ClipTarget {
                clip_id,
                text_param_id,
                transforms,
            }));
        }
        (ClipKind::SongName, _) => {
            result.push(ClipDestination::SongName(ClipTarget {
                clip_id,
                text_param_id,
                transforms,
            }));
        }
        (ClipKind::BandName, _) => {
            result.push(ClipDestination::BandName(ClipTarget {
                clip_id,
                text_param_id,
                transforms,
            }));
        }
        _ => {}
    }

    result
}

fn parse_transforms(tokens: &[&str]) -> Vec<TextTransform> {
    let mut transforms = Vec::new();
    for token in tokens {
        match *token {
            "u" | "upper" => {
                if !transforms.contains(&TextTransform::Uppercase) {
                    transforms.push(TextTransform::Uppercase);
                }
            }
            "re" | "noenter" | "singleline" => {
                if !transforms.contains(&TextTransform::RemoveLineBreaks) {
                    transforms.push(TextTransform::RemoveLineBreaks);
                }
            }
            _ => {}
        }
    }
    transforms
}

fn compute_missing_tokens(mapping: &ClipMapping) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if mapping.main_a.is_empty() {
        missing.push("#main-a");
    }
    if mapping.main_b.is_empty() {
        missing.push("#main-b");
    }
    if mapping.translation_a.is_empty() {
        missing.push("#translate-a");
    }
    if mapping.translation_b.is_empty() {
        missing.push("#translate-b");
    }
    if mapping.bible_a.is_empty() {
        missing.push("#bible-a");
    }
    if mapping.bible_b.is_empty() {
        missing.push("#bible-b");
    }
    if mapping.bible_translation_a.is_empty() {
        missing.push("#bible-translate-a");
    }
    if mapping.bible_translation_b.is_empty() {
        missing.push("#bible-translate-b");
    }
    if mapping.bible_clear.is_empty() {
        missing.push("#bible-clear");
    }
    if mapping.timer.is_empty() {
        missing.push("#timer");
    }
    if mapping.song_name.is_empty() {
        missing.push("#song-name");
    }
    if mapping.band_name.is_empty() {
        missing.push("#band-name");
    }
    missing
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
