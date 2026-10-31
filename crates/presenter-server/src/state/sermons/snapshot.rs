//! Item snapshots (#851 spec §7.1, §7.5): the database text of one verse range
//! when the item is added (or re-snapshotted). Reads go through the `AppState`
//! dispatch in `bible_source.rs` (bible-remote rule); the NLT is never an item
//! translation, so its text is never stored as a sermon source.

use chrono::Utc;
use presenter_core::bible::canonical_book_by_code;
use presenter_core::bible::sermon::{
    translation_short_code, ItemEdits, SourceVerse, MAX_ITEM_VERSES,
};
use presenter_persistence::SermonItemRecord;
use uuid::Uuid;

use super::super::AppState;
use super::SermonRefusal;
use crate::bible_remote;

/// One contiguous verse range of one chapter, as a request names it. A
/// cross-chapter selection is one range per chapter (the client splits it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemRange {
    pub(crate) translation: String,
    pub(crate) book_code: String,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    pub(crate) verse_end: u16,
}

impl AppState {
    /// A new item (fresh id, no edits) holding the database text of `range`;
    /// 422 with a Slovak message for an invalid range, more than 60 verses,
    /// the NLT, an unknown translation or book, or verses the translation lacks.
    pub(crate) async fn snapshot_item(
        &self,
        range: &ItemRange,
    ) -> anyhow::Result<SermonItemRecord> {
        check_range(range)?;
        let translation = self
            .list_bible_translations()
            .await?
            .into_iter()
            .find(|candidate| {
                candidate
                    .code
                    .eq_ignore_ascii_case(range.translation.trim())
            })
            .ok_or_else(|| {
                SermonRefusal::new(format!(
                    "Preklad {} nie je nainštalovaný",
                    range.translation
                ))
            })?;
        let canon = canonical_book_by_code(&range.book_code)
            .ok_or_else(|| SermonRefusal::new(format!("Neznáma kniha {}", range.book_code)))?;
        let rows = self
            .bible_passage_range(
                &translation.code,
                canon.english_name,
                Some(canon.code),
                range.chapter,
                range.verse_start,
                range.verse_end,
            )
            .await?;
        let complete = rows
            .first()
            .is_some_and(|first| first.reference.verse_start == range.verse_start)
            && rows
                .last()
                .is_some_and(|last| last.reference.verse_end == range.verse_end);
        let (Some(first), true) = (rows.first(), complete) else {
            let short = translation_short_code(&translation.code);
            let (chapter, start, end) = (range.chapter, range.verse_start, range.verse_end);
            return Err(SermonRefusal::new(format!(
                "{} {chapter}:{start}-{end} nie je celé v preklade {short}",
                canon.code
            ))
            .into());
        };
        let now = Utc::now();
        Ok(SermonItemRecord {
            id: Uuid::new_v4(),
            translation_code: translation.code.clone(),
            book_code: canon.code.to_string(),
            book_number: canon.number,
            book_name: first.reference.book.clone(),
            chapter: range.chapter,
            verse_start: range.verse_start,
            verse_end: range.verse_end,
            source: rows
                .iter()
                .map(|row| SourceVerse {
                    verse: row.reference.verse_start,
                    text: row.text.clone(),
                })
                .collect(),
            edits: ItemEdits::default(),
            slide_count: 0,
            created_at: now,
            updated_at: now,
        })
    }
}

fn check_range(range: &ItemRange) -> Result<(), SermonRefusal> {
    if range.chapter == 0 || range.verse_start == 0 || range.verse_end < range.verse_start {
        return Err(SermonRefusal::new("Neplatný rozsah veršov"));
    }
    if range.verse_end - range.verse_start >= MAX_ITEM_VERSES {
        return Err(SermonRefusal::new(format!(
            "Položka môže mať najviac {MAX_ITEM_VERSES} veršov"
        )));
    }
    if bible_remote::is_remote_translation(&range.translation) {
        return Err(SermonRefusal::new(
            "NLT sa do kázne neukladá — vyber iný preklad",
        ));
    }
    Ok(())
}
