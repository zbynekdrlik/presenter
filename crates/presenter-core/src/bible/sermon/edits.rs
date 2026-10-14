//! Validation and upkeep of a sermon item's edits (#851 spec §7.2, §7.5).

use super::{tokenize, ItemEdits, SourceVerse, VerseEdits};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Why edits were refused (the server answers 422 with this Slovak text).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    #[error("Verš {verse} nie je v tejto položke")]
    UnknownVerse { verse: u16 },
    #[error("Verš {verse} nemá slovo s indexom {index}")]
    IndexOutOfRange { verse: u16, index: u16 },
}

/// The edits checked against `source` and normalized: every verse key exists,
/// every index points at a token, indices are sorted and deduplicated, a
/// struck token is not emphasized, and untouched verses are dropped.
pub fn normalize_edits(source: &[SourceVerse], edits: &ItemEdits) -> Result<ItemEdits, EditError> {
    let mut normalized = BTreeMap::new();
    for (&verse, verse_edits) in &edits.0 {
        let Some(source_verse) = source.iter().find(|candidate| candidate.verse == verse) else {
            return Err(EditError::UnknownVerse { verse });
        };
        let len = tokenize(&source_verse.text).len();
        let deleted = checked_indices(verse, &verse_edits.deleted, len)?;
        let emphasized = checked_indices(verse, &verse_edits.emphasized, len)?;
        let emphasized: Vec<u16> = emphasized.difference(&deleted).copied().collect();
        if deleted.is_empty() && emphasized.is_empty() {
            continue;
        }
        normalized.insert(
            verse,
            VerseEdits {
                deleted: deleted.into_iter().collect(),
                emphasized,
            },
        );
    }
    Ok(ItemEdits(normalized))
}

fn checked_indices(verse: u16, indices: &[u16], len: usize) -> Result<BTreeSet<u16>, EditError> {
    let mut checked = BTreeSet::new();
    for &index in indices {
        if usize::from(index) >= len {
            return Err(EditError::IndexOutOfRange { verse, index });
        }
        checked.insert(index);
    }
    Ok(checked)
}

/// "Obnoviť text" (spec §7.5): the edits of the verses whose text is the same
/// in `old` and `new`; the edits of a changed or vanished verse are cleared.
pub fn retain_unchanged_edits(
    old: &[SourceVerse],
    new: &[SourceVerse],
    edits: &ItemEdits,
) -> ItemEdits {
    let text_of = |source: &[SourceVerse], verse: u16| {
        source
            .iter()
            .find(|candidate| candidate.verse == verse)
            .map(|candidate| candidate.text.clone())
    };
    ItemEdits(
        edits
            .0
            .iter()
            .filter(|(verse, _)| {
                let before = text_of(old, **verse);
                before.is_some() && before == text_of(new, **verse)
            })
            .map(|(verse, verse_edits)| (*verse, verse_edits.clone()))
            .collect(),
    )
}
