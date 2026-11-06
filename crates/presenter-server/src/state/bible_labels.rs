//! The wall output of a legacy `/bible/trigger` (Companion, the AI
//! `trigger_bible_verse` tool, #850 A2b spec §5.10): the same
//! `BibleSlideOutput` every other path sends, so Resolume, the stage and the
//! on-air strip get one shape of output.

use presenter_core::bible::sermon::translation_short_code;
use presenter_core::{BibleBroadcast, BibleReference, BibleSlideOutput};

/// The secondary side of a legacy trigger (#824): its text, its translation
/// code and the book name that translation uses (for the translate reference).
#[derive(Debug, Default)]
pub(super) struct TriggerSecondary {
    pub(super) text: Option<String>,
    pub(super) translation_code: Option<String>,
    pub(super) book: Option<String>,
}

/// What a legacy trigger puts on the wall:
///   - main text = the passage text (the numbered verses or the edited text);
///   - main reference = `"{stored book} {c}:{v[-v]} ({SHORT})"`;
///   - secondary reference = only with a secondary text, under the
///     secondary translation's book name (#824).
pub(super) fn legacy_slide_output(
    broadcast: &BibleBroadcast,
    secondary: &TriggerSecondary,
) -> BibleSlideOutput {
    let passage = &broadcast.passage;
    BibleSlideOutput::new(
        passage.text.clone(),
        format!(
            "{} ({})",
            passage.reference.to_human_readable(),
            translation_short_code(&passage.translation.code)
        ),
        secondary.text.clone().unwrap_or_default(),
        legacy_translation_reference(
            &passage.reference,
            secondary.book.as_deref(),
            secondary
                .text
                .as_ref()
                .and(secondary.translation_code.as_deref()),
        ),
        broadcast.triggered_at,
    )
}

/// The secondary reference of a legacy trigger (#824): the main reference
/// with the SECONDARY translation's book name — "1 John 1:1-3 (KJV)", not the
/// main "1 Ján 1:1-3 (KJV)". Without a secondary book the main name stays;
/// without a secondary translation the text is empty.
fn legacy_translation_reference(
    reference: &BibleReference,
    secondary_book: Option<&str>,
    secondary_translation_code: Option<&str>,
) -> String {
    let Some(code) = secondary_translation_code else {
        return String::new();
    };
    let mut reference = reference.clone();
    if let Some(book) = secondary_book.filter(|book| !book.trim().is_empty()) {
        reference.book = book.to_string();
    }
    format!(
        "{} ({})",
        reference.to_human_readable(),
        translation_short_code(code)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_jan(verse_start: u16, verse_end: u16) -> BibleReference {
        BibleReference::new_with_code("1 Ján", "1JN", 62, 1, verse_start, verse_end)
            .expect("reference")
    }

    #[test]
    fn the_translation_reference_names_the_secondary_book() {
        assert_eq!(
            legacy_translation_reference(&first_jan(1, 3), Some("1 John"), Some("eng-kjv")),
            "1 John 1:1-3 (KJV)"
        );
    }

    #[test]
    fn without_a_secondary_book_the_main_book_name_stays() {
        assert_eq!(
            legacy_translation_reference(&first_jan(1, 1), None, Some("eng-kjv")),
            "1 Ján 1:1 (KJV)"
        );
        assert_eq!(
            legacy_translation_reference(&first_jan(1, 1), Some("  "), Some("eng-kjv")),
            "1 Ján 1:1 (KJV)"
        );
    }

    #[test]
    fn without_a_secondary_translation_there_is_no_translation_reference() {
        assert_eq!(
            legacy_translation_reference(&first_jan(1, 3), Some("1 John"), None),
            ""
        );
    }

    #[test]
    fn a_secondary_translation_without_text_has_no_secondary_reference() {
        let passage = presenter_core::BiblePassage::new(
            first_jan(1, 1),
            presenter_core::BibleTranslation::new("slk-seb", "SEB", "sk"),
            "1. Čo bolo od počiatku.".to_string(),
        );
        let broadcast = BibleBroadcast::new(passage, chrono::Utc::now());
        let secondary = TriggerSecondary {
            text: None,
            translation_code: Some("eng-kjv".to_string()),
            book: None,
        };

        let output = legacy_slide_output(&broadcast, &secondary);

        assert_eq!(output.main_reference, "1 Ján 1:1 (SEB)");
        assert_eq!(output.secondary_text, "");
        assert_eq!(output.secondary_reference, "");
    }
}
