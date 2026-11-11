//! The pure half of the study page's named bookmarks (#915): what a row says,
//! the name a new bookmark is offered, and where "Presunúť sem" moves one. No
//! `web_sys` here, host-tested with `cargo test --lib`.

use presenter_core::bible::sermon::translation_short_code;

use crate::state::study::{StudyRoute, StudyView};

/// The name a new bookmark is offered: the place, `Hebrejom 11:4`.
pub fn offered_name(book: &str, chapter: u16, verse: u16) -> String {
    format!("{book} {chapter}:{verse}")
}

/// A row's place under its name: `Hebrejom 11:4 (SEB)`.
pub fn place_label(book: &str, chapter: u16, verse: u16, translation: &str) -> String {
    format!(
        "{} ({})",
        offered_name(book, chapter, verse),
        translation_short_code(translation)
    )
}

/// The verse being read, as a bookmark keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingPlace {
    pub translation: String,
    pub book: String,
    pub chapter: u16,
    pub verse: u16,
}

/// Where "Presunúť sem" moves a bookmark: in the reader the first selected
/// verse, else the verse at the top of the screen, else the chapter's first; in
/// compare its verse. Elsewhere (a search, a sermon) there is no place, and the
/// button is not shown.
pub fn reading_place(
    route: &StudyRoute,
    translation: Option<&str>,
    in_view: Option<u16>,
) -> Option<ReadingPlace> {
    let translation = translation?.to_string();
    let (book, chapter, verse) = match &route.view {
        StudyView::Reader { book, chapter } => {
            let verse = route
                .verses
                .map(|(first, _)| first)
                .or(in_view)
                .unwrap_or(1);
            (book, *chapter, verse)
        }
        StudyView::Compare {
            book,
            chapter,
            verse,
            ..
        } => (book, *chapter, *verse),
        _ => return None,
    };
    Some(ReadingPlace {
        translation,
        book: book.clone(),
        chapter,
        verse,
    })
}

/// Where a bookmark opens: the reader at its verse, highlighted.
pub fn open_route(translation: &str, book: &str, chapter: u16, verse: u16) -> StudyRoute {
    StudyRoute::reader(translation, book, chapter).with_verses(Some((verse, verse)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::study::StudySheet;

    #[test]
    fn a_row_names_the_place_and_the_translation() {
        assert_eq!(offered_name("Hebrejom", 11, 4), "Hebrejom 11:4");
        assert_eq!(
            place_label("Hebrejom", 11, 4, "slk-seb"),
            "Hebrejom 11:4 (SEB)"
        );
        assert_eq!(place_label("John", 3, 16, "eng-kjv"), "John 3:16 (KJV)");
    }

    #[test]
    fn the_reader_moves_a_bookmark_to_the_selection_then_the_view_then_verse_one() {
        let reader = StudyRoute::reader("slk-seb", "HEB", 11);
        let selected = reader.clone().with_verses(Some((4, 6)));
        let place = |route: &StudyRoute, in_view| {
            reading_place(route, Some("slk-seb"), in_view).map(|p| (p.book, p.chapter, p.verse))
        };
        assert_eq!(place(&selected, Some(9)), Some(("HEB".to_string(), 11, 4)));
        assert_eq!(place(&reader, Some(9)), Some(("HEB".to_string(), 11, 9)));
        assert_eq!(place(&reader, None), Some(("HEB".to_string(), 11, 1)));
        // The sheet over the reader does not change the place.
        let over = reader.with_sheet(Some(StudySheet::Bookmarks));
        assert_eq!(place(&over, Some(2)), Some(("HEB".to_string(), 11, 2)));
    }

    #[test]
    fn compare_moves_to_its_verse_and_other_views_have_no_place() {
        let compare = StudyRoute::compare(Some("slk-seb".to_string()), ("ROM", 8, 28), Vec::new());
        let found = reading_place(&compare, Some("slk-seb"), Some(3)).expect("place");
        assert_eq!(
            found,
            ReadingPlace {
                translation: "slk-seb".to_string(),
                book: "ROM".to_string(),
                chapter: 8,
                verse: 28,
            }
        );
        let search = StudyRoute::search("slk-seb", "viera");
        assert_eq!(reading_place(&search, Some("slk-seb"), Some(1)), None);
        assert_eq!(
            reading_place(&StudyRoute::sermons(None), Some("slk-seb"), None),
            None
        );
        // No installed translation yet: no place.
        assert_eq!(reading_place(&compare, None, None), None);
    }

    #[test]
    fn a_bookmark_opens_the_reader_with_its_verse_selected() {
        let route = open_route("slk-seb", "HEB", 11, 4);
        assert_eq!(route.to_url(), "/ui/study?t=slk-seb&p=HEB.11&v=4");
        assert_eq!(route.sheet, None);
    }

    #[test]
    fn the_bookmark_list_has_a_url_and_the_old_save_sheet_none() {
        let list = StudyRoute::reader("slk-seb", "HEB", 11).with_sheet(Some(StudySheet::Bookmarks));
        assert_eq!(list.to_url(), "/ui/study?t=slk-seb&p=HEB.11&s=zalozky");
        assert_eq!(
            StudyRoute::parse("?t=slk-seb&p=HEB.11&s=zalozky"),
            Some(list)
        );
        // #932: "Pridať záložku" adds at once; an old `s=zalozka` link opens the
        // verse with no sheet.
        let old_save = StudyRoute::parse("?t=slk-seb&p=HEB.11&v=4&s=zalozka").expect("route");
        assert_eq!(old_save.sheet, None);
        assert_eq!(old_save.verses, Some((4, 4)));
        // The list opens over any view, the sermon list too.
        let sermons = StudyRoute::parse("?view=sermons&s=zalozky").expect("route");
        assert_eq!(sermons.sheet, Some(StudySheet::Bookmarks));
    }
}
