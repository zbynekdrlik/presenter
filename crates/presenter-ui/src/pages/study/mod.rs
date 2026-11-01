//! `/ui/study` (#851 spec §6): the Bible on the preacher's phone and tablet,
//! reading and searching only. Nothing on this page can reach a wall (T11): it
//! never calls a trigger, clear or stage endpoint.
//!
//! The URL is the state (`state::study::StudyRoute`): every view and sheet has
//! its own, so the system Back gesture works. `nav` owns the moves, the
//! components only read `Study`.

mod favourites;
mod field;
mod header;
mod history;
mod long_press;
mod nav;
mod reader;
mod scroll;
mod sheet;
mod text_size;
mod verse_actions;

use std::collections::HashMap;

use gloo_timers::callback::Timeout;
use leptos::html::Main;
use leptos::prelude::*;
use presenter_core::bible::resolver::Versification;
use presenter_core::BibleTranslation;

use crate::api::bible::{self as bible_api, BibleSearchHit};
use crate::api::study::{load_chapter, ChapterCache, ChapterText};
use crate::state::bible_console::versification;
use crate::state::session;
use crate::state::study::{
    chapter_key, failure_text, parse_favourites, visible_favourites, StudyRoute, StudySheet,
    StudyView, TextSize, KEY_FAVOURITES, KEY_HISTORY, KEY_TEXT_SIZE, START_BOOK, START_CHAPTER,
};
use crate::state::study_history::StudyHistory;

use favourites::{FavouriteSheet, Favourites};
use field::StudyField;
use header::StudyHeader;
use history::HistorySheet;
use reader::{ChapterBar, Reader};
use scroll::ScrollPlan;
use text_size::TextSizeSheet;
use verse_actions::VerseActionsSheet;

/// `/bible/search` answers at most this many verses.
const SEARCH_LIMIT: u32 = 100;
/// How long "Skopírované" stays.
const NOTICE_MS: u32 = 2_500;

/// A chapter of a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChapterPlace {
    translation: String,
    book: String,
    chapter: u16,
}

impl ChapterPlace {
    fn key(&self) -> String {
        chapter_key(&self.translation, &self.book, self.chapter)
    }
}

/// What the reader shows. A chapter on its way leaves the previous one on
/// screen (`Study::pending` marks the wait), so a swipe never flashes empty.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chapter {
    None,
    Ready(ChapterPlace, ChapterText),
    /// This translation does not have the chapter.
    Missing(ChapterPlace),
    Failed(ChapterPlace, String),
}

#[derive(Clone, Default)]
enum Search {
    #[default]
    Idle,
    Ready {
        key: String,
        hits: Vec<BibleSearchHit>,
    },
    Failed {
        key: String,
        message: String,
    },
}

/// The page's state. `Copy` (signals only), passed to every component.
#[derive(Clone, Copy)]
struct Study {
    route: RwSignal<StudyRoute>,
    /// `/bible/translations`; empty until it answered.
    translations: RwSignal<Vec<BibleTranslation>>,
    translations_failed: RwSignal<Option<String>>,
    /// `/bible/books` per translation: the reference resolver and the chapter
    /// steps of the reader's translation.
    structures: RwSignal<HashMap<String, Versification>>,
    chapter: RwSignal<Chapter>,
    pending: RwSignal<Option<String>>,
    search: RwSignal<Search>,
    favourites: RwSignal<Vec<String>>,
    text_size: RwSignal<TextSize>,
    history: RwSignal<StudyHistory>,
    notice: RwSignal<Option<String>>,
    /// Bumped by a retry button: asks again for the same chapter or search.
    retry: RwSignal<u32>,
    cache: StoredValue<ChapterCache>,
    scroll_plan: StoredValue<ScrollPlan>,
    /// Scroll offsets by browser history index, for Back.
    offsets: StoredValue<HashMap<usize, i32>>,
    scroller: NodeRef<Main>,
}

impl Study {
    /// The route's translation, once it is known to be installed.
    fn translation(self) -> Option<String> {
        let code = self.route.with(|route| route.translation.clone())?;
        self.translations
            .with(|list| list.iter().any(|t| t.code == code))
            .then_some(code)
    }

    fn installed_codes(self) -> Vec<String> {
        self.translations
            .with(|list| list.iter().map(|t| t.code.clone()).collect())
    }

    fn with_structure<T>(self, read: impl FnOnce(Option<&Versification>) -> T) -> Option<T> {
        let code = self.translation()?;
        Some(self.structures.with(|map| read(map.get(&code))))
    }

    fn place(self) -> Option<ChapterPlace> {
        let translation = self.translation()?;
        self.route.with(|route| match &route.view {
            StudyView::Reader { book, chapter } => Some(ChapterPlace {
                translation,
                book: book.clone(),
                chapter: *chapter,
            }),
            StudyView::Search { .. } => None,
        })
    }

    fn search_key(self) -> Option<(String, String)> {
        let translation = self.translation()?;
        self.route.with(|route| match &route.view {
            StudyView::Search { query } => Some((translation, query.clone())),
            StudyView::Reader { .. } => None,
        })
    }
}

fn study_error(err: &crate::api::ApiError) -> String {
    use crate::api::ApiError;
    match err {
        ApiError::Status(_, detail) => failure_text(detail),
        _ => failure_text(""),
    }
}

fn fetch_chapter(s: Study, place: ChapterPlace) {
    let key = place.key();
    if s.pending.with_untracked(|p| p.as_deref() == Some(&key)) {
        return;
    }
    let cached = s.cache.try_update_value(|cache| cache.get(&key)).flatten();
    if let Some(text) = cached {
        s.pending.set(None);
        show_chapter(s, Chapter::Ready(place, text));
        return;
    }
    s.pending.set(Some(key.clone()));
    leptos::task::spawn_local(async move {
        let answer = load_chapter(&place.translation, &place.book, place.chapter).await;
        // Only the chapter still wanted may land (slow Wi-Fi, a quick swipe).
        if s.pending.with_untracked(|p| p.as_deref() != Some(&key)) {
            return;
        }
        s.pending.set(None);
        let chapter = match answer {
            Ok(text) if text.verses.is_empty() => Chapter::Missing(place),
            Ok(text) => {
                s.cache.update_value(|cache| cache.put(key, text.clone()));
                Chapter::Ready(place, text)
            }
            Err(err) => Chapter::Failed(place, study_error(&err)),
        };
        show_chapter(s, chapter);
    });
}

fn show_chapter(s: Study, chapter: Chapter) {
    let book = match &chapter {
        Chapter::Ready(_, text) => Some(text.book.clone()),
        _ => None,
    };
    s.chapter.set(chapter);
    if let Some(book) = book {
        nav::label_current(s, &book);
    }
    scroll::settle(s);
}

fn fetch_search(s: Study, translation: String, query: String) {
    let key = format!("{translation}:{query}");
    leptos::task::spawn_local(async move {
        let answer = bible_api::search(&query, &translation, Some(SEARCH_LIMIT)).await;
        if s.search_key() != Some((translation, query)) {
            return;
        }
        s.search.set(match answer {
            Ok(hits) => Search::Ready { key, hits },
            Err(err) => Search::Failed {
                key,
                message: study_error(&err),
            },
        });
    });
}

fn ensure_structure(s: Study, code: String) {
    if s.structures.with_untracked(|map| map.contains_key(&code)) {
        return;
    }
    leptos::task::spawn_local(async move {
        if let Ok(books) = bible_api::list_books(&code).await {
            let structure = versification(&books);
            s.structures.update(|map| {
                map.insert(code, structure);
            });
        }
    });
}

fn fetch_translations(s: Study) {
    s.translations_failed.set(None);
    leptos::task::spawn_local(async move {
        match bible_api::list_translations().await {
            Ok(list) if list.is_empty() => {
                s.translations_failed
                    .set(Some("Nie je nainštalovaný žiaden preklad.".to_string()));
            }
            Ok(list) => {
                s.translations.set(list);
                settle_translation(s);
            }
            Err(err) => s.translations_failed.set(Some(study_error(&err))),
        }
    });
}

/// A URL without a usable translation opens in the first favourite.
fn settle_translation(s: Study) {
    if s.translation().is_some() {
        return;
    }
    let chips = visible_favourites(&s.favourites.get_untracked(), &s.installed_codes());
    let first = chips
        .into_iter()
        .next()
        .map(|(_, code)| code)
        .or_else(|| s.installed_codes().into_iter().next());
    if let Some(code) = first {
        let mut route = s.route.get_untracked();
        route.translation = Some(code);
        nav::replace(s, route);
    }
}

/// The route of this visit: the URL, else the last position (and the verse it
/// was scrolled to), else the start.
fn opening_route() -> (StudyRoute, Option<u16>) {
    let search = crate::utils::window::window()
        .location()
        .search()
        .unwrap_or_default();
    if let Some(route) = StudyRoute::parse(&search) {
        return (route, None);
    }
    let last = session::get_persistent(crate::state::study::KEY_LAST)
        .and_then(|url| StudyRoute::parse(url.split_once('?')?.1));
    match last {
        Some(route) => {
            let verse = scroll::saved_verse(&route);
            (route, verse)
        }
        None => (
            StudyRoute {
                translation: None,
                view: StudyView::Reader {
                    book: START_BOOK.to_string(),
                    chapter: START_CHAPTER,
                },
                verses: None,
                sheet: None,
            },
            None,
        ),
    }
}

/// Install-as-an-app tags, as the tablet does (`tablet.rs`). The viewport stays
/// zoomable: this is a page to read.
fn install_pwa() {
    let document = crate::utils::window::document();
    document.set_title("Štúdium · Presenter");
    let Some(head) = document.head() else {
        return;
    };
    let tags: [(&str, &[(&str, &str)]); 5] = [
        (
            "link",
            &[("rel", "manifest"), ("href", "/ui/study/manifest.json")],
        ),
        (
            "link",
            &[
                ("rel", "apple-touch-icon"),
                ("href", "/ui/tablet/apple-touch-icon.png"),
            ],
        ),
        (
            "meta",
            &[("name", "apple-mobile-web-app-capable"), ("content", "yes")],
        ),
        (
            "meta",
            &[
                ("name", "apple-mobile-web-app-title"),
                ("content", "Štúdium"),
            ],
        ),
        ("meta", &[("name", "theme-color"), ("content", "#0f172a")]),
    ];
    for (tag, attrs) in tags {
        if let Ok(element) = document.create_element(tag) {
            for (name, value) in attrs {
                let _ = element.set_attribute(name, value);
            }
            let _ = head.append_child(&element);
        }
    }
    let _ = js_sys::eval(
        "if ('serviceWorker' in navigator) { navigator.serviceWorker.register('/ui/study/sw.js').catch(function () {}); }",
    );
}

#[component]
pub fn StudyPage() -> impl IntoView {
    crate::state::phone_shell::install_phone_shell(RwSignal::new(false));
    crate::state::keyboard_inset::install_keyboard_inset();
    install_pwa();

    let (route, saved_verse) = opening_route();
    let s = Study {
        route: RwSignal::new(route),
        translations: RwSignal::new(Vec::new()),
        translations_failed: RwSignal::new(None),
        structures: RwSignal::new(HashMap::new()),
        chapter: RwSignal::new(Chapter::None),
        pending: RwSignal::new(None),
        search: RwSignal::new(Search::Idle),
        favourites: RwSignal::new(parse_favourites(
            session::get_persistent(KEY_FAVOURITES).as_deref(),
        )),
        text_size: RwSignal::new(TextSize::parse(
            session::get_persistent(KEY_TEXT_SIZE).as_deref(),
        )),
        history: RwSignal::new(StudyHistory::parse(session::get(KEY_HISTORY).as_deref())),
        notice: RwSignal::new(None),
        retry: RwSignal::new(0),
        cache: StoredValue::new(ChapterCache::default()),
        scroll_plan: StoredValue::new(saved_verse.map_or(ScrollPlan::Keep, ScrollPlan::Verse)),
        offsets: StoredValue::new(HashMap::new()),
        scroller: NodeRef::new(),
    };
    if s.route.with_untracked(|route| route.verses).is_some() {
        scroll::plan_for_route(s);
    }
    nav::install(s);
    fetch_translations(s);

    let place = Memo::new(move |_| s.place());
    Effect::new(move |_| {
        s.retry.track();
        if let Some(place) = place.get() {
            let shown = s.chapter.with_untracked(|shown| match shown {
                Chapter::Ready(held, _) => *held == place,
                _ => false,
            });
            if shown {
                s.pending.set(None);
            } else {
                fetch_chapter(s, place);
            }
        }
    });
    let translation = Memo::new(move |_| s.translation());
    Effect::new(move |_| {
        s.retry.track();
        if let Some(code) = translation.get() {
            ensure_structure(s, code);
        }
    });
    let search_key = Memo::new(move |_| s.search_key());
    Effect::new(move |_| {
        s.retry.track();
        if let Some((translation, query)) = search_key.get() {
            s.search.set(Search::Idle);
            fetch_search(s, translation, query);
        }
    });
    Effect::new(move |previous: Option<Option<Timeout>>| {
        drop(previous);
        s.notice
            .with(Option::is_some)
            .then(|| Timeout::new(NOTICE_MS, move || s.notice.set(None)))
    });

    let open_sheet = Memo::new(move |_| s.route.with(|route| route.sheet));
    let reading = Memo::new(move |_| {
        s.route
            .with(|route| matches!(route.view, StudyView::Reader { .. }))
    });
    let sheets = move || match open_sheet.get() {
        Some(StudySheet::Actions) => view! { <VerseActionsSheet s=s /> }.into_any(),
        Some(StudySheet::History) => view! { <HistorySheet s=s /> }.into_any(),
        Some(StudySheet::TextSize) => view! { <TextSizeSheet s=s /> }.into_any(),
        Some(StudySheet::Favourite(slot)) => view! { <FavouriteSheet s=s index=slot /> }.into_any(),
        None => ().into_any(),
    };

    view! {
        <div
            class="study"
            data-role="study"
            data-text-size=move || s.text_size.get().key()
        >
            <StudyHeader s=s />
            <StudyField s=s />
            <Favourites s=s />
            {move || {
                s.translations_failed
                    .get()
                    .map(|message| {
                        view! {
                            <div class="study-error" data-role="study-page-error" role="alert">
                                <p>{message}</p>
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="study-retry"
                                    on:click=move |_| fetch_translations(s)
                                >
                                    "Skúsiť znova"
                                </button>
                            </div>
                        }
                    })
            }}
            <Show when=move || reading.get()>
                <Reader s=s />
                <ChapterBar s=s />
            </Show>
            {sheets}
            <div
                class="study-notice"
                data-role="study-notice"
                data-visible=move || if s.notice.with(Option::is_some) { "true" } else { "false" }
                role="status"
            >
                {move || s.notice.get().unwrap_or_default()}
            </div>
        </div>
    }
}
