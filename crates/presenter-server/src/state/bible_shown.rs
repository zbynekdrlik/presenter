//! "Táto bohoslužba" (#850 B2d, spec §7.4): the verses shown on the wall today,
//! shared by every device, because there is one wall. In memory only: a
//! restart starts the list again, and no event carries it, the console reads
//! `GET /bible/shown-today` when the wall changes.

use chrono::NaiveDate;
use presenter_core::BibleBroadcast;
use serde::Serialize;

use super::bible_show_log::{BibleShowOrigin, BibleShowSource};
use super::bible_source::canonical_book;
use super::sermons::local_today;
use super::AppState;

const MAX_SHOWN: usize = 20;

/// One verse the console can bring back; the code keeps it valid in another
/// translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShownVerse {
    /// `Hebrejom 11:1`, the list's identity.
    pub(crate) label: String,
    pub(crate) book: String,
    pub(crate) book_code: String,
    pub(crate) book_number: u16,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    pub(crate) verse_end: u16,
    pub(crate) translation_code: String,
}

impl ShownVerse {
    /// `None` for a show the console could not bring back: a book nobody knows
    /// (a placeholder reference), or a sermon's slide. That one holds the
    /// preacher's cut, which a preview of the verse would lose, and it already
    /// has its row under "Pripravené kazateľom". Companion and the AI name only
    /// the book, so the code comes from its name.
    fn of_show(broadcast: &BibleBroadcast, origin: &BibleShowOrigin) -> Option<Self> {
        if matches!(origin.source, BibleShowSource::Sermon) {
            return None;
        }
        let reference = &broadcast.passage.reference;
        let book = canonical_book(&reference.book, reference.book_code.as_deref())?;
        Some(Self {
            label: reference.to_human_readable(),
            book: reference.book.clone(),
            book_code: book.code.to_string(),
            book_number: book.number,
            chapter: reference.chapter,
            verse_start: reference.verse_start,
            verse_end: reference.verse_end,
            translation_code: broadcast.passage.translation.code.clone(),
        })
    }
}

/// Newest first, one entry per label, at most [`MAX_SHOWN`], for one date.
#[derive(Debug, Default)]
pub(crate) struct ShownToday {
    date: Option<NaiveDate>,
    verses: Vec<ShownVerse>,
}

impl ShownToday {
    fn record(&mut self, today: NaiveDate, verse: ShownVerse) {
        if self.date != Some(today) {
            self.date = Some(today);
            self.verses.clear();
        }
        self.verses.retain(|held| held.label != verse.label);
        self.verses.insert(0, verse);
        self.verses.truncate(MAX_SHOWN);
    }

    fn read(&self, today: NaiveDate) -> Vec<ShownVerse> {
        if self.date == Some(today) {
            self.verses.clone()
        } else {
            Vec::new()
        }
    }
}

impl AppState {
    /// Called before the show is published: a client that reads the list on
    /// the event must find the verse in it.
    pub(super) async fn record_shown_verse(
        &self,
        broadcast: &BibleBroadcast,
        origin: &BibleShowOrigin,
    ) {
        if let Some(verse) = ShownVerse::of_show(broadcast, origin) {
            self.bible
                .shown_today
                .write()
                .await
                .record(local_today(), verse);
        }
    }

    pub(crate) async fn shown_verses_today(&self) -> Vec<ShownVerse> {
        self.bible.shown_today.read().await.read(local_today())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, n).expect("a date")
    }

    fn verse(n: u16) -> ShownVerse {
        ShownVerse {
            label: format!("Žalm 23:{n}"),
            book: "Žalm".to_string(),
            book_code: "PSA".to_string(),
            book_number: 19,
            chapter: 23,
            verse_start: n,
            verse_end: n,
            translation_code: "slk-seb".to_string(),
        }
    }

    fn labels(list: &[ShownVerse]) -> Vec<&str> {
        list.iter().map(|v| v.label.as_str()).collect()
    }

    fn broadcast(code: Option<&str>) -> BibleBroadcast {
        let mut reference = BibleReference::new("Žalm", 23, 1, 2).expect("a reference");
        reference.book_code = code.map(str::to_string);
        BibleBroadcast::new(
            BiblePassage::new(
                reference,
                BibleTranslation::new("slk-seb", "", ""),
                "text".to_string(),
            ),
            Utc::now(),
        )
    }

    #[test]
    fn newest_comes_first_and_a_repeat_moves_up_without_a_second_entry() {
        let mut list = ShownToday::default();
        list.record(day(8), verse(1));
        list.record(day(8), verse(2));
        list.record(day(8), verse(1));
        assert_eq!(labels(&list.read(day(8))), ["Žalm 23:1", "Žalm 23:2"]);
    }

    #[test]
    fn the_same_verse_in_another_translation_is_one_entry() {
        let mut list = ShownToday::default();
        list.record(day(8), verse(1));
        let mut english = verse(1);
        english.label = "Psalms 23:1".to_string();
        english.translation_code = "eng-kjv".to_string();
        list.record(day(8), english);
        assert_eq!(labels(&list.read(day(8))), ["Psalms 23:1"]);
    }

    #[test]
    fn the_oldest_goes_past_twenty() {
        let mut list = ShownToday::default();
        for n in 1..=21 {
            list.record(day(8), verse(n));
        }
        let held = list.read(day(8));
        assert_eq!(held.len(), 20);
        assert_eq!(held[0].label, "Žalm 23:21");
        assert_eq!(held[19].label, "Žalm 23:2");
    }

    #[test]
    fn another_date_reads_empty_and_a_new_show_starts_a_new_list() {
        let mut list = ShownToday::default();
        list.record(day(8), verse(1));
        assert_eq!(list.read(day(9)), Vec::new());
        list.record(day(9), verse(2));
        assert_eq!(labels(&list.read(day(9))), ["Žalm 23:2"]);
        assert_eq!(list.read(day(8)), Vec::new());
    }

    #[test]
    fn a_console_show_is_kept_with_its_code_and_translation() {
        let origin = BibleShowOrigin::internal(BibleShowSource::Console);
        let shown = ShownVerse::of_show(&broadcast(Some("PSA")), &origin).expect("a verse");
        assert_eq!(shown.label, "Žalm 23:1-2");
        assert_eq!(shown.book_code, "PSA");
        assert_eq!(shown.book_number, 19);
        assert_eq!(shown.translation_code, "slk-seb");
        assert_eq!((shown.verse_start, shown.verse_end), (1, 2));
    }

    #[test]
    fn a_sermon_slide_is_not_kept_a_plain_presentation_slide_is() {
        let sermon = BibleShowOrigin::internal(BibleShowSource::Sermon);
        assert_eq!(ShownVerse::of_show(&broadcast(Some("PSA")), &sermon), None);
        let plain = BibleShowOrigin::internal(BibleShowSource::Prepared);
        assert!(ShownVerse::of_show(&broadcast(Some("PSA")), &plain).is_some());
    }

    #[test]
    fn a_show_without_a_code_is_kept_by_its_book_name() {
        // Companion and the AI name only the book.
        let companion = BibleShowOrigin::internal(BibleShowSource::Companion);
        let shown = ShownVerse::of_show(&broadcast(None), &companion).expect("a verse");
        assert_eq!(shown.book_code, "PSA");
        assert_eq!(shown.book_number, 19);
    }

    #[test]
    fn a_book_nobody_knows_is_not_kept() {
        let origin = BibleShowOrigin::internal(BibleShowSource::Api);
        let mut unknown = broadcast(None);
        unknown.passage.reference.book = "Neznáma kniha".to_string();
        assert_eq!(ShownVerse::of_show(&unknown, &origin), None);
    }
}
