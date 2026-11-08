//! Verses from the operator console into a sermon (#917). There is one kind of Bible
//! presentation for the operator: the console's "add to presentation" works on a sermon
//! too. A sermon's slides are generated from its items, so the appended slides are not
//! stored as slides — each run of verses becomes an item (snapshot of the database text),
//! committed in ONE sermon write like `add_sermon_item`. Consecutive slides of the same
//! translation, book and chapter whose verses follow each other form one item (a passage
//! the console split over several slides), at most `MAX_ITEM_VERSES` verses.

use presenter_core::bible::sermon::MAX_ITEM_VERSES;
use presenter_core::{BiblePresentation, BiblePresentationId, BiblePresentationSlide};
use presenter_persistence::RepositoryError;

use super::super::AppState;
use super::snapshot::ItemRange;
use super::{SermonOp, SermonOrigin, SermonRefusal};

const NOT_A_VERSE: &str = "Do kázne sa pridávajú len verše z Biblie";

/// The item ranges of `slides`, in order: one per run of adjacent verses of the
/// same translation, book and chapter. A slide without Bible verses (an empty or a
/// text slide) is refused — a sermon holds only Bible items.
pub(super) fn item_ranges(
    slides: &[BiblePresentationSlide],
) -> Result<Vec<ItemRange>, SermonRefusal> {
    let mut ranges: Vec<ItemRange> = Vec::new();
    for slide in slides {
        let meta = slide
            .metadata
            .as_ref()
            .filter(|meta| !meta.verses.is_empty() && !meta.translation_code.trim().is_empty())
            .ok_or_else(|| SermonRefusal::new(NOT_A_VERSE))?;
        let book_code = meta
            .book_code
            .as_deref()
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .ok_or_else(|| SermonRefusal::new(NOT_A_VERSE))?;
        for verse in &meta.verses {
            let next = ItemRange {
                translation: meta.translation_code.trim().to_string(),
                book_code: book_code.to_string(),
                chapter: meta.chapter,
                verse_start: verse.start,
                verse_end: verse.end,
            };
            match ranges.last_mut() {
                Some(last) if continues(last, &next) => last.verse_end = next.verse_end,
                _ => ranges.push(next),
            }
        }
    }
    Ok(ranges)
}

/// `next` extends `last` into one item: same place, the very next verse, and the
/// merged item stays within `MAX_ITEM_VERSES`.
fn continues(last: &ItemRange, next: &ItemRange) -> bool {
    last.translation.eq_ignore_ascii_case(&next.translation)
        && last.book_code.eq_ignore_ascii_case(&next.book_code)
        && last.chapter == next.chapter
        && next.verse_start == last.verse_end.saturating_add(1)
        && next.verse_end >= next.verse_start
        && next.verse_end - last.verse_start < MAX_ITEM_VERSES
}

impl AppState {
    /// Append `slides` to the sermon `id` as items (one sermon write, `op=add`)
    /// and answer the presentation with the regenerated slides. 422
    /// (`SermonRefusal`) for a slide without verses or a range the sermon
    /// cannot hold (the NLT, more than 60 verses, verses the translation lacks).
    pub(crate) async fn append_to_sermon(
        &self,
        id: BiblePresentationId,
        slides: &[BiblePresentationSlide],
        origin: &SermonOrigin,
    ) -> anyhow::Result<BiblePresentation> {
        let ranges = item_ranges(slides)?;
        let mut snapshot = self.load_sermon(id).await?;
        let expected_revision = snapshot.sermon.revision;
        let mut last_item = None;
        for range in &ranges {
            let item = self.snapshot_item(range).await?;
            last_item = Some(item.id);
            snapshot.items.push(item);
        }
        if last_item.is_some() {
            self.commit_sermon(super::commit::SermonChange {
                op: SermonOp::Add,
                origin,
                expected_revision,
                sermon: snapshot.sermon,
                items: snapshot.items,
                item_id: last_item,
            })
            .await?;
        }
        Ok(self
            .repository
            .fetch_bible_presentation(id)
            .await?
            .ok_or(RepositoryError::NotFound("bible presentation not found"))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef};
    use presenter_core::{BibleSlideId, SlideText};

    fn slide(translation: &str, chapter: u16, verses: &[(u16, u16)]) -> BiblePresentationSlide {
        BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::from_stored_unchecked("x"),
            main_reference: String::new(),
            secondary: SlideText::from_stored_unchecked(""),
            secondary_reference: String::new(),
            metadata: Some(BibleSlideMetadata {
                translation_code: translation.to_string(),
                secondary_translation_code: None,
                book: "Hebrejom".to_string(),
                book_code: Some("HEB".to_string()),
                book_number: Some(58),
                chapter,
                verses: verses
                    .iter()
                    .map(|(s, e)| BibleSlideVerseRef::new(*s, *e))
                    .collect(),
                main_reference_label: None,
                translation_reference_label: None,
            }),
        }
    }

    fn spans(ranges: &[ItemRange]) -> Vec<(String, u16, u16, u16)> {
        ranges
            .iter()
            .map(|r| (r.translation.clone(), r.chapter, r.verse_start, r.verse_end))
            .collect()
    }

    #[test]
    fn adjacent_verses_of_one_place_merge_into_one_item() {
        let ranges = item_ranges(&[
            slide("slk-seb", 11, &[(1, 2)]),
            slide("slk-seb", 11, &[(3, 3)]),
        ])
        .expect("ranges");
        assert_eq!(spans(&ranges), vec![("slk-seb".to_string(), 11, 1, 3)]);
    }

    #[test]
    fn a_gap_another_chapter_or_translation_starts_a_new_item() {
        let ranges = item_ranges(&[
            slide("slk-seb", 11, &[(1, 1)]),
            slide("slk-seb", 11, &[(3, 3)]),
            slide("slk-seb", 12, &[(4, 4)]),
            slide("eng-kjv", 12, &[(5, 5)]),
        ])
        .expect("ranges");
        assert_eq!(
            spans(&ranges),
            vec![
                ("slk-seb".to_string(), 11, 1, 1),
                ("slk-seb".to_string(), 11, 3, 3),
                ("slk-seb".to_string(), 12, 4, 4),
                ("eng-kjv".to_string(), 12, 5, 5),
            ]
        );
    }

    #[test]
    fn a_merge_never_passes_sixty_verses() {
        let ranges = item_ranges(&[
            slide("slk-seb", 119, &[(1, 59)]),
            slide("slk-seb", 119, &[(60, 60)]),
            slide("slk-seb", 119, &[(61, 61)]),
        ])
        .expect("ranges");
        assert_eq!(
            spans(&ranges),
            vec![
                ("slk-seb".to_string(), 119, 1, 60),
                ("slk-seb".to_string(), 119, 61, 61),
            ]
        );
    }

    #[test]
    fn a_slide_without_verses_or_book_code_is_refused() {
        let mut empty = slide("slk-seb", 11, &[(1, 1)]);
        empty.metadata = None;
        assert_eq!(item_ranges(&[empty]).expect_err("no verses").0, NOT_A_VERSE);
        let mut no_verses = slide("slk-seb", 11, &[]);
        no_verses.metadata.as_mut().expect("meta").verses.clear();
        assert_eq!(
            item_ranges(&[no_verses]).expect_err("no verses").0,
            NOT_A_VERSE
        );
        let mut no_book = slide("slk-seb", 11, &[(1, 1)]);
        no_book.metadata.as_mut().expect("meta").book_code = None;
        assert_eq!(item_ranges(&[no_book]).expect_err("no book").0, NOT_A_VERSE);
    }
}
