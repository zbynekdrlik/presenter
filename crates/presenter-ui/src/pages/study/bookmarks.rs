//! Bookmarks (#915, reshaped by #951 — owner, 2026-11-14: "ja len potrebujem
//! vediet kde som skoncil citanie"). "Pridať záložku" in the verse sheet adds the
//! first selected verse at once (#932); the header's Záložky sheet (`s=zalozky`)
//! lists them NEWEST FIRST, each row the place ("Hebrejom 11:4", its
//! translation as a small tag) and the start of the verse's text, the whole row
//! opening the reader there. Nothing else per row: a drag on the handle reorders
//! (pointer events on a `touch-action: none` handle, as `sermon_reorder.rs`; the
//! decisions are `nameplates::order`'s), and "Vybrať" in the head turns the rows
//! into choices for ONE "Zmazať (N)" under the list, with Späť. No rename, no
//! "Presunúť sem" (0 presses in 30 days on SNV). The server keeps the list (the
//! phone and the PC share it); lookups and `study.last` never touch it.

use leptos::prelude::*;
use web_sys::PointerEvent;

use super::scroll::ScrollPlan;
use super::sheet::Sheet;
use super::{nav, sermon_error, Chapter, Study, Undo};
use crate::api::bookmarks::{self as api, Bookmark, BookmarkPlace};
use crate::api::ApiError;
use crate::components::drag_rows;
use crate::components::nameplates::order::{apply_order, drop_marker, reorder_to_slot};
use crate::state::study_bookmarks::{
    chosen_in_order, delete_label, deleted_notice, offered_name, open_route, reading_place,
    restore_order, toggle_chosen, translation_tag, ReadingPlace,
};

/// What the Záložky sheet shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum BookmarkList {
    #[default]
    Loading,
    Ready(Vec<Bookmark>),
    Failed(String),
}

/// `MouseEvent.button` of the primary button (a finger, the left mouse button).
const PRIMARY_BUTTON: i16 = 0;

fn to_api(place: ReadingPlace) -> BookmarkPlace {
    BookmarkPlace {
        translation: place.translation,
        book_code: place.book,
        chapter: place.chapter,
        verse: place.verse,
    }
}

fn of(bookmark: &Bookmark) -> BookmarkPlace {
    BookmarkPlace {
        translation: bookmark.translation.clone(),
        book_code: bookmark.book_code.clone(),
        chapter: bookmark.chapter,
        verse: bookmark.verse,
    }
}

fn load(s: Study) {
    leptos::task::spawn_local(async move {
        s.bookmarks.set(match api::list().await {
            Ok(list) => BookmarkList::Ready(list),
            Err(err) => BookmarkList::Failed(sermon_error(&err)),
        });
    });
}

/// "Pridať záložku" in the verse sheet (#932, owner 2026-11-11: "miesto aby sa
/// rovno pridala tak vyskočí ďalšie okno … Načo je to dobré?!"): one tap adds
/// the first selected verse under its place ("Hebrejom 11:4"), on TOP of the
/// list (#951). The verse sheet closes now, the answer is a notice. `true` = the
/// sheet was closed.
pub(super) fn add_here(s: Study) -> bool {
    let route = s.route.get_untracked();
    let (Some(place), Some(name)) = (
        reading_place(&route, s.translation().as_deref(), None),
        untrack(|| offered(s)),
    ) else {
        // The chapter on screen is not the route's yet: say so, no dead button.
        s.tell("Verš sa ešte načítava, skús znova".to_string());
        return false;
    };
    nav::close_sheet(s);
    leptos::task::spawn_local(async move {
        match api::create(&name, &to_api(place), None).await {
            Ok(made) => s.tell(format!("Záložka pridaná: {}", made.name)),
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
    true
}

/// Späť on "Zmazané …": each comes back where it stood, the lowest index first
/// (`restore_order`), so every row lands on its old place.
pub(super) fn undo_delete(s: Study, items: Vec<(Bookmark, usize)>) {
    leptos::task::spawn_local(async move {
        let items = restore_order(items);
        for (at, (bookmark, index)) in items.iter().enumerate() {
            if let Err(err) = api::create(&bookmark.name, &of(bookmark), Some(*index)).await {
                // The ones not back yet keep their Späť.
                s.tell_undo(
                    sermon_error(&err),
                    Undo::Bookmarks {
                        items: items[at..].to_vec(),
                    },
                );
                load(s);
                return;
            }
        }
        s.tell(if items.len() == 1 {
            "Záložka je späť".to_string()
        } else {
            "Záložky sú späť".to_string()
        });
        load(s);
    });
}

fn open(s: Study, bookmark: &Bookmark) {
    let route = open_route(
        &bookmark.translation,
        &bookmark.book_code,
        bookmark.chapter,
        bookmark.verse,
    );
    nav::go(s, route, ScrollPlan::Verse(bookmark.verse));
}

/// The rows on screen: none while the list is not `Ready`.
fn ready_rows(s: Study) -> Vec<Bookmark> {
    s.bookmarks.with(|list| match list {
        BookmarkList::Ready(rows) => rows.clone(),
        _ => Vec::new(),
    })
}

fn ids_now(s: Study) -> Vec<i64> {
    s.bookmarks.with_untracked(|list| match list {
        BookmarkList::Ready(rows) => rows.iter().map(|row| row.id).collect(),
        _ => Vec::new(),
    })
}

/// The page's bookmark writes (#951): ONE at a time, an order before a delete,
/// owned by the page, so a write still waiting when the sheet closes is sent,
/// and a waiting order never lands after a delete as a stale 409 whose toast
/// would replace the delete's Späť.
#[derive(Default)]
pub(super) struct BookmarkWrites {
    /// A write on its way.
    sending: bool,
    /// The newest order chosen and not sent yet.
    order: Option<Vec<i64>>,
    /// The chosen ids of "Zmazať (N)", with the sheet that asked.
    delete: Option<(Vec<i64>, ListUi)>,
}

/// Send the next waiting write, if none is on its way.
fn pump(s: Study) {
    let next = s.bookmark_writes.try_update_value(|writes| {
        if writes.sending {
            return None;
        }
        let next = match writes.order.take() {
            Some(ids) => Some(Write::Order(ids)),
            None => writes.delete.take().map(|(ids, ui)| Write::Delete(ids, ui)),
        };
        writes.sending = next.is_some();
        next
    });
    let Some(next) = next.flatten() else {
        return;
    };
    leptos::task::spawn_local(async move {
        match next {
            Write::Order(ids) => {
                if let Err(err) = api::reorder(&ids).await {
                    // Another device changed the list: show it as stored.
                    s.tell(sermon_error(&err));
                    load(s);
                }
            }
            Write::Delete(ids, ui) => deleted(s, ui, &ids, api::delete_many(&ids).await),
        }
        s.bookmark_writes
            .update_value(|writes| writes.sending = false);
        pump(s);
    });
}

enum Write {
    Order(Vec<i64>),
    Delete(Vec<i64>, ListUi),
}

/// A delete answered. The sheet may be gone by now: its signals are written
/// with `try_*` only.
fn deleted(s: Study, ui: ListUi, ids: &[i64], answer: Result<Vec<api::Deleted>, ApiError>) {
    ui.busy.try_set(false);
    match answer {
        Ok(deleted) => {
            s.bookmarks.update(|list| {
                if let BookmarkList::Ready(rows) = list {
                    rows.retain(|row| !ids.contains(&row.id));
                }
            });
            ui.choosing.try_set(false);
            ui.chosen.try_set(Vec::new());
            s.tell_undo(
                deleted_notice(deleted.len()),
                Undo::Bookmarks {
                    items: deleted
                        .into_iter()
                        .map(|item| (item.bookmark, item.index))
                        .collect(),
                },
            );
        }
        Err(err) => {
            s.tell(sermon_error(&err));
            load(s);
        }
    }
}

/// The sheet's own state, `Copy` for the rows.
#[derive(Clone, Copy)]
pub(super) struct ListUi {
    s: Study,
    /// "Vybrať" is on: a row tap chooses instead of opening.
    choosing: RwSignal<bool>,
    chosen: RwSignal<Vec<i64>>,
    /// The dragged id and the slot it would drop into now.
    drag: RwSignal<Option<(i64, usize)>>,
    /// A delete waiting or on its way.
    busy: RwSignal<bool>,
}

impl ListUi {
    fn new(s: Study) -> Self {
        Self {
            s,
            choosing: RwSignal::new(false),
            chosen: RwSignal::new(Vec::new()),
            drag: RwSignal::new(None),
            busy: RwSignal::new(false),
        }
    }

    /// The chosen ids still listed, in the list's order.
    fn chosen_now(self) -> Vec<i64> {
        let listed: Vec<i64> = ready_rows(self.s).iter().map(|row| row.id).collect();
        self.chosen.with(|chosen| chosen_in_order(&listed, chosen))
    }

    fn set_choosing(self, on: bool) {
        self.choosing.set(on);
        self.chosen.set(Vec::new());
    }

    /// A drop: the rows move at once, the order goes to the server (only the
    /// newest waiting order is sent next, so the server ends on the last one).
    fn drop_at(self, id: i64, slot: usize) {
        let Some(ids) = reorder_to_slot(&ids_now(self.s), &id, slot) else {
            return;
        };
        self.s.bookmarks.update(|list| {
            if let BookmarkList::Ready(rows) = list {
                apply_order(rows, &ids, |row| row.id);
            }
        });
        self.s
            .bookmark_writes
            .update_value(|writes| writes.order = Some(ids));
        pump(self.s);
    }

    /// "Zmazať (N)": the chosen ones at once (after any order still on its
    /// way), then a toast with Späť.
    fn delete_chosen(self) {
        let ids = self.chosen_now();
        if ids.is_empty() || self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.s
            .bookmark_writes
            .update_value(|writes| writes.delete = Some((ids, self)));
        pump(self.s);
    }
}

const HANDLE: &str = "[data-role=\"bookmark-drag\"]";

/// The drop slot under the pointer, measured on the rows on screen.
fn slot_under(ev: &PointerEvent) -> Option<usize> {
    drag_rows::slot_under(
        ev,
        HANDLE,
        "[data-role=\"bookmarks-list\"]",
        "[data-role=\"bookmark-row\"]",
    )
}

/// The handle: press, move, release. A press captures the pointer, so the
/// moves and the release reach the handle anywhere on the screen.
#[component]
fn DragHandle(ui: ListUi, id: i64, position: Memo<Option<usize>>) -> impl IntoView {
    let drag = ui.drag;
    let on_down = move |ev: PointerEvent| {
        if ev.button() != PRIMARY_BUTTON {
            return;
        }
        let Some(handle) = drag_rows::handle_of(&ev, HANDLE) else {
            return;
        };
        ev.prevent_default();
        let _ = handle.set_pointer_capture(ev.pointer_id());
        drag.set(Some((id, position.get_untracked().unwrap_or(0))));
    };
    let on_move = move |ev: PointerEvent| {
        if drag.with_untracked(Option::is_none) {
            return;
        }
        if ev.buttons() == 0 {
            drag.set(None);
            return;
        }
        if let Some(slot) = slot_under(&ev) {
            drag.update(|held| {
                if let Some(held) = held {
                    held.1 = slot;
                }
            });
        }
    };
    let on_up = move |_: PointerEvent| {
        let Some((id, slot)) = drag.get_untracked() else {
            return;
        };
        drag.set(None);
        ui.drop_at(id, slot);
    };
    view! {
        <button
            type="button"
            class="study-bookmark__handle"
            data-role="bookmark-drag"
            aria-label="Presunúť záložku"
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=move |_| drag.set(None)
            on:lostpointercapture=move |_| drag.set(None)
        >
            "\u{2261}"
        </button>
    }
}

#[component]
fn BookmarkRow(ui: ListUi, bookmark: Bookmark) -> impl IntoView {
    let s = ui.s;
    let id = bookmark.id;
    let place = offered_name(&bookmark.book, bookmark.chapter, bookmark.verse);
    let tag = translation_tag(&bookmark.translation);
    let text = bookmark.text.clone();
    // The row's place follows the list, never captured: a drop moves it.
    let position = Memo::new(move |_| {
        s.bookmarks.with(|list| match list {
            BookmarkList::Ready(rows) => rows.iter().position(|row| row.id == id),
            _ => None,
        })
    });
    let count = Memo::new(move |_| {
        s.bookmarks.with(|list| match list {
            BookmarkList::Ready(rows) => rows.len(),
            _ => 0,
        })
    });
    let marker = move || {
        let (dragged, slot) = ui.drag.get()?;
        let from = ids_now(s).iter().position(|held| *held == dragged)?;
        drop_marker(from, slot, position.get()?, count.get())
    };
    let chosen = Memo::new(move |_| ui.chosen.with(|chosen| chosen.contains(&id)));
    let opened = bookmark.clone();
    let on_tap = move |_| {
        if ui.choosing.get_untracked() {
            ui.chosen.update(|chosen| toggle_chosen(chosen, id));
        } else {
            open(s, &opened);
        }
    };
    view! {
        <div
            class="study-bookmark"
            data-role="bookmark-row"
            data-bookmark-id=id.to_string()
            data-chosen=move || chosen.get().then_some("true")
            data-drop=marker
            data-dragging=move || {
                ui.drag.with(|held| held.is_some_and(|(dragged, _)| dragged == id)).then_some("true")
            }
        >
            <Show when=move || ui.choosing.get()>
                <span class="study-bookmark__check" aria-hidden="true">
                    {move || if chosen.get() { "\u{2713}" } else { "" }}
                </span>
            </Show>
            <button
                type="button"
                class="study-bookmark__open"
                data-role=move || if ui.choosing.get() { "bookmark-pick" } else { "bookmark-open" }
                aria-pressed=move || ui.choosing.get().then(|| chosen.get().to_string())
                on:click=on_tap
            >
                <span class="study-bookmark__place">
                    <span class="study-bookmark__label">{place}</span>
                    <span class="study-bookmark__tag">{tag}</span>
                </span>
                {text
                    .map(|text| {
                        view! {
                            <span class="study-bookmark__text" data-role="bookmark-text">
                                {text}
                            </span>
                        }
                    })}
            </button>
            <Show when=move || !ui.choosing.get()>
                <DragHandle ui=ui id=id position=position />
            </Show>
        </div>
    }
}

#[component]
fn ListState(s: Study) -> impl IntoView {
    move || match s.bookmarks.get() {
        BookmarkList::Loading => view! {
            <p class="study-empty" data-role="bookmarks-loading">"Načítavam…"</p>
        }
        .into_any(),
        BookmarkList::Failed(message) => view! {
            <div class="study-error" data-role="bookmarks-error" role="alert">
                <p>{message}</p>
                <button
                    type="button"
                    class="study-button"
                    data-role="bookmarks-retry"
                    on:click=move |_| load(s)
                >
                    "Skúsiť znova"
                </button>
            </div>
        }
        .into_any(),
        BookmarkList::Ready(rows) if rows.is_empty() => view! {
            <p class="study-empty" data-role="bookmarks-empty">
                "Zatiaľ žiadne záložky. Ťukni na verš a zvoľ „Pridať záložku“."
            </p>
        }
        .into_any(),
        BookmarkList::Ready(_) => ().into_any(),
    }
}

/// "Vybrať" / "Hotovo" in the head: only with rows to choose from.
fn choose_toggle(ui: ListUi) -> impl IntoView {
    let any = Memo::new(move |_| {
        ui.s.bookmarks
            .with(|list| matches!(list, BookmarkList::Ready(rows) if !rows.is_empty()))
    });
    move || {
        (any.get() || ui.choosing.get()).then(|| {
            view! {
                <button
                    type="button"
                    class="study-button study-sheet__action"
                    data-role="bookmarks-choose"
                    aria-pressed=move || ui.choosing.get().to_string()
                    on:click=move |_| ui.set_choosing(!ui.choosing.get_untracked())
                >
                    {move || if ui.choosing.get() { "Hotovo" } else { "Vybrať" }}
                </button>
            }
        })
    }
}

/// The ONE delete under the list while choosing.
fn delete_bar(ui: ListUi) -> impl IntoView {
    let count = Memo::new(move |_| ui.chosen_now().len());
    move || {
        ui.choosing.get().then(|| {
            view! {
                <button
                    type="button"
                    class="study-button study-button--danger"
                    data-role="bookmarks-delete"
                    disabled=move || count.get() == 0 || ui.busy.get()
                    on:click=move |_| ui.delete_chosen()
                >
                    {move || delete_label(count.get())}
                </button>
            }
        })
    }
}

#[component]
pub(super) fn BookmarksSheet(s: Study) -> impl IntoView {
    // A list from an earlier visit is never shown as this one's answer.
    s.bookmarks.set(BookmarkList::Loading);
    load(s);
    let ui = ListUi::new(s);
    // The settle marker of the specs: `ready` once this visit's list answered.
    let state = Signal::derive(move || match s.bookmarks.with(|list| list.clone()) {
        BookmarkList::Loading => "loading",
        BookmarkList::Ready(_) => "ready",
        BookmarkList::Failed(_) => "error",
    });
    // A drag whose row is gone (a reload after a refused order) never ends on
    // its own: its handle, and so its release, is gone.
    Effect::new(move |_| {
        let gone = ui
            .drag
            .get()
            .is_some_and(|(id, _)| !ready_rows(s).iter().any(|row| row.id == id));
        if gone {
            ui.drag.set(None);
        }
    });
    view! {
        <Sheet
            s=s
            role="bookmarks"
            title=Signal::derive(|| "Záložky".to_string())
            modal=true
            state=state
            action=ViewFn::from(move || choose_toggle(ui))
            footer=ViewFn::from(move || delete_bar(ui))
        >
            <ListState s=s />
            <div
                class="study-list study-bookmarks"
                data-role="bookmarks-list"
                data-choosing=move || ui.choosing.get().then_some("true")
            >
                <For
                    each=move || ready_rows(s)
                    key=|row| (row.id, row.text.clone())
                    children=move |row| view! { <BookmarkRow ui=ui bookmark=row /> }
                />
            </div>
        </Sheet>
    }
}

/// The name of the first selected verse, once its chapter is here.
fn offered(s: Study) -> Option<String> {
    let verse = s.route.with(|route| route.verses.map(|(first, _)| first))?;
    let wanted = s.place()?;
    s.chapter.with(|chapter| match chapter {
        Chapter::Ready(place, text) if *place == wanted => {
            Some(offered_name(&text.book, place.chapter, verse))
        }
        _ => None,
    })
}
