//! ProPresenter import module.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::needless_continue,
    clippy::too_many_lines,
    clippy::large_enum_variant,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::redundant_closure,
    clippy::needless_lifetimes,
    clippy::match_same_arms,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::map_unwrap_or
)]

pub mod bible;
mod rtf;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use presenter_core::{Library, Presentation, Slide, SlideContent, SlideGroup, SlideText};
use presenter_persistence::Repository;
use prost::Message;
use proto::presentation::CueGroup;
use tracing::instrument;

pub use rtf::decode_rtf;

#[allow(clippy::large_enum_variant)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/rv.data.rs"));
}

/// Default location for the shared ProPresenter bundle.
pub fn default_library_root() -> PathBuf {
    if let Ok(root) = env::var("PRESENTER_LIBRARY_ROOT") {
        return PathBuf::from(root);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push_candidate = |path: PathBuf| {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    };

    if let Ok(current_dir) = env::current_dir() {
        push_candidate(current_dir.join("../presenter-libraries"));
        push_candidate(current_dir.join("presenter-libraries"));
        push_candidate(current_dir.join("../propresenter-libraries"));
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_candidate(dir.join("../presenter-libraries"));
            push_candidate(dir.join("../../presenter-libraries"));
            push_candidate(dir.join("../propresenter-libraries"));
        }
        if let Some(repo_root) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            push_candidate(repo_root.join("../presenter-libraries"));
            push_candidate(repo_root.join("presenter-libraries"));
        }
    }

    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        push_candidate(home.join("presenter-libraries"));
        push_candidate(home.join("propresenter-libraries"));
    }

    if let Ok(profile) = env::var("USERPROFILE") {
        let profile = PathBuf::from(profile);
        push_candidate(profile.join("presenter-libraries"));
        push_candidate(profile.join("propresenter-libraries"));
    }

    push_candidate(PathBuf::from("../presenter-libraries"));
    push_candidate(PathBuf::from("presenter-libraries"));
    push_candidate(PathBuf::from("../propresenter-libraries"));
    push_candidate(PathBuf::from("propresenter-libraries"));

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from("../presenter-libraries")
}

/// Summary describing the outcome of importing a single ProPresenter library directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub library_name: String,
    pub presentation_count: usize,
    pub slide_count: usize,
    pub source: PathBuf,
}

/// Converts ProPresenter presentation files (\*.pro) into the domain model and persists them.
#[derive(Debug)]
pub struct ProPresenterImporter<'a> {
    repository: &'a Repository,
}

impl<'a> ProPresenterImporter<'a> {
    pub fn new(repository: &'a Repository) -> Self {
        Self { repository }
    }

    /// Import every presentation found in `library_dir` (non-recursive) as a single library.
    /// Returns `Ok(None)` if no presentation files are present.
    #[instrument(skip(self), fields(library = %library_dir.display()))]
    pub async fn import_library_dir(&self, library_dir: &Path) -> Result<Option<ImportSummary>> {
        let library = load_library_from_directory(library_dir)?;
        let Some(library) = library else {
            return Ok(None);
        };
        let presentation_count = library.presentations.len();
        let slide_count: usize = library.presentations.iter().map(|p| p.slides.len()).sum();
        let summary = ImportSummary {
            library_name: library.name.clone(),
            presentation_count,
            slide_count,
            source: library_dir.to_path_buf(),
        };
        self.repository
            .upsert_library(&library)
            .await
            .with_context(|| {
                format!(
                    "failed to upsert library {} into persistence",
                    &library.name
                )
            })?;

        if library.name.eq_ignore_ascii_case("NEW LEVEL") {
            self.repository
                .set_library_favorite(library.id, true)
                .await
                .with_context(|| format!("failed to mark library {} as favorite", &library.name))?;
        }
        Ok(Some(summary))
    }

    /// Import every immediate subdirectory of `root_dir` as a library.
    #[instrument(skip(self), fields(root = %root_dir.display()))]
    pub async fn import_root(&self, root_dir: &Path) -> Result<Vec<ImportSummary>> {
        let mut summaries = Vec::new();
        let mut entries = fs::read_dir(root_dir)
            .with_context(|| format!("failed to scan root dir {}", root_dir.display()))?
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            if entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", path.display()))?
                .is_dir()
            {
                if let Some(summary) = self.import_library_dir(&path).await? {
                    summaries.push(summary);
                }
            }
        }
        Ok(summaries)
    }
}

/// Parses a single ProPresenter presentation file into the domain model.
#[instrument(fields(file = %path.display()))]
pub fn load_presentation_from_path(path: &Path) -> Result<Presentation> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    load_presentation_from_bytes(&bytes)
        .with_context(|| format!("failed to load presentation from {}", path.display()))
}

/// Parses a ProPresenter `.pro` file from raw bytes into the domain model.
pub fn load_presentation_from_bytes(bytes: &[u8]) -> Result<Presentation> {
    let raw = proto::Presentation::decode(bytes)
        .context("failed to decode ProPresenter protobuf data")?;
    presentation_from_proto(&raw)
        .with_context(|| format!("failed to convert presentation {}", raw.name.trim()))
}

fn load_library_from_directory(dir: &Path) -> Result<Option<Library>> {
    if !dir.exists() {
        return Ok(None);
    }
    let name = dir
        .file_name()
        .and_then(|os| os.to_str())
        .ok_or_else(|| {
            anyhow!(
                "library directory must have a valid UTF-8 name: {}",
                dir.display()
            )
        })?
        .to_string();

    let mut presentations = Vec::new();
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("failed to list directory {}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_file() && is_pro_presentation(&path) {
            let presentation = load_presentation_from_path(&path)?;
            presentations.push(presentation);
        }
    }

    if presentations.is_empty() {
        return Ok(None);
    }

    let library = Library::new(name, presentations)?;
    Ok(Some(library))
}

#[derive(Debug, Clone)]
struct GroupAssignment {
    name: String,
    anchor: bool,
}

fn presentation_from_proto(raw: &proto::Presentation) -> Result<Presentation> {
    let groups = build_group_lookup(raw);
    let cue_sequence = resolve_cue_sequence(raw);
    let mut slides = Vec::new();

    for (order, cue) in cue_sequence.into_iter().enumerate() {
        let mut first_action = true;
        for slide_proto in extract_presentation_slides(cue) {
            let base_slide = slide_proto
                .base_slide
                .as_ref()
                .ok_or_else(|| anyhow!("presentation slide missing base slide data"))?;
            let group = if first_action {
                cue.uuid
                    .as_ref()
                    .and_then(|uuid| groups.get(&uuid.string))
                    .and_then(|assignment| {
                        if assignment.anchor {
                            Some(SlideGroup::new(assignment.name.clone()))
                        } else {
                            None
                        }
                    })
            } else {
                None
            };
            let content = slide_content_from_proto(base_slide, group)?;
            slides.push(Slide::new(order as u32, content));
            first_action = false;
        }
    }

    if slides.is_empty() {
        return Err(anyhow!(
            "presentation '{}' contains no slide actions",
            raw.name
        ));
    }

    Ok(Presentation::new(raw.name.clone(), slides)?)
}

fn slide_content_from_proto(
    base_slide: &proto::Slide,
    group: Option<SlideGroup>,
) -> Result<SlideContent> {
    let mut buckets: Vec<(TextRole, String)> = Vec::new();

    for element in &base_slide.elements {
        if let Some(graphic) = &element.element {
            if let Some(text) = &graphic.text {
                if text.rtf_data.is_empty() {
                    continue;
                }
                let decoded = decode_rtf(&text.rtf_data)?;
                let trimmed = decoded.trim();
                if trimmed.is_empty() || is_placeholder_text(trimmed) {
                    continue;
                }
                let role = classify_text_role(&graphic.name);
                buckets.push((role, trimmed.to_string()));
            }
        }
    }

    if buckets.is_empty() {
        buckets.push((TextRole::Main, String::new()));
    }

    let main = select_text(&buckets, TextRole::Main)
        .or_else(|| {
            buckets
                .iter()
                .find(|(role, _)| *role == TextRole::Unknown)
                .map(|(_, text)| text.clone())
        })
        .unwrap_or_default();
    let translation = select_text(&buckets, TextRole::Translation).unwrap_or_default();
    let stage = select_text(&buckets, TextRole::Stage).unwrap_or_default();

    Ok(SlideContent::new(
        SlideText::new(main)?,
        SlideText::new(translation)?,
        SlideText::new(stage)?,
        group,
    ))
}

fn select_text(buckets: &[(TextRole, String)], role: TextRole) -> Option<String> {
    buckets.iter().find_map(|(candidate_role, value)| {
        if *candidate_role == role {
            Some(value.clone())
        } else {
            None
        }
    })
}

fn extract_presentation_slides(
    cue: &proto::Cue,
) -> impl Iterator<Item = &proto::PresentationSlide> {
    cue.actions.iter().filter_map(|action| {
        if matches!(
            proto::action::ActionType::from_i32(action.r#type),
            Some(proto::action::ActionType::PresentationSlide)
        ) {
            if let Some(proto::action::ActionTypeData::Slide(slide_type)) = &action.action_type_data
            {
                if let Some(proto::action::slide_type::Slide::Presentation(presentation)) =
                    &slide_type.slide
                {
                    return Some(presentation);
                }
            }
        }
        None
    })
}

fn build_group_lookup(presentation: &proto::Presentation) -> HashMap<String, GroupAssignment> {
    let mut mapping = HashMap::new();
    for cue_group in &presentation.cue_groups {
        if let Some(group) = cue_group.group.as_ref() {
            let name = group.name.clone();
            for (index, cue_uuid) in cue_group.cue_identifiers.iter().enumerate() {
                if !cue_uuid.string.is_empty() {
                    mapping.insert(
                        cue_uuid.string.clone(),
                        GroupAssignment {
                            name: name.clone(),
                            anchor: index == 0,
                        },
                    );
                }
            }
        }
    }
    mapping
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextRole {
    Main,
    Translation,
    Stage,
    Unknown,
}

fn classify_text_role(name: &str) -> TextRole {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return TextRole::Unknown;
    }

    if STAGE_ALIASES.iter().any(|alias| normalized.contains(alias)) {
        return TextRole::Stage;
    }
    if TRANSLATION_ALIASES
        .iter()
        .any(|alias| normalized.contains(alias))
    {
        return TextRole::Translation;
    }
    if MAIN_ALIASES.iter().any(|alias| normalized.contains(alias)) {
        return TextRole::Main;
    }
    TextRole::Unknown
}

const MAIN_ALIASES: &[&str] = &["main", "lyrics", "text", "slovak", "hlavny", "hlavný"];
const TRANSLATION_ALIASES: &[&str] = &["translation", "preklad", "english", "eng"];
const STAGE_ALIASES: &[&str] = &["stage", "stage display", "stage text"];

fn is_pro_presentation(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pro"))
        .unwrap_or(false)
}

fn is_placeholder_text(text: &str) -> bool {
    matches!(text, "." | "0")
}

fn resolve_cue_sequence(raw: &proto::Presentation) -> Vec<&proto::Cue> {
    use std::collections::{HashMap, HashSet};

    let mut cues_by_uuid: HashMap<&str, &proto::Cue> = HashMap::new();
    let mut cues_without_uuid: Vec<&proto::Cue> = Vec::new();

    for cue in &raw.cues {
        if let Some(uuid) = cue.uuid.as_ref() {
            cues_by_uuid.insert(uuid.string.as_str(), cue);
        } else {
            cues_without_uuid.push(cue);
        }
    }

    let mut ordered: Vec<&proto::Cue> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    let group_map: HashMap<&str, &CueGroup> = raw
        .cue_groups
        .iter()
        .filter_map(|group| {
            let uuid = group.group.as_ref()?.uuid.as_ref()?;
            Some((uuid.string.as_str(), group))
        })
        .collect();

    if let Some(arrangement) = raw
        .arrangements
        .iter()
        .find(|arr| !arr.group_identifiers.is_empty())
    {
        for group_id in &arrangement.group_identifiers {
            let group_key = group_id.string.as_str();
            if let Some(group) = group_map.get(group_key) {
                for cue_id in &group.cue_identifiers {
                    let cue_key = cue_id.string.as_str();
                    if let Some(cue) = cues_by_uuid.get(cue_key) {
                        ordered.push(*cue);
                    }
                    seen.insert(cue_key);
                }
            } else if let Some(cue) = cues_by_uuid.get(group_key) {
                ordered.push(*cue);
                seen.insert(group_key);
            }
        }
    }

    if ordered.is_empty() {
        for group in &raw.cue_groups {
            for cue_id in &group.cue_identifiers {
                let cue_key = cue_id.string.as_str();
                if seen.insert(cue_key) {
                    if let Some(cue) = cues_by_uuid.get(cue_key) {
                        ordered.push(*cue);
                    }
                }
            }
        }
    }

    for cue in &raw.cues {
        if let Some(uuid) = cue.uuid.as_ref() {
            let cue_key = uuid.string.as_str();
            if seen.insert(cue_key) {
                ordered.push(cue);
            }
        }
    }

    if !cues_without_uuid.is_empty() {
        ordered.extend(cues_without_uuid);
    }

    if ordered.is_empty() {
        return raw.cues.iter().collect();
    }

    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Group;
    use std::path::PathBuf;

    fn library_path(relative: &str) -> Option<PathBuf> {
        let path = default_library_root().join(relative);
        if path.exists() {
            Some(path)
        } else {
            eprintln!(
                "Skipping test: missing library data at {} (set PRESENTER_LIBRARY_ROOT)",
                path.display()
            );
            None
        }
    }

    #[test]
    fn classify_text_role_matches_aliases() {
        assert_eq!(classify_text_role("Main"), TextRole::Main);
        assert_eq!(classify_text_role("Slovak"), TextRole::Main);
        assert_eq!(classify_text_role("Translation"), TextRole::Translation);
        assert_eq!(classify_text_role("Preklad"), TextRole::Translation);
        assert_eq!(classify_text_role("Stage Display"), TextRole::Stage);
        assert_eq!(classify_text_role("Random"), TextRole::Unknown);
    }

    #[test]
    fn slide_content_treats_single_dot_as_blank() {
        let text = proto::graphics::Text {
            rtf_data: b".".to_vec(),
            ..Default::default()
        };

        let element = proto::graphics::Element {
            name: "Main".into(),
            text: Some(text),
            ..Default::default()
        };

        let slide = proto::Slide {
            elements: vec![proto::slide::Element {
                element: Some(element),
                ..Default::default()
            }],
            ..Default::default()
        };

        let content = super::slide_content_from_proto(&slide, None).expect("content");
        assert!(content.main.value().is_empty(), "main text should be blank");
    }

    #[test]
    fn slide_content_preserves_real_text_when_stage_placeholder_removed() {
        let main = proto::graphics::Text {
            rtf_data: b"Verse".to_vec(),
            ..Default::default()
        };

        let stage = proto::graphics::Text {
            rtf_data: b".".to_vec(),
            ..Default::default()
        };

        let main_element = proto::graphics::Element {
            name: "Main".into(),
            text: Some(main),
            ..Default::default()
        };

        let stage_element = proto::graphics::Element {
            name: "Stage".into(),
            text: Some(stage),
            ..Default::default()
        };

        let slide = proto::Slide {
            elements: vec![
                proto::slide::Element {
                    element: Some(main_element),
                    ..Default::default()
                },
                proto::slide::Element {
                    element: Some(stage_element),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let content = super::slide_content_from_proto(&slide, None).expect("content");
        assert_eq!(content.main.value(), "Verse");
        assert!(
            content.stage.value().is_empty(),
            "stage placeholder should be stripped"
        );
    }

    #[test]
    fn slide_content_defaults_to_blank_when_no_elements_present() {
        let slide = proto::Slide::default();
        let content = super::slide_content_from_proto(&slide, None).expect("content");
        assert!(content.main.value().is_empty());
        assert!(content.translation.value().is_empty());
        assert!(content.stage.value().is_empty());
    }

    #[tokio::test]
    async fn parses_sample_presentation() {
        let Some(path) = library_path("IFY NSOHA/Gospel.pro") else {
            return;
        };
        let presentation = load_presentation_from_path(&path).expect("presentation to parse");
        assert!(presentation.name.starts_with("Gospel"));
        assert!(!presentation.slides.is_empty());
        let first_non_empty = presentation
            .slides
            .iter()
            .find(|slide| !slide.content.main.is_empty())
            .expect("expected at least one slide with content");
        assert!(first_non_empty
            .content
            .main
            .value()
            .contains("Talking that gospel"));
    }

    #[test]
    fn importer_sets_group_only_on_first_slide_per_section() {
        let Some(path) = library_path("IFY NSOHA/Gospel.pro") else {
            return;
        };
        let presentation = load_presentation_from_path(&path).expect("presentation to parse");
        assert!(presentation.slides.len() > 1, "expected multiple slides");

        let mut previous_group: Option<String> = None;
        let mut found_inherited = false;

        for slide in &presentation.slides {
            if let Some(group) = slide.content.group.as_ref() {
                let name = group.name().to_string();
                if previous_group.as_deref() == Some(name.as_str()) {
                    panic!(
                        "group '{}' should not repeat explicitly on consecutive slides",
                        name
                    );
                }
                previous_group = Some(name);
            } else if previous_group.is_some() {
                found_inherited = true;
            }
        }

        assert!(
            found_inherited,
            "expected at least one slide inheriting a group"
        );
    }

    #[test]
    fn presence_song_preserves_diacritics() {
        let Some(path) = library_path("NEW LEVEL/006 Presence.pro") else {
            return;
        };
        let presentation = load_presentation_from_path(&path).expect("presentation to parse");
        let all_text: String = presentation
            .slides
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains('Ž') || all_text.contains('ž'),
            "expected ž/Ž diacritics in 006 Presence but none found in decoded text"
        );
    }

    #[tokio::test]
    async fn imports_library_into_repository() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let importer = ProPresenterImporter::new(&repo);
        let Some(library_dir) = library_path("RACHEM") else {
            return;
        };
        let summary = importer
            .import_library_dir(&library_dir)
            .await
            .expect("import succeeds");
        assert!(summary.is_some());
        let summary = summary.unwrap();
        assert_eq!(summary.library_name, "RACHEM");
        assert_eq!(summary.presentation_count, 15);
        assert!(summary.slide_count > 0);

        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "RACHEM");
    }

    #[test]
    fn classify_text_role_case_insensitive() {
        assert_eq!(classify_text_role("MAIN"), TextRole::Main);
        assert_eq!(classify_text_role("main"), TextRole::Main);
        assert_eq!(classify_text_role("  Main  "), TextRole::Main);
    }

    #[test]
    fn classify_text_role_all_main_aliases() {
        for alias in &["lyrics", "text", "hlavny", "hlavný"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Main,
                "expected Main for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_all_translation_aliases() {
        for alias in &["translation", "preklad", "english", "eng"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Translation,
                "expected Translation for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_all_stage_aliases() {
        for alias in &["stage", "stage display", "stage text"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Stage,
                "expected Stage for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_empty_is_unknown() {
        assert_eq!(classify_text_role(""), TextRole::Unknown);
        assert_eq!(classify_text_role("  "), TextRole::Unknown);
    }

    #[test]
    fn slide_content_assigns_translation_and_stage_roles() {
        let main_text = proto::graphics::Text {
            rtf_data: b"Main lyrics".to_vec(),
            ..Default::default()
        };
        let translation_text = proto::graphics::Text {
            rtf_data: b"Translation text".to_vec(),
            ..Default::default()
        };
        let stage_text = proto::graphics::Text {
            rtf_data: b"Stage text".to_vec(),
            ..Default::default()
        };

        let slide = proto::Slide {
            elements: vec![
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Main".into(),
                        text: Some(main_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Translation".into(),
                        text: Some(translation_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Stage Display".into(),
                        text: Some(stage_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let content = slide_content_from_proto(&slide, None).expect("content");
        assert_eq!(content.main.value(), "Main lyrics");
        assert_eq!(content.translation.value(), "Translation text");
        assert_eq!(content.stage.value(), "Stage text");
    }

    #[test]
    fn build_group_lookup_assigns_anchor_to_first_cue() {
        let presentation = proto::Presentation {
            cue_groups: vec![proto::presentation::CueGroup {
                group: Some(Group {
                    name: "Verse 1".to_string(),
                    uuid: Some(proto::Uuid {
                        string: "group-1".to_string(),
                    }),
                    ..Default::default()
                }),
                cue_identifiers: vec![
                    proto::Uuid {
                        string: "cue-a".to_string(),
                    },
                    proto::Uuid {
                        string: "cue-b".to_string(),
                    },
                    proto::Uuid {
                        string: "cue-c".to_string(),
                    },
                ],
            }],
            ..Default::default()
        };

        let lookup = build_group_lookup(&presentation);
        assert_eq!(lookup.len(), 3);

        let cue_a = &lookup["cue-a"];
        assert_eq!(cue_a.name, "Verse 1");
        assert!(cue_a.anchor, "first cue should be anchor");

        let cue_b = &lookup["cue-b"];
        assert_eq!(cue_b.name, "Verse 1");
        assert!(!cue_b.anchor, "second cue should not be anchor");

        let cue_c = &lookup["cue-c"];
        assert!(!cue_c.anchor, "third cue should not be anchor");
    }

    #[test]
    fn build_group_lookup_skips_empty_uuids() {
        let presentation = proto::Presentation {
            cue_groups: vec![proto::presentation::CueGroup {
                group: Some(Group {
                    name: "Chorus".to_string(),
                    uuid: Some(proto::Uuid {
                        string: "group-1".to_string(),
                    }),
                    ..Default::default()
                }),
                cue_identifiers: vec![
                    proto::Uuid {
                        string: "cue-a".to_string(),
                    },
                    proto::Uuid {
                        string: String::new(),
                    },
                ],
            }],
            ..Default::default()
        };

        let lookup = build_group_lookup(&presentation);
        assert_eq!(lookup.len(), 1);
        assert!(lookup.contains_key("cue-a"));
    }
}
