//! `/ui/study` (#851 spec §6, §7.6): the Bible on the preacher's phone and
//! tablet, reading, searching and preparing sermons. Nothing on this page can
//! reach a wall (T11): it never calls a trigger, clear or stage endpoint.
//!
//! The URL is the state (`state::study::StudyRoute`): every view and sheet has
//! its own, so the system Back gesture works. `nav` owns the moves, the
//! components only read `Study`.

mod add_to_sermon;
mod favourites;
mod field;
mod header;
mod history;
mod long_press;
mod nav;
mod reader;
mod results;
mod scroll;
mod sermon_editor;
mod sermon_item;
mod sermons;
mod sheet;
mod text_size;
mod verse_actions;

use std::collections::HashMap;

use gloo_timers::callback::Timeout;
use leptos::html::Main;
use leptos::prelude::*;
use presenter_core::bible::resolver::Versification;
use presenter_core::bible::sermon::ItemEdits;
use presenter_core::BibleTranslation;

use crate::api::bible::{self as bible_api, BibleSearchHit};
use crate::api::sermons::{SermonItem, SermonPassage};
use crate::api::study::{load_chapter, ChapterCache, ChapterText};
use crate::state::bible_console::versification;
use crate::state::session;
use crate::state::study::{
    chapter_key, failure_text, parse_favourites, visible_favourites, StudyRoute, StudySheet,
    StudyView, TextSize, KEY_ACTIVE_SERMON, KEY_FAVOURITES, KEY_HISTORY, KEY_TEXT_SIZE, START_BOOK,
    START_CHAPTER,
};
use crate::state::study_history::StudyHistory;

use add_to_sermon::AddToSermonSheet;
use favourites::{FavouriteSheet, Favourites};
use field::StudyField;
use header::StudyHeader;
use history::HistorySheet;
use reader::{ChapterBar, Reader};
use results::SearchResults;
use scroll::ScrollPlan;
use sermon_editor::{Editor, SermonEditor};
use sermons::SermonListView;
use text_size::TextSizeSheet;
use verse_actions::VerseActionsSheet;

/// `/bible/search` answers at most this many verses.
const SEARCH_LIMIT: u32 = 100;
/// How long "Skopírované" stays.
const NOTICE_MS: u32 = 2_500;
/// How long a toast with Späť stays.
const UNDO_MS: u32 = 5_000;
/// The NLT is fetched on demand: it has no text to snapshot into a sermon.
pub(crate) const REMOTE_TRANSLATION: &str = "eng-nlt";

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

/// What Späť on a toast takes back.
#[derive(Clone)]
enum Undo {
    Added {
        sermon: String,
        item: String,
    },
    Removed {
        sermon: String,
        item: Box<SermonItem>,
        position: usize,
        edits: ItemEdits,
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
    /// The translation whose `/bible/books` failed; the page says so and retries.
    structures_failed: RwSignal<Option<String>>,
    chapter: RwSignal<Chapter>,
    pending: RwSignal<Option<String>>,
    search: RwSignal<Search>,
    favourites: RwSignal<Vec<String>>,
    text_size: RwSignal<TextSize>,
    history: RwSignal<StudyHistory>,
    notice: RwSignal<Option<String>>,
    undo: RwSignal<Option<Undo>>,
    /// A message waiting for the toast with Späť to end.
    later: StoredValue<Option<String>>,
    /// The sermon "Pridať do kázne" goes to (`study.activeSermon`).
    active_sermon: RwSignal<Option<String>>,
    /// The passage the add sheet is asking about.
    pending_passage: RwSignal<Option<SermonPassage>>,
    /// The passage being added: a double tap adds it once.
    adding: StoredValue<Option<SermonPassage>>,
    /// One editor per opened sermon, owned by the page: its save queue outlives
    /// the view, so leaving a sermon never drops an edit.
    editors: StoredValue<HashMap<String, Editor>, LocalStorage>,
    owner: StoredValue<Owner, LocalStorage>,
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
            _ => None,
        })
    }

    /// `slk-seb:viera`, the settle marker of the search on the route.
    fn search_state_key(self) -> Option<String> {
        let (translation, query) = self.search_key()?;
        Some(format!("{translation}:{query}"))
    }

    fn search_key(self) -> Option<(String, String)> {
        let translation = self.translation()?;
        self.route.with(|route| match &route.view {
            StudyView::Search { query } => Some((translation, query.clone())),
            _ => None,
        })
    }

    /// A Späť on screen stays with its own sentence; this message follows it.
    fn tell(self, message: String) {
        if self.undo.with_untracked(Option::is_some) {
            self.later.set_value(Some(message));
        } else {
            self.notice.set(Some(message));
        }
    }

    /// A toast with Späť.
    fn tell_undo(self, message: String, undo: Undo) {
        self.undo.set(Some(undo));
        self.notice.set(Some(message));
    }
}

/// A refused sermon request reads as the server's own Slovak sentence.
fn sermon_error(err: &crate::api::ApiError) -> String {
    match err {
        crate::api::ApiError::Refused(_, message) => message.clone(),
        _ => failure_text(""),
    }
}

/// A translation a sermon can snapshot: every one but the on-demand NLT.
fn snapshot_translation(code: &str) -> Option<&str> {
    (code != REMOTE_TRANSLATION).then_some(code)
}

fn end_undo_toast(s: Study) {
    s.undo.set(None);
    s.notice
        .set(s.later.try_update_value(Option::take).flatten());
}

fn run_undo(s: Study) {
    let Some(undo) = s.undo.get_untracked() else {
        return;
    };
    end_undo_toast(s);
    match undo {
        Undo::Added { sermon, item } => {
            sermon_editor::undo_added(s, sermon, item);
        }
        Undo::Removed {
            sermon,
            item,
            position,
            edits,
        } => sermon_editor::undo_removed(s, sermon, *item, position, edits),
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
        scroll::settle(s);
    });
}

fn ensure_structure(s: Study, code: String) {
    if s.structures.with_untracked(|map| map.contains_key(&code)) {
        return;
    }
    s.structures_failed.set(None);
    leptos::task::spawn_local(async move {
        match bible_api::list_books(&code).await {
            Ok(books) => {
                let structure = versification(&books);
                s.structures.update(|map| {
                    map.insert(code, structure);
                });
            }
            Err(_) => s.structures_failed.set(Some(code)),
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
        // A link names its verse; a reload stays where the reader was.
        let keeps_place = route.verses.is_none() || nav::has_entry_state();
        let verse = keeps_place.then(|| scroll::saved_verse(&route)).flatten();
        return (route, verse);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewKind {
    Reader,
    Search,
    Sermons,
    Sermon(String),
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
        structures_failed: RwSignal::new(None),
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
        undo: RwSignal::new(None),
        later: StoredValue::new(None),
        active_sermon: RwSignal::new(session::get_persistent(KEY_ACTIVE_SERMON)),
        pending_passage: RwSignal::new(None),
        adding: StoredValue::new(None),
        editors: StoredValue::new_local(HashMap::new()),
        owner: StoredValue::new_local(Owner::current().unwrap_or_default()),
        retry: RwSignal::new(0),
        cache: StoredValue::new(ChapterCache::default()),
        scroll_plan: StoredValue::new(saved_verse.map_or(ScrollPlan::Keep, ScrollPlan::Verse)),
        offsets: StoredValue::new(HashMap::new()),
        scroller: NodeRef::new(),
    };
    if saved_verse.is_none() && s.route.with_untracked(|route| route.verses).is_some() {
        scroll::plan_for_route(s);
    }
    nav::install(s);
    sermon_editor::install(s);
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
            // Back from a hit: the list is still here, scroll and all.
            let key = format!("{translation}:{query}");
            let held = s.search.with_untracked(|search| match search {
                Search::Ready { key: shown, .. } => *shown == key,
                _ => false,
            });
            if !held {
                s.search.set(Search::Idle);
                fetch_search(s, translation, query);
            }
        }
    });
    Effect::new(move |previous: Option<Option<Timeout>>| {
        drop(previous);
        let stay = if s.undo.with(Option::is_some) {
            UNDO_MS
        } else {
            NOTICE_MS
        };
        s.notice
            .with(Option::is_some)
            .then(|| Timeout::new(stay, move || end_undo_toast(s)))
    });

    let open_sheet = Memo::new(move |_| s.route.with(|route| route.sheet));
    let view_kind = Memo::new(move |_| {
        s.route.with(|route| match &route.view {
            StudyView::Reader { .. } => ViewKind::Reader,
            StudyView::Search { .. } => ViewKind::Search,
            StudyView::Sermons => ViewKind::Sermons,
            StudyView::Sermon { id } => ViewKind::Sermon(id.clone()),
        })
    });
    let bible_view = move || matches!(view_kind.get(), ViewKind::Reader | ViewKind::Search);
    let body = move || match view_kind.get() {
        ViewKind::Reader => view! {
            <Reader s=s />
            <ChapterBar s=s />
        }
        .into_any(),
        ViewKind::Search => view! { <SearchResults s=s /> }.into_any(),
        ViewKind::Sermons => view! { <SermonListView s=s /> }.into_any(),
        ViewKind::Sermon(id) => view! { <SermonEditor s=s id=id /> }.into_any(),
    };
    let sheets = move || match open_sheet.get() {
        Some(StudySheet::Actions) => view! { <VerseActionsSheet s=s /> }.into_any(),
        Some(StudySheet::History) => view! { <HistorySheet s=s /> }.into_any(),
        Some(StudySheet::TextSize) => view! { <TextSizeSheet s=s /> }.into_any(),
        Some(StudySheet::Favourite(slot)) => view! { <FavouriteSheet s=s index=slot /> }.into_any(),
        Some(StudySheet::AddToSermon) => view! { <AddToSermonSheet s=s /> }.into_any(),
        None => ().into_any(),
    };

    view! {
        <div
            class="study"
            data-role="study"
            data-text-size=move || s.text_size.get().key()
        >
            <StudyHeader s=s />
            <Show when=bible_view>
                <StudyField s=s />
                <Favourites s=s />
            </Show>
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
            {move || {
                s.structures_failed
                    .get()
                    .map(|code| {
                        view! {
                            <div
                                class="study-error study-error--strip"
                                data-role="study-structure-error"
                                role="alert"
                            >
                                <p>"Zoznam kníh sa nepodarilo načítať. Odkazy a listovanie nefungujú."</p>
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="study-retry"
                                    on:click=move |_| ensure_structure(s, code.clone())
                                >
                                    "Skúsiť znova"
                                </button>
                            </div>
                        }
                    })
            }}
            {body}
            {sheets}
            <div
                class="study-notice"
                data-role="study-notice"
                data-visible=move || if s.notice.with(Option::is_some) { "true" } else { "false" }
                role="status"
            >
                {move || s.notice.get().unwrap_or_default()}
                {move || {
                    s.undo
                        .with(Option::is_some)
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class="study-notice__undo"
                                    data-role="study-notice-undo"
                                    on:click=move |_| run_undo(s)
                                >
                                    "Späť"
                                </button>
                            }
                        })
                }}
            </div>
        </div>
    }
}
