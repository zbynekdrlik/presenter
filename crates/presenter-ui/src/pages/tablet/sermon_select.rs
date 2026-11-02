//! The tablet and today's sermon (#851 H1, spec §8.3): sermons are Bible
//! presentations, so they play through the stored-slide trigger. On the day
//! of the service the tablet opens on the sermon, not on the first name.

use std::sync::atomic::{AtomicU32, Ordering};

use leptos::prelude::*;

use crate::api::bible::{get_presentation, BiblePresentationSummary, BibleSlideDto};
use crate::state::tablet::TabletContext;

/// Today's sermon if there is one in the list, else the first presentation.
pub(super) fn initial_presentation<'a>(
    presentations: &'a [BiblePresentationSummary],
    todays_sermons: &[String],
) -> Option<&'a BiblePresentationSummary> {
    presentations
        .iter()
        .find(|presentation| todays_sermons.contains(&presentation.id))
        .or_else(|| presentations.first())
}

/// What the tablet shows differs from the stored slides. A sermon's text
/// changes under the same ids and the same count.
pub(super) fn slides_differ(shown: &[BibleSlideDto], stored: &[BibleSlideDto]) -> bool {
    shown.len() != stored.len()
        || shown.iter().zip(stored).any(|(shown, stored)| {
            shown.id != stored.id
                || shown.bible_main != stored.bible_main
                || shown.bible_translation != stored.bible_translation
                || shown.bible_main_reference != stored.bible_main_reference
                || shown.bible_translation_reference != stored.bible_translation_reference
        })
}

/// The newest read of the open slides: the 10 s poll, a visibility change, a
/// live event and a tap overlap on slow Wi-Fi, and an older answer must not
/// redraw newer text. Every path that writes `ctx.slides` takes a ticket here.
static SLIDE_READ: AtomicU32 = AtomicU32::new(0);

pub(super) fn next_slide_read() -> u32 {
    SLIDE_READ.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

pub(super) fn is_newest_slide_read(ticket: u32) -> bool {
    SLIDE_READ.load(Ordering::Relaxed) == ticket
}

/// The live event's reload of the open presentation.
pub(super) async fn reload_open_slides(ctx: &TabletContext, id: &str) {
    let mine = next_slide_read();
    let Ok(detail) = get_presentation(id).await else {
        return;
    };
    if !is_newest_slide_read(mine) {
        return;
    }
    ctx.slides_cache
        .update(|cache| drop(cache.insert(id.to_string(), detail.slides.clone())));
    if ctx.current_presentation_id.get_untracked().as_deref() == Some(id) {
        ctx.slides.set(detail.slides);
    }
}

/// A sermon cut after the tablet read it keeps its ids and its count: the open
/// one is compared with what is stored, and redrawn only when it differs.
pub(super) async fn refresh_open_sermon(ctx: &TabletContext, fresh: &[BiblePresentationSummary]) {
    let Some(id) = ctx.current_presentation_id.get_untracked() else {
        return;
    };
    if !fresh
        .iter()
        .any(|pres| pres.id == id && pres.sermon_date.is_some())
    {
        return;
    }
    let mine = next_slide_read();
    let Ok(detail) = get_presentation(&id).await else {
        return;
    };
    if !is_newest_slide_read(mine) {
        return;
    }
    if ctx.current_presentation_id.get_untracked().as_deref() != Some(id.as_str()) {
        return;
    }
    if slides_differ(&ctx.slides.get_untracked(), &detail.slides) {
        ctx.slides_cache
            .update(|cache| drop(cache.insert(id.clone(), detail.slides.clone())));
        ctx.slides.set(detail.slides);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presentation(id: &str) -> BiblePresentationSummary {
        BiblePresentationSummary {
            id: id.to_string(),
            name: id.to_string(),
            slide_count: 1,
            sermon_date: None,
        }
    }

    fn slide(id: &str, main: &str) -> BibleSlideDto {
        BibleSlideDto {
            id: id.to_string(),
            order: 0,
            bible_main: main.to_string(),
            bible_translation: String::new(),
            metadata: None,
            bible_main_reference: String::new(),
            bible_translation_reference: String::new(),
        }
    }

    #[test]
    fn todays_sermon_wins_over_the_first_name() {
        let list = [presentation("a"), presentation("b")];
        let picked = initial_presentation(&list, &["b".to_string()]);
        assert_eq!(picked.map(|p| p.id.as_str()), Some("b"));
    }

    #[test]
    fn without_a_sermon_it_is_the_first_presentation() {
        let list = [presentation("a"), presentation("b")];
        assert_eq!(
            initial_presentation(&list, &[]).map(|p| p.id.as_str()),
            Some("a")
        );
        assert!(initial_presentation(&[], &["b".to_string()]).is_none());
    }

    #[test]
    fn a_cut_text_under_the_same_id_is_a_difference() {
        assert!(slides_differ(
            &[slide("a", "dlhý")],
            &[slide("a", "krátky")]
        ));
        assert!(!slides_differ(&[slide("a", "text")], &[slide("a", "text")]));
    }

    #[test]
    fn a_later_read_retires_an_earlier_one() {
        let first = next_slide_read();
        assert!(is_newest_slide_read(first));
        let second = next_slide_read();
        assert!(!is_newest_slide_read(first));
        assert!(is_newest_slide_read(second));
    }

    #[test]
    fn a_changed_reference_alone_is_a_difference() {
        let mut changed = slide("a", "text");
        changed.bible_translation_reference = "Heb 11:1 (ROH)".to_string();
        assert!(slides_differ(&[slide("a", "text")], &[changed]));
    }
}
