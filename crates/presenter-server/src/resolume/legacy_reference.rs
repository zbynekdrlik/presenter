//! Legacy Bible push text (#824 phase 2): the `#bible-translate-reference`
//! label of the deprecated `/bible/trigger` path (Companion, the AI
//! `trigger_bible` tool). The live path sends the composer's label as is.

use super::handlers::translation_short_code;

/// `#bible-translate-reference` text of a legacy Bible push (#824): the main
/// reference with the SECONDARY translation's book name — "1 John 1:1-3 (KJV)",
/// not the main "1 Ján 1:1-3 (KJV)". Without a secondary book the main name
/// stays; without a secondary translation the text is empty.
pub(super) fn legacy_translation_reference(
    reference: &presenter_core::BibleReference,
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
