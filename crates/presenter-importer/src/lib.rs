pub mod bible;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use encoding_rs::{Encoding, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1254};
use presenter_core::{Library, Presentation, Slide, SlideContent, SlideGroup, SlideText};
use presenter_persistence::Repository;
use prost::Message;
use rtf_parser::RtfDocument;
use tracing::instrument;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/rv.data.rs"));
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
    let raw = proto::Presentation::decode(bytes.as_slice())
        .with_context(|| format!("failed to decode ProPresenter data in {}", path.display()))?;
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
    let mut slides = Vec::new();

    for (index, cue) in raw.cues.iter().enumerate() {
        if let Some(slide_proto) = extract_presentation_slide(cue) {
            let base_slide = slide_proto
                .base_slide
                .as_ref()
                .ok_or_else(|| anyhow!("presentation slide missing base slide data"))?;
            let group = cue
                .uuid
                .as_ref()
                .and_then(|uuid| groups.get(&uuid.string))
                .and_then(|assignment| {
                    if assignment.anchor {
                        Some(SlideGroup::new(assignment.name.clone()))
                    } else {
                        None
                    }
                });
            let content = slide_content_from_proto(base_slide, group)?;
            slides.push(Slide::new(index as u32, content));
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
                if trimmed.is_empty() {
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
        .or_else(|| buckets.first().map(|(_, text)| text.clone()))
        .unwrap_or_default();
    let translation = select_text(&buckets, TextRole::Translation).unwrap_or_else(|| String::new());
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

pub fn extract_presentation_slide(cue: &proto::Cue) -> Option<&proto::PresentationSlide> {
    cue.actions.iter().find_map(
        |action| match proto::action::ActionType::from_i32(action.r#type) {
            Some(proto::action::ActionType::PresentationSlide) => {
                if let Some(proto::action::ActionTypeData::Slide(slide_type)) =
                    &action.action_type_data
                {
                    if let Some(proto::action::slide_type::Slide::Presentation(presentation)) =
                        &slide_type.slide
                    {
                        return Some(presentation);
                    }
                }
                None
            }
            _ => None,
        },
    )
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

pub fn decode_rtf(bytes: &[u8]) -> Result<String> {
    let raw = String::from_utf8_lossy(bytes).to_string();
    let encoding = detect_rtf_encoding(&raw);

    if let Ok(doc) = RtfDocument::try_from(raw.clone()) {
        let text = doc.get_text();
        if !contains_control_range(&text) {
            return Ok(clean_text(text));
        }
    }

    Ok(clean_text(fallback_rtf_to_text(&raw, encoding)))
}

fn detect_rtf_encoding(raw: &str) -> &'static Encoding {
    if let Some(idx) = raw.find("\\ansicpg") {
        let mut digits = String::new();
        for ch in raw[idx + 8..].chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else {
                break;
            }
        }
        if let Ok(codepage) = digits.parse::<u32>() {
            return match codepage {
                1250 => WINDOWS_1250,
                1251 => WINDOWS_1251,
                1252 => WINDOWS_1252,
                1254 => WINDOWS_1254,
                _ => WINDOWS_1250,
            };
        }
    }
    WINDOWS_1250
}

fn fallback_rtf_to_text(raw: &str, encoding: &'static Encoding) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut unicode_skip = 1usize;

    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' => continue,
            '\\' => match chars.peek().copied() {
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('{') => {
                    chars.next();
                    result.push('{');
                }
                Some('}') => {
                    chars.next();
                    result.push('}');
                }
                Some('\'') => {
                    chars.next();
                    if let Some(decoded) = decode_rtf_hex(&mut chars, encoding) {
                        result.push(decoded);
                    }
                }
                Some('u') => {
                    chars.next();
                    if let Some(decoded) = decode_rtf_unicode(&mut chars) {
                        result.push(decoded);
                    }
                    skip_unicode_fallback(&mut chars, unicode_skip);
                }
                Some(_) => {
                    let word = read_control_word(&mut chars);
                    match word.as_str() {
                        "par" | "line" => result.push('\n'),
                        word if word.starts_with("uc") => {
                            unicode_skip =
                                word.trim_start_matches("uc").parse::<usize>().unwrap_or(1);
                        }
                        _ => {}
                    }
                    if let Some(' ') = chars.peek() {
                        chars.next();
                    }
                }
                None => {}
            },
            _ => result.push(ch),
        }
    }

    result
}

fn decode_rtf_hex<I>(chars: &mut I, encoding: &'static Encoding) -> Option<char>
where
    I: Iterator<Item = char>,
{
    let mut hex = String::new();
    for _ in 0..2 {
        if let Some(c) = chars.next() {
            hex.push(c);
        } else {
            return None;
        }
    }
    let byte = u8::from_str_radix(&hex, 16).ok()?;
    let binding = [byte];
    let (converted, _, had_errors) = encoding.decode(&binding);
    if had_errors {
        Some(byte as char)
    } else {
        converted.chars().next()
    }
}

fn decode_rtf_unicode<I>(chars: &mut std::iter::Peekable<I>) -> Option<char>
where
    I: Iterator<Item = char>,
{
    let mut numeric = String::new();
    if let Some('-') = chars.peek().copied() {
        numeric.push('-');
        chars.next();
    }
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            numeric.push(c);
            chars.next();
        } else {
            break;
        }
    }
    let value = numeric.parse::<i32>().ok()?;
    if value >= 0 {
        char::from_u32(value as u32)
    } else {
        char::from_u32((65536 + value) as u32)
    }
}

fn skip_unicode_fallback<I>(chars: &mut std::iter::Peekable<I>, count: usize)
where
    I: Iterator<Item = char>,
{
    for _ in 0..count {
        if !consume_fallback_char(chars) {
            break;
        }
    }
    while matches!(chars.peek(), Some('?')) {
        chars.next();
    }
}

fn consume_fallback_char<I>(chars: &mut std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = char>,
{
    match chars.peek().copied() {
        Some('\\') => {
            chars.next();
            match chars.peek().copied() {
                Some('\'') => {
                    chars.next();
                    for _ in 0..2 {
                        if chars.next().is_none() {
                            break;
                        }
                    }
                    true
                }
                Some(_) => {
                    let _ = read_control_word(chars);
                    true
                }
                None => false,
            }
        }
        Some(_) => {
            chars.next();
            true
        }
        None => false,
    }
}

fn read_control_word<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    let mut word = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    while let Some(&c) = chars.peek() {
        if c == '-' || c.is_ascii_digit() {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    word
}

fn contains_control_range(text: &str) -> bool {
    text.chars().any(|ch| matches!(ch as u32, 0x80..=0x9F))
}

fn clean_text(text: String) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut prev_alpha = false;
    let mut prev_boundary = true;
    let mut prev_lower = false;
    let mut iter = text.chars().peekable();
    while let Some(ch) = iter.next() {
        if is_formatting_char(ch) {
            continue;
        }
        if ch == '\u{00A0}' {
            cleaned.push(' ');
            prev_alpha = false;
            prev_boundary = true;
            prev_lower = false;
            continue;
        }
        if ch == '0' {
            if let Some(&next) = iter.peek() {
                if next > '\u{007F}' && next.is_alphabetic() && (prev_alpha || prev_boundary) {
                    continue;
                }
            }
        }
        let mut output = ch;
        if ch.is_uppercase() && prev_lower {
            let mut lower = ch.to_lowercase();
            if let Some(first) = lower.next() {
                if lower.next().is_none() {
                    output = first;
                }
            }
        }
        prev_alpha = output.is_alphabetic();
        prev_lower = output.is_lowercase();
        prev_boundary = output.is_whitespace()
            || output.is_ascii_punctuation()
            || matches!(output, '\n' | '\r');
        cleaned.push(output);
    }
    strip_header_lines(cleaned)
}

fn is_formatting_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'
            | '\u{202B}'
            | '\u{202C}'
            | '\u{202D}'
            | '\u{202E}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
            | '\u{00AD}'
    )
}

fn strip_header_lines(input: String) -> String {
    let mut lines = Vec::new();
    let mut seen_content = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if !seen_content {
            if trimmed.is_empty() || is_header_line(trimmed) {
                continue;
            }
            seen_content = true;
        }
        lines.push(line);
    }
    if !seen_content {
        return String::new();
    }
    lines.join("\n")
}

fn is_header_line(line: &str) -> bool {
    if line.is_empty() {
        return true;
    }
    if line.chars().all(|c| matches!(c, ';' | '*' | '-')) {
        return true;
    }
    if line.ends_with(';') && !line.contains(' ') {
        let body = &line[..line.len() - 1];
        if !body.is_empty()
            && body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return true;
        }
    }
    false
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

    if MAIN_ALIASES.iter().any(|alias| normalized.contains(alias)) {
        return TextRole::Main;
    }
    if TRANSLATION_ALIASES
        .iter()
        .any(|alias| normalized.contains(alias))
    {
        return TextRole::Translation;
    }
    if STAGE_ALIASES.iter().any(|alias| normalized.contains(alias)) {
        return TextRole::Stage;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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
    fn decode_rtf_handles_slovak_characters() {
        let raw = br"{\rtf1\ansi\ansicpg1250 Slov\'e1k\par}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Slovák"));
    }

    #[test]
    fn decode_rtf_fallback_handles_hex_and_unicode() {
        let raw = br"Slov\'e1k \u353?peci\'e1l";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Slovák"));
        assert!(text.contains("špeciál"));
    }

    #[test]
    fn decode_rtf_retains_central_european_letters() {
        let raw = br"{\rtf1\ansi\ansicpg1252 Ka\'9ed\'fd dobr\'fd \u250\'fadel a ka\'9ed\'fd dokonal\'fd dar poch\'e1dza zhora od Otca svetiel, v ktorom niet premeny ani tie\uc0\u328 a zmeny.}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Každý dobrý údel"), "decoded={text}");
        assert!(
            text.contains("niet premeny ani tieňa zmeny"),
            "decoded={text}"
        );
    }

    #[test]
    fn clean_text_drops_spurious_zero_before_caron() {
        let sample = "Monaco;\n\nA niet v 0ňom žiadne klamstvá";
        let cleaned = super::clean_text(sample.to_string());
        assert!(cleaned.contains("A niet v ňom"), "cleaned={cleaned}");
        assert!(!cleaned.contains("0ň"), "cleaned={cleaned}");
    }

    #[test]
    fn clean_text_strips_leading_font_headers() {
        let sample = "Monaco;\n;;;\n*;;;\n\n\nA kto to zmeniť chce musí sa znova narodiť";
        let cleaned = super::clean_text(sample.to_string());
        assert_eq!(
            cleaned, "A kto to zmeniť chce musí sa znova narodiť",
            "cleaned={cleaned}"
        );
    }

    #[tokio::test]
    async fn parses_sample_presentation() {
        let path = repo_root().join("Propresenter library/IFY NSOHA/Gospel.pro");
        let presentation = load_presentation_from_path(&path).expect("presentation to parse");
        assert!(presentation.name.starts_with("Gospel"));
        assert!(!presentation.slides.is_empty());
        assert!(presentation.slides[0]
            .content
            .main
            .value()
            .contains("I came here"));
    }

    #[test]
    fn importer_sets_group_only_on_first_slide_per_section() {
        let path = repo_root().join("Propresenter library/IFY NSOHA/Gospel.pro");
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

    #[tokio::test]
    async fn imports_library_into_repository() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let importer = ProPresenterImporter::new(&repo);
        let library_dir = repo_root().join("Propresenter library/RACHEM");
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
}
