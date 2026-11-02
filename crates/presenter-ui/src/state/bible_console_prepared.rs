//! "Pripravené kazateľom" (#851 H1, spec §8.1): today's sermons as the verse
//! console shows them. A prepared item plays its STORED slides: the text the
//! preacher cut, never a fresh `/bible/resolve` of the whole verse. No
//! `web_sys` here — host-tested with `cargo test --lib`.

use crate::api::bible::BibleSlideDto;
use crate::api::sermons::SermonDetail;

/// A row's text is cut here (spec §8.1).
const DETAIL_CHARS: usize = 80;

/// One item of a sermon: a tap previews its first slide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRow {
    /// The item's id; the keyed `<For>` adds the sermon's revision.
    pub key: String,
    pub label: String,
    pub detail: String,
    /// Index of the item's first slide in `PreparedSermon::slides`.
    pub first_slide: usize,
}

/// A sermon of today: its rows and all its slides in the prepared order.
#[derive(Debug, Clone)]
pub struct PreparedSermon {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub rows: Vec<PreparedRow>,
    pub slides: Vec<BibleSlideDto>,
}

impl PreparedSermon {
    /// The console's preview key: a new revision is a new preview.
    pub fn preview_key(&self) -> String {
        format!("prepared:{}:{}", self.id, self.revision)
    }
}

/// What the console has chosen: `index` into the sermon's slides.
#[derive(Debug, Clone)]
pub struct PreparedPick {
    pub sermon: PreparedSermon,
    pub index: usize,
}

impl PreparedPick {
    pub fn key(&self) -> String {
        self.sermon.preview_key()
    }

    pub fn slide_id(&self) -> Option<&str> {
        self.sermon.slides.get(self.index).map(|s| s.id.as_str())
    }

    /// The same place in today's fresh list: the same slide, or the place it
    /// held when the preacher removed it; `None` when the sermon is gone or
    /// has no slides left.
    pub fn refreshed(&self, sermons: &[PreparedSermon]) -> Option<Self> {
        let sermon = sermons.iter().find(|s| s.id == self.sermon.id)?.clone();
        let last = sermon.slides.len().checked_sub(1)?;
        let held = self.slide_id();
        let index = sermon
            .slides
            .iter()
            .position(|slide| Some(slide.id.as_str()) == held)
            .unwrap_or_else(|| self.index.min(last));
        Some(Self { sermon, index })
    }
}

fn slide_dto(slide: &crate::api::sermons::SermonSlide, order: usize) -> BibleSlideDto {
    BibleSlideDto {
        id: slide.id.clone(),
        order: u32::try_from(order).unwrap_or(u32::MAX),
        bible_main: slide.main_text.clone(),
        bible_translation: slide.secondary_text.clone(),
        metadata: None,
        bible_main_reference: slide.main_reference.clone(),
        bible_translation_reference: slide.secondary_reference.clone(),
    }
}

/// The items in their order, each with its slides; an item without a slide
/// has nothing to play and no row.
pub fn prepared_sermon(detail: &SermonDetail) -> PreparedSermon {
    let mut slides = Vec::new();
    let mut rows = Vec::new();
    for item in &detail.items {
        let first_slide = slides.len();
        for id in &item.slide_ids {
            if let Some(slide) = detail.slides.iter().find(|slide| slide.id == *id) {
                slides.push(slide_dto(slide, slides.len()));
            }
        }
        if slides.len() > first_slide {
            rows.push(PreparedRow {
                key: item.id.clone(),
                label: item.label.clone(),
                detail: item.rendered_text.chars().take(DETAIL_CHARS).collect(),
                first_slide,
            });
        }
    }
    PreparedSermon {
        id: detail.id.clone(),
        title: detail.title.clone(),
        revision: detail.revision,
        rows,
        slides,
    }
}

/// Today's sermons that have something to show; the section hides without any.
pub fn prepared_sermons(details: &[SermonDetail]) -> Vec<PreparedSermon> {
    details
        .iter()
        .map(prepared_sermon)
        .filter(|sermon| !sermon.rows.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::sermons::{SermonItem, SermonSlide};
    use presenter_core::bible::sermon::ItemEdits;

    fn item(id: &str, slides: &[&str]) -> SermonItem {
        SermonItem {
            id: id.to_string(),
            translation: "slk-seb".to_string(),
            book_code: "HEB".to_string(),
            book: "Hebrejom".to_string(),
            chapter: 11,
            verse_start: 1,
            verse_end: 1,
            source: Vec::new(),
            edits: ItemEdits::default(),
            source_changed: false,
            slide_ids: slides.iter().map(ToString::to_string).collect(),
            label: format!("Hebrejom {id}"),
            rendered_text: "ž".repeat(100),
        }
    }

    fn slide(id: &str) -> SermonSlide {
        SermonSlide {
            id: id.to_string(),
            main_text: format!("text {id}"),
            main_reference: format!("ref {id}"),
            secondary_text: "second".to_string(),
            secondary_reference: "second ref".to_string(),
        }
    }

    fn detail(id: &str, revision: i64, items: Vec<SermonItem>, slides: &[&str]) -> SermonDetail {
        SermonDetail {
            id: id.to_string(),
            title: format!("Kázeň {id}"),
            service_date: chrono::NaiveDate::default(),
            default_translation: "slk-seb".to_string(),
            revision,
            items,
            slides: slides.iter().map(|id| slide(id)).collect(),
            secondary_missing: false,
        }
    }

    fn ids(sermon: &PreparedSermon) -> Vec<&str> {
        sermon.slides.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn slides_follow_the_items_not_the_stored_list() {
        let sermon = prepared_sermon(&detail(
            "a",
            1,
            vec![item("two", &["s2", "s3"]), item("one", &["s1"])],
            &["s1", "s2", "s3"],
        ));
        assert_eq!(ids(&sermon), ["s2", "s3", "s1"]);
        assert_eq!(sermon.rows[0].first_slide, 0);
        assert_eq!(sermon.rows[1].first_slide, 2);
        assert_eq!(sermon.slides[0].bible_main, "text s2");
        assert_eq!(sermon.slides[0].bible_main_reference, "ref s2");
        assert_eq!(sermon.slides[0].bible_translation, "second");
    }

    #[test]
    fn the_detail_is_cut_by_characters() {
        let sermon = prepared_sermon(&detail("a", 1, vec![item("one", &["s1"])], &["s1"]));
        assert_eq!(sermon.rows[0].detail.chars().count(), 80);
    }

    #[test]
    fn a_sermon_without_slides_is_not_listed() {
        let empty = detail("a", 1, vec![item("one", &[])], &[]);
        let full = detail("b", 1, vec![item("one", &["s1"])], &["s1"]);
        let listed = prepared_sermons(&[empty, full]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "b");
    }

    fn pick(index: usize) -> PreparedPick {
        PreparedPick {
            sermon: prepared_sermon(&detail(
                "a",
                1,
                vec![item("one", &["s1", "s2", "s3"])],
                &["s1", "s2", "s3"],
            )),
            index,
        }
    }

    #[test]
    fn a_refresh_follows_the_slide_to_its_new_place() {
        let fresh = prepared_sermons(&[detail(
            "a",
            2,
            vec![item("one", &["s3", "s2", "s1"])],
            &["s1", "s2", "s3"],
        )]);
        let held = pick(0).refreshed(&fresh).expect("still there");
        assert_eq!(held.index, 2);
        assert_ne!(held.key(), pick(0).key());
    }

    #[test]
    fn a_refresh_keeps_the_place_of_a_removed_slide() {
        let fresh = prepared_sermons(&[detail("a", 2, vec![item("one", &["s1"])], &["s1"])]);
        assert_eq!(pick(2).refreshed(&fresh).expect("still there").index, 0);
    }

    #[test]
    fn a_refresh_lets_go_of_a_deleted_sermon() {
        assert!(pick(0).refreshed(&[]).is_none());
    }
}
