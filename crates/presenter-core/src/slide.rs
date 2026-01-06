use crate::id::SlideId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum characters permitted within a slide text field.
const SLIDE_TEXT_LIMIT: usize = 4000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SlideValidationError {
    #[error("slide text exceeds {limit} characters")]
    TextTooLong { limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideText {
    value: String,
}

impl SlideText {
    pub fn new<T: Into<String>>(value: T) -> Result<Self, SlideValidationError> {
        let value = value.into();
        if value.chars().count() > SLIDE_TEXT_LIMIT {
            return Err(SlideValidationError::TextTooLong {
                limit: SLIDE_TEXT_LIMIT,
            });
        }
        Ok(Self { value })
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideGroup {
    name: String,
}

impl SlideGroup {
    pub fn new<T: Into<String>>(name: T) -> Self {
        Self { name: name.into() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideVerseRef {
    pub start: u16,
    pub end: u16,
}

impl BibleSlideVerseRef {
    pub fn new(start: u16, end: u16) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideMetadata {
    pub translation_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_translation_code: Option<String>,
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verses: Vec<BibleSlideVerseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_reference_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation_reference_label: Option<String>,
}

impl BibleSlideMetadata {
    pub fn verse_span(&self) -> Option<(u16, u16)> {
        let first = self.verses.first()?;
        let last = self.verses.last()?;
        Some((first.start, last.end))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bible: Option<BibleSlideMetadata>,
}

impl SlideMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bible(mut self, metadata: BibleSlideMetadata) -> Self {
        self.bible = Some(metadata);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideContent {
    pub main: SlideText,
    pub translation: SlideText,
    pub stage: SlideText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<SlideGroup>,
}

impl SlideContent {
    pub fn new(
        main: SlideText,
        translation: SlideText,
        stage: SlideText,
        group: Option<SlideGroup>,
    ) -> Self {
        Self {
            main,
            translation,
            stage,
            group,
        }
    }

    pub fn with_group(mut self, group: Option<SlideGroup>) -> Self {
        self.group = group;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    pub id: SlideId,
    pub order: u32,
    pub content: SlideContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SlideMetadata>,
}

impl Slide {
    pub fn new(order: u32, content: SlideContent) -> Self {
        Self {
            id: SlideId::new(),
            order,
            content,
            metadata: None,
        }
    }

    pub fn with_id(mut self, id: SlideId) -> Self {
        self.id = id;
        self
    }

    pub fn with_metadata(mut self, metadata: Option<SlideMetadata>) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSlide {
    pub id: SlideId,
    pub order: u32,
    pub main: SlideText,
    pub translation: SlideText,
    pub stage: SlideText,
    pub effective_group: Option<SlideGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SlideMetadata>,
}

pub fn resolve_sequence(slides: &[Slide]) -> Vec<ResolvedSlide> {
    let mut active_group: Option<SlideGroup> = None;
    slides
        .iter()
        .map(|slide| {
            if let Some(group) = slide.content.group.as_ref() {
                active_group = Some(group.clone());
            }
            ResolvedSlide {
                id: slide.id,
                order: slide.order,
                main: slide.content.main.clone(),
                translation: slide.content.translation.clone(),
                stage: slide.content.stage.clone(),
                effective_group: active_group.clone(),
                metadata: slide.metadata.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_text_rejects_long_payloads() {
        let long_text = "a".repeat(SLIDE_TEXT_LIMIT + 1);
        let err = SlideText::new(long_text).unwrap_err();
        assert_eq!(
            err,
            SlideValidationError::TextTooLong {
                limit: SLIDE_TEXT_LIMIT
            }
        );
    }

    #[test]
    fn resolve_sequence_propagates_groups_forward() {
        let slides = vec![
            Slide::new(
                0,
                SlideContent::new(
                    SlideText::new("A").unwrap(),
                    SlideText::new("A tr").unwrap(),
                    SlideText::new("A stage").unwrap(),
                    Some(SlideGroup::new("Verse 1")),
                ),
            ),
            Slide::new(
                1,
                SlideContent::new(
                    SlideText::new("B").unwrap(),
                    SlideText::new("B tr").unwrap(),
                    SlideText::new("B stage").unwrap(),
                    None,
                ),
            ),
            Slide::new(
                2,
                SlideContent::new(
                    SlideText::new("C").unwrap(),
                    SlideText::new("C tr").unwrap(),
                    SlideText::new("C stage").unwrap(),
                    Some(SlideGroup::new("Chorus")),
                ),
            ),
        ];

        let resolved = resolve_sequence(&slides);
        let groups: Vec<_> = resolved
            .iter()
            .map(|s| s.effective_group.as_ref().map(|g| g.name().to_string()))
            .collect();

        assert_eq!(
            groups,
            vec![
                Some("Verse 1".to_string()),
                Some("Verse 1".to_string()),
                Some("Chorus".to_string())
            ]
        );
    }

    #[test]
    fn resolve_sequence_handles_missing_group() {
        let slides = vec![Slide::new(
            0,
            SlideContent::new(
                SlideText::new("Intro").unwrap(),
                SlideText::new("Intro tr").unwrap(),
                SlideText::new("Intro stage").unwrap(),
                None,
            ),
        )];

        let resolved = resolve_sequence(&slides);
        assert!(resolved[0].effective_group.is_none());
    }
}
