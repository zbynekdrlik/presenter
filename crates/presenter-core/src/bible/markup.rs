//! A verse as ordered segments (#851 D2a, spec §5.2): the plain `content` is
//! derived from them, so the words a study page shows and the words a wall
//! shows cannot drift apart. Pure: used by the importer, the server and WASM.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VerseMarkup(pub Vec<MarkupSegment>);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MarkupSegment {
    #[serde(rename = "t")]
    pub text: String,
    #[serde(rename = "k", default, skip_serializing_if = "SegmentKind::is_text")]
    pub kind: SegmentKind,
    #[serde(rename = "s", default, skip_serializing_if = "Vec::is_empty")]
    pub strongs: Vec<String>,
    #[serde(rename = "i", default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    #[default]
    Text,
    Note,
    Heading,
}

impl SegmentKind {
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text)
    }
}

impl MarkupSegment {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn note(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SegmentKind::Note,
            ..Self::default()
        }
    }

    pub fn heading(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SegmentKind::Heading,
            ..Self::default()
        }
    }

    /// Plain verse text with nothing to tell about it.
    fn is_bare_text(&self) -> bool {
        self.kind.is_text() && self.strongs.is_empty() && !self.italic
    }
}

impl VerseMarkup {
    /// The Text segments joined: equal to `bible_passages.content`.
    pub fn plain_text(&self) -> String {
        self.0
            .iter()
            .filter(|segment| segment.kind.is_text())
            .map(|segment| segment.text.as_str())
            .collect()
    }

    /// Whether the markup says more than `content` does: only such a verse
    /// is worth a row.
    pub fn has_markup(&self) -> bool {
        self.0.iter().any(|segment| !segment.is_bare_text())
    }

    /// `"G4102"` → tagged words in this verse.
    pub fn strongs_counts(&self) -> BTreeMap<String, u16> {
        let mut counts = BTreeMap::new();
        for strongs in self.0.iter().flat_map(|segment| &segment.strongs) {
            let count = counts.entry(strongs.clone()).or_insert(0_u16);
            *count = count.saturating_add(1);
        }
        counts
    }
}

/// `"H0430"` → `"H430"`, `"g26"` → `"G26"`, anything else `None`.
pub fn normalize_strongs(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let mut chars = raw.chars();
    let language = chars.next()?.to_ascii_uppercase();
    let digits = chars.as_str();
    let number: u32 = match language {
        'H' | 'G' if digits.chars().all(|c| c.is_ascii_digit()) => digits.parse().ok()?,
        _ => return None,
    };
    (number > 0).then(|| format!("{language}{number}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bible::BibleTranslation;
    use crate::bible::{BibleIngestionBatch, BibleIngestionError, BiblePassage, BibleReference};

    #[test]
    fn strongs_numbers_lose_their_zeros_and_gain_a_capital() {
        assert_eq!(normalize_strongs("H0430").as_deref(), Some("H430"));
        assert_eq!(normalize_strongs("g26").as_deref(), Some("G26"));
        for bad in ["", "H", "H0", "X12", "H12a", "12", "G 1", "G+1"] {
            assert_eq!(normalize_strongs(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_verse_serializes_in_its_short_form() {
        let verse = VerseMarkup(vec![
            MarkupSegment::heading("Nadpis"),
            MarkupSegment {
                text: "faith".to_string(),
                strongs: vec!["G4102".to_string()],
                ..MarkupSegment::default()
            },
            MarkupSegment::text(" "),
            MarkupSegment {
                text: "is".to_string(),
                italic: true,
                ..MarkupSegment::default()
            },
            MarkupSegment::note("Heb. x"),
        ]);
        let json = serde_json::to_string(&verse).expect("serialize");
        assert_eq!(
            json,
            r#"[{"t":"Nadpis","k":"heading"},{"t":"faith","s":["G4102"]},{"t":" "},{"t":"is","i":true},{"t":"Heb. x","k":"note"}]"#
        );
        let back: VerseMarkup = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, verse);
    }

    #[test]
    fn plain_text_is_only_the_text_segments() {
        let verse = VerseMarkup(vec![
            MarkupSegment::heading("Nadpis"),
            MarkupSegment::text("A "),
            MarkupSegment::note("pozn."),
            MarkupSegment::text("B"),
        ]);
        assert_eq!(verse.plain_text(), "A B");
    }

    #[test]
    fn only_a_verse_with_something_to_tell_has_markup() {
        assert!(!VerseMarkup(vec![MarkupSegment::text("A")]).has_markup());
        assert!(VerseMarkup(vec![MarkupSegment::note("n")]).has_markup());
        assert!(VerseMarkup(vec![MarkupSegment::heading("n")]).has_markup());
        let italic = MarkupSegment {
            italic: true,
            ..MarkupSegment::text("A")
        };
        assert!(VerseMarkup(vec![italic]).has_markup());
    }

    fn batch(text: &str) -> BibleIngestionBatch {
        let translation = BibleTranslation::new("eng-kjv", "KJV", "en");
        let reference = BibleReference::new("Genesis", 1, 1, 1).expect("reference");
        let passage = BiblePassage::new(reference, translation.clone(), text.to_string());
        BibleIngestionBatch::new(translation, vec![passage]).expect("batch")
    }

    #[test]
    fn a_batch_keeps_markup_that_spells_its_passage() {
        let markup = VerseMarkup(vec![MarkupSegment::note("n"), MarkupSegment::text("A")]);
        let batch = batch("A")
            .with_markup(vec![Some(markup.clone())])
            .expect("markup");
        assert_eq!(batch.markup(), &[Some(markup.clone())]);
        let (_, passages, entries) = batch.into_parts_with_markup();
        assert_eq!((passages.len(), entries), (1, vec![Some(markup)]));
    }

    #[test]
    fn a_batch_starts_without_markup() {
        assert_eq!(batch("A").markup(), &[None]);
    }

    #[test]
    fn a_batch_refuses_markup_of_another_text_or_length() {
        let other = VerseMarkup(vec![MarkupSegment::text("B")]);
        assert!(matches!(
            batch("A").with_markup(vec![Some(other)]),
            Err(BibleIngestionError::MarkupText { .. })
        ));
        assert_eq!(
            batch("A").with_markup(vec![]).err(),
            Some(BibleIngestionError::MarkupCount {
                expected: 1,
                found: 0
            })
        );
    }

    #[test]
    fn strongs_counts_count_the_tagged_words() {
        let word = |strongs: &str| MarkupSegment {
            text: "w".to_string(),
            strongs: vec![strongs.to_string()],
            ..MarkupSegment::default()
        };
        let verse = VerseMarkup(vec![
            word("G1"),
            MarkupSegment::text(" "),
            word("G1"),
            word("H2"),
        ]);
        let counts = verse.strongs_counts();
        assert_eq!(counts.get("G1"), Some(&2));
        assert_eq!(counts.get("H2"), Some(&1));
        assert_eq!(counts.len(), 2);
    }
}
