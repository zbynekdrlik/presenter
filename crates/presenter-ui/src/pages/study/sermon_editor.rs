//! One sermon's editor (spec §7.6): the cards, the save queue and every write.
//! All writes share the sermon's revision, so they run one at a time: an edit
//! waits 300 ms after the last tap, a structural write waits for the edits
//! before it. The page owns each editor (`editor_for`), so a save on its way
//! finishes after the preacher left the sermon, and a failed one is still there
//! when he returns. A failed save keeps the local state and tries again on the
//! next tap, on the button or when the network returns.

use std::collections::{BTreeSet, HashMap};

use gloo_timers::callback::Timeout;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};

use super::add_to_sermon::add_with_retry;
use super::scroll::ScrollPlan;
use super::sermon_add::AddVerse;
use super::sermon_item::SermonItemCard;
use super::sermon_live::listen_live;
use super::{ensure_structure, nav, sermon_error, Study, Undo};
use crate::api::sermons::{
    self, is_conflict, is_not_found, SermonDetail, SermonItem, SermonPassage,
};
use crate::api::ApiError;
use crate::state::sermon_edit::{retain_unchanged, ItemDraft, SaveState, TapMode};
use crate::state::sermon_list::{count_text, short_date};
use crate::state::study::{StudyRoute, StudySheet, StudyView};

const SAVE_AFTER_MS: u32 = 300;
const WAIT_STEP_MS: u32 = 50;

#[derive(Clone, PartialEq, Eq)]
enum Loaded {
    Waiting,
    Ready,
    Missing,
    Failed(String),
}

/// A sermon's state. `Copy` (signals only), passed to every card.
#[derive(Clone, Copy)]
pub(super) struct Editor {
    pub(super) s: Study,
    id: StoredValue<String>,
    loaded: RwSignal<Loaded>,
    pub(super) detail: RwSignal<Option<SermonDetail>>,
    pub(super) drafts: RwSignal<HashMap<String, ItemDraft>>,
    pub(super) mode: RwSignal<TapMode>,
    pub(super) save: RwSignal<SaveState>,
    dirty: StoredValue<BTreeSet<String>>,
    /// A write is on its way: the next one waits.
    busy: StoredValue<bool>,
    /// Items whose delete is on its way: a double tap deletes once.
    removing: StoredValue<BTreeSet<String>>,
    timer: StoredValue<Option<Timeout>, LocalStorage>,
}

/// This sermon's editor, created under the page's owner: no view's cleanup
/// cancels its timer or disposes what an answer on its way reads.
fn editor_for(s: Study, id: &str) -> Editor {
    if let Some(held) = s.editors.with_value(|editors| editors.get(id).copied()) {
        return held;
    }
    let e = s.owner.with_value(|owner| {
        owner.with(|| Editor {
            s,
            id: StoredValue::new(id.to_string()),
            loaded: RwSignal::new(Loaded::Waiting),
            detail: RwSignal::new(None),
            drafts: RwSignal::new(HashMap::new()),
            mode: RwSignal::new(TapMode::Strike),
            save: RwSignal::new(SaveState::Saved),
            dirty: StoredValue::new(BTreeSet::new()),
            busy: StoredValue::new(false),
            removing: StoredValue::new(BTreeSet::new()),
            timer: StoredValue::new_local(None),
        })
    });
    s.editors.update_value(|editors| {
        editors.insert(id.to_string(), e);
    });
    e
}

/// The editor of the sermon on the route.
pub(super) fn open_editor(s: Study) -> Option<Editor> {
    let id = s.route.with_untracked(|route| match &route.view {
        StudyView::Sermon { id } => Some(id.clone()),
        _ => None,
    })?;
    Some(editor_for(s, &id))
}

fn every_editor(s: Study) -> Vec<Editor> {
    s.editors
        .with_value(|editors| editors.values().copied().collect())
}

/// Out now, whatever the timer planned.
fn save_now(e: Editor) {
    if !e.dirty.with_value(BTreeSet::is_empty) {
        e.timer.set_value(None);
        flush(e);
    }
}

/// The page-wide moments an unsaved edit must not wait for: the network came
/// back, the phone left the app, the tab goes away.
pub(super) fn install(s: Study) {
    let save_all = move || every_editor(s).into_iter().for_each(save_now);
    let _ = window_event_listener(leptos::ev::online, move |_| save_all());
    let _ = window_event_listener(leptos::ev::pagehide, move |_| save_all());
    let _ = window_event_listener_untyped("visibilitychange", move |_| {
        let hidden = crate::utils::window::document().hidden();
        if hidden {
            save_all();
        }
    });
    let _ = window_event_listener(leptos::ev::beforeunload, move |event| {
        let unsaved = every_editor(s)
            .into_iter()
            .any(|e| !e.dirty.with_value(BTreeSet::is_empty));
        if unsaved {
            event.prevent_default();
        }
    });
}

impl Editor {
    pub(super) fn id(self) -> String {
        self.id.get_value()
    }

    /// A write of this page is on its way.
    pub(super) fn writing(self) -> bool {
        self.busy.get_value()
    }

    fn revision(self) -> i64 {
        self.detail
            .with_untracked(|detail| detail.as_ref().map_or(0, |held| held.revision))
    }

    /// The server's sermon. An item with unsaved taps keeps them, unless its
    /// text changed: the taps named another text's words. An answer older than
    /// what is held is ignored: it would take back a saved edit.
    fn apply(self, detail: SermonDetail) {
        if detail.revision < self.revision() {
            return;
        }
        let dirty = self.dirty.get_value();
        let mut lost = Vec::new();
        self.drafts.update(|drafts| {
            let mut next = HashMap::with_capacity(detail.items.len());
            for item in &detail.items {
                let draft = match drafts.remove(&item.id) {
                    Some(held) if dirty.contains(&item.id) && held.source == item.source => held,
                    Some(mut held) => {
                        if dirty.contains(&item.id) {
                            lost.push(item.id.clone());
                        }
                        held.accept(item.source.clone(), item.edits.clone());
                        held
                    }
                    None => ItemDraft::new(item.source.clone(), item.edits.clone()),
                };
                next.insert(item.id.clone(), draft);
            }
            *drafts = next;
        });
        self.dirty.update_value(|dirty| {
            dirty.retain(|id| detail.items.iter().any(|item| item.id == *id) && !lost.contains(id));
        });
        if !lost.is_empty() {
            self.s.tell(CHANGED_ELSEWHERE.to_string());
        }
        // What failed to save is gone: nothing is unsaved any more.
        if self.dirty.with_value(BTreeSet::is_empty)
            && self.save.get_untracked() == SaveState::Error
        {
            self.save.set(SaveState::Saved);
        }
        self.detail.set(Some(detail));
    }

    pub(super) fn item(self, id: &str) -> Option<SermonItem> {
        self.detail.with_untracked(|detail| {
            detail
                .as_ref()?
                .items
                .iter()
                .find(|item| item.id == id)
                .cloned()
        })
    }

    /// Change a draft with a gesture; a change is saved after the last tap.
    pub(super) fn edit(self, id: &str, gesture: impl FnOnce(&mut ItemDraft) -> bool) {
        let mut changed = false;
        self.drafts.update(|drafts| {
            if let Some(draft) = drafts.get_mut(id) {
                changed = gesture(draft);
            }
        });
        if changed {
            self.dirty.update_value(|dirty| {
                dirty.insert(id.to_string());
            });
            self.save.set(SaveState::Saving);
            self.timer
                .set_value(Some(Timeout::new(SAVE_AFTER_MS, move || {
                    flush(self);
                })));
        }
    }
}

const CHANGED_ELSEWHERE: &str =
    "Položka sa medzitým zmenila na inom zariadení — tvoja úprava sa neuložila";

fn drafted_edits(e: Editor, id: &str) -> Option<ItemEdits> {
    e.drafts
        .with_untracked(|drafts| drafts.get(id).map(|draft| draft.edits.clone()))
}

/// Save the next changed item, then the one after.
fn flush(e: Editor) {
    if e.busy.get_value() {
        return;
    }
    let Some(item) = e.dirty.with_value(|dirty| dirty.first().cloned()) else {
        e.save.set(SaveState::Saved);
        return;
    };
    let Some(edits) = drafted_edits(e, &item) else {
        e.dirty.update_value(|dirty| {
            dirty.remove(&item);
        });
        flush(e);
        return;
    };
    // The text these edits were made on: a reload may replace it during the PUT.
    let sent_on = e.item(&item).map(|held| (held.translation, held.source));
    e.busy.set_value(true);
    e.save.set(SaveState::Saving);
    leptos::task::spawn_local(async move {
        let answer = sermons::put_edits(&e.id(), &item, e.revision(), &edits).await;
        let answer = match answer {
            Err(err) if is_conflict(&err) => resend_after_conflict(e, &item, &edits, sent_on).await,
            other => other.map(Some),
        };
        e.busy.set_value(false);
        match answer {
            Ok(detail) => {
                // A tap landed during the request: it goes out next.
                if drafted_edits(e, &item).as_ref() == Some(&edits) {
                    e.dirty.update_value(|dirty| {
                        dirty.remove(&item);
                    });
                }
                if let Some(detail) = detail {
                    e.apply(detail);
                }
                flush(e);
            }
            // The item or the sermon is gone: no retry can store this.
            Err(err) if is_not_found(&err) => {
                e.dirty.update_value(|dirty| {
                    dirty.remove(&item);
                });
                e.s.tell(CHANGED_ELSEWHERE.to_string());
                reload(e);
                flush(e);
            }
            Err(err) => {
                e.save.set(SaveState::Error);
                e.s.tell(sermon_error(&err));
            }
        }
    });
}

/// 409 on an edit: another device moved the sermon. The same item with the same
/// text gets this edit once on the new revision (per-item last writer wins).
async fn resend_after_conflict(
    e: Editor,
    item: &str,
    edits: &ItemEdits,
    sent_on: Option<(String, Vec<SourceVerse>)>,
) -> Result<Option<SermonDetail>, ApiError> {
    let fresh = sermons::get_sermon(&e.id()).await?;
    let same = fresh
        .items
        .iter()
        .find(|held| held.id == item)
        .is_some_and(|held| {
            sent_on.as_ref() == Some(&(held.translation.clone(), held.source.clone()))
        });
    if same {
        let answer = sermons::put_edits(&e.id(), item, fresh.revision, edits).await?;
        e.s.tell("Zmeny z iného zariadenia načítané · tvoja úprava je uložená".to_string());
        Ok(Some(answer))
    } else {
        e.dirty.update_value(|dirty| {
            dirty.remove(item);
        });
        e.apply(fresh);
        e.s.tell(CHANGED_ELSEWHERE.to_string());
        Ok(None)
    }
}

/// Every edit out, nothing on its way: a structural write may start. `false`
/// when a save failed; the write waits for the preacher.
async fn settle_saves(e: Editor) -> bool {
    loop {
        if e.save.get_untracked() == SaveState::Error {
            return false;
        }
        if !e.busy.get_value() && e.dirty.with_value(BTreeSet::is_empty) {
            return true;
        }
        save_now(e);
        TimeoutFuture::new(WAIT_STEP_MS).await;
    }
}

/// This write's turn in the queue. Pair with [`end_turn`].
async fn take_turn(e: Editor) -> bool {
    if !settle_saves(e).await {
        return false;
    }
    e.busy.set_value(true);
    true
}

/// The queue moves on: a tap made during the write has only this to wake it.
fn end_turn(e: Editor) {
    e.busy.set_value(false);
    flush(e);
}

const SAVE_FIRST: &str = "Najprv sa musí uložiť úprava";

/// A refused write changes nothing, but a control the preacher moved (the
/// translation select) must read what is held again.
fn show_held(e: Editor) {
    e.detail.update(|_| {});
}

/// A write that is not an edit: never replayed after a 409 (spec §7.6). True
/// when it landed.
pub(super) async fn run_write<F, Fut>(e: Editor, refused: &'static str, op: F) -> bool
where
    F: FnOnce(String, i64) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<SermonDetail, ApiError>> + 'static,
{
    if !take_turn(e).await {
        e.s.tell(SAVE_FIRST.to_string());
        show_held(e);
        return false;
    }
    let answer = op(e.id(), e.revision()).await;
    let landed = match answer {
        Ok(detail) => {
            e.apply(detail);
            true
        }
        Err(err) if is_conflict(&err) || is_not_found(&err) => {
            e.s.tell(refused.to_string());
            reload(e);
            false
        }
        Err(err) => {
            e.s.tell(sermon_error(&err));
            show_held(e);
            false
        }
    };
    end_turn(e);
    landed
}

pub(super) fn write<F, Fut>(e: Editor, refused: &'static str, op: F)
where
    F: FnOnce(String, i64) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<SermonDetail, ApiError>> + 'static,
{
    leptos::task::spawn_local(async move {
        run_write(e, refused, op).await;
    });
}

pub(super) fn reload(e: Editor) {
    leptos::task::spawn_local(async move {
        match sermons::get_sermon(&e.id()).await {
            Ok(detail) => {
                e.apply(detail);
                e.loaded.set(Loaded::Ready);
            }
            Err(err) if is_not_found(&err) => {
                e.timer.set_value(None);
                e.dirty.update_value(BTreeSet::clear);
                e.detail.set(None);
                e.drafts.update(HashMap::clear);
                e.save.set(SaveState::Saved);
                e.loaded.set(Loaded::Missing);
            }
            Err(err) => e.loaded.set(Loaded::Failed(sermon_error(&err))),
        }
    });
}

pub(super) fn delete_item(e: Editor, item: SermonItem) {
    let Some(position) = e.detail.with_untracked(|detail| {
        detail
            .as_ref()?
            .items
            .iter()
            .position(|held| held.id == item.id)
    }) else {
        return;
    };
    // Read now: a tap still waiting for its save is part of what Späť restores.
    let edits = drafted_edits(e, &item.id).unwrap_or_default();
    let id = item.id.clone();
    let first = e
        .removing
        .try_update_value(|removing| removing.insert(id.clone()))
        .unwrap_or(false);
    if !first {
        return;
    }
    leptos::task::spawn_local(async move {
        let target = id.clone();
        let landed = run_write(
            e,
            "Položka sa medzitým zmenila — skús znova",
            move |sermon, revision| async move {
                sermons::delete_item(&sermon, &target, revision).await
            },
        )
        .await;
        if landed {
            e.s.tell_undo(
                "Odstránené".to_string(),
                Undo::Removed {
                    sermon: e.id(),
                    edits,
                    item: Box::new(item),
                    position,
                },
            );
        }
        e.removing.update_value(|removing| {
            removing.remove(&id);
        });
    });
}

async fn delete_with_retry(sermon: &str, item: &str) -> Result<SermonDetail, ApiError> {
    let held = sermons::get_sermon(sermon).await?;
    match sermons::delete_item(sermon, item, held.revision).await {
        Err(err) if is_conflict(&err) => {
            let fresh = sermons::get_sermon(sermon).await?;
            sermons::delete_item(sermon, item, fresh.revision).await
        }
        other => other,
    }
}

/// Späť on "Pridané": the added item goes again, in the sermon's queue, so an
/// editor open on it follows. A refusal keeps Späť on screen.
pub(super) fn undo_added(s: Study, sermon: String, item: String) {
    let e = editor_for(s, &sermon);
    leptos::task::spawn_local(async move {
        let offer_again = |message: String, item: String| {
            s.tell_undo(
                message,
                Undo::Added {
                    sermon: e.id(),
                    item,
                },
            );
        };
        if !take_turn(e).await {
            offer_again(SAVE_FIRST.to_string(), item);
            return;
        }
        match delete_with_retry(&e.id(), &item).await {
            Ok(detail) => {
                e.apply(detail);
                s.tell("Odstránené z kázne".to_string());
            }
            Err(err) => offer_again(sermon_error(&err), item),
        }
        end_turn(e);
    });
}

/// The deleted item's edits on the item that came back. The server snapshots
/// today's text, so only the verses still reading the same take theirs.
fn restore_edits(e: Editor, removed: &SermonItem, new_id: &str, edits: &ItemEdits) {
    let Some(back) = e.item(new_id) else {
        return;
    };
    let kept = retain_unchanged(&removed.source, &back.source, edits);
    if kept != *edits {
        e.s.tell("Text sa medzitým zmenil — úpravy zmenených veršov sa nevrátili".to_string());
    }
    if kept != ItemEdits::default() {
        e.edit(new_id, |draft| draft.restore(kept));
    }
}

/// Späť on "Odstránené", in the sermon's queue: the item at the same position
/// with its edits. A refusal keeps Späť on screen: the item exists only there.
pub(super) fn undo_removed(
    s: Study,
    sermon: String,
    item: SermonItem,
    position: usize,
    edits: ItemEdits,
) {
    let e = editor_for(s, &sermon);
    leptos::task::spawn_local(async move {
        let offer_again = |message: String, item: SermonItem, edits: ItemEdits| {
            s.tell_undo(
                message,
                Undo::Removed {
                    sermon: e.id(),
                    item: Box::new(item),
                    position,
                    edits,
                },
            );
        };
        if !take_turn(e).await {
            offer_again(SAVE_FIRST.to_string(), item, edits);
            return;
        }
        let passage = SermonPassage {
            translation: item.translation.clone(),
            book_code: item.book_code.clone(),
            chapter: item.chapter,
            verse_start: item.verse_start,
            verse_end: item.verse_end,
        };
        match add_with_retry(&e.id(), Some(position), &passage).await {
            Ok((detail, new_id)) => {
                e.apply(detail);
                // A normal edit from here: it retries like any other.
                restore_edits(e, &item, &new_id, &edits);
                end_turn(e);
            }
            Err(err) => {
                end_turn(e);
                offer_again(sermon_error(&err), item, edits);
            }
        }
    });
}

/// HOTOVO: everything is saved, or this says what is not. Back to the list.
fn finish(e: Editor) {
    leptos::task::spawn_local(async move {
        if !settle_saves(e).await {
            e.s.tell(SAVE_FIRST.to_string());
            return;
        }
        let translation = e.s.route.with_untracked(|route| route.translation.clone());
        nav::go(e.s, StudyRoute::sermons(translation), ScrollPlan::Top);
        e.s.tell(
            "Kázeň je uložená · u operátora v „Pripravené kazateľom“ v deň bohoslužby a na tablete"
                .to_string(),
        );
    });
}

#[component]
pub(super) fn SermonEditor(s: Study, id: String) -> impl IntoView {
    let e = editor_for(s, &id);
    listen_live(s);
    if e.detail.with_untracked(Option::is_none) {
        e.loaded.set(Loaded::Waiting);
    }
    Effect::new(move |_| {
        s.retry.track();
        reload(e);
    });
    let default_translation = Memo::new(move |_| {
        e.detail
            .with(|detail| detail.as_ref().map(|held| held.default_translation.clone()))
    });
    Effect::new(move |_| {
        if let Some(code) = default_translation.get() {
            ensure_structure(s, code);
        }
    });

    let header = move || {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .map(|held| format!("Kázeň · {}", short_date(held.service_date)))
                .unwrap_or_else(|| "Kázeň".to_string())
        })
    };
    let title = move || {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .map(|held| held.title.clone())
                .unwrap_or_default()
        })
    };
    let item_ids = move || {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .map(|held| held.items.iter().map(|item| item.id.clone()).collect())
                .unwrap_or_else(Vec::new)
        })
    };
    let footer = move || {
        e.detail.with(|detail| {
            detail.as_ref().map(|held| {
                let slides = held.items.iter().map(|item| item.slide_ids.len()).sum();
                count_text(held.items.len(), slides)
            })
        })
    };
    let state = move || {
        e.loaded.with(|loaded| match loaded {
            Loaded::Waiting => "loading",
            Loaded::Ready => "ready",
            Loaded::Missing => "missing",
            Loaded::Failed(_) => "error",
        })
    };
    let to_list = move |_| {
        let translation = s.route.with_untracked(|route| route.translation.clone());
        nav::go(s, StudyRoute::sermons(translation), ScrollPlan::Top);
    };

    view! {
        <main class="study-scroll study-scroll--results">
            <section
                class="sermon-editor"
                data-role="sermon-editor"
                data-state=state
            >
                <div class="sermon-editor__head">
                    <div>
                        <h2 class="sermon-section__title" data-role="sermon-date">{header}</h2>
                        <p class="sermon-editor__title" data-role="sermon-title">{title}</p>
                    </div>
                    <button
                        type="button"
                        class="study-button"
                        data-role="sermon-preview-open"
                        on:click=move |_| {
                            nav::open_sheet(s, nav::sheet_route(s, StudySheet::Preview));
                        }
                    >
                        "Náhľad"
                    </button>
                    <span
                        class="sermon-save"
                        data-role="sermon-save"
                        data-save-state=move || e.save.get().key()
                    >
                        {move || match e.save.get() {
                            SaveState::Saved => "Uložené",
                            SaveState::Saving => "Ukladá sa…",
                            SaveState::Error => "Neuložené",
                        }}
                    </span>
                </div>
                {move || {
                    (e.save.get() == SaveState::Error)
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="sermon-save-retry"
                                    on:click=move |_| flush(e)
                                >
                                    "Neuložené — skúsiť znova"
                                </button>
                            }
                        })
                }}
                {move || match e.loaded.get() {
                    Loaded::Missing => {
                        view! {
                            <div class="study-error" data-role="sermon-missing" role="alert">
                                <p>"Nenašlo sa"</p>
                                <button type="button" class="study-button" on:click=to_list>
                                    "Späť na zoznam"
                                </button>
                            </div>
                        }
                            .into_any()
                    }
                    Loaded::Failed(message) => {
                        view! {
                            <div class="study-error" data-role="sermon-error" role="alert">
                                <p>{message}</p>
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="study-retry"
                                    on:click=move |_| reload(e)
                                >
                                    "Skúsiť znova"
                                </button>
                            </div>
                        }
                            .into_any()
                    }
                    _ => ().into_any(),
                }}
                <For
                    each=item_ids
                    key=|id: &String| id.clone()
                    children=move |id| view! { <SermonItemCard e=e id=id /> }
                />
                {move || {
                    e.detail
                        .with(|detail| detail.as_ref().is_some_and(|held| held.items.is_empty()))
                        .then(|| {
                            view! {
                                <p class="study-empty" data-role="sermon-empty">
                                    "Zatiaľ žiadne položky"
                                </p>
                            }
                        })
                }}
                <AddVerse e=e />
                <p class="sermon-editor__footer" data-role="sermon-counts">{footer}</p>
                <div class="sermon-bar">
                    <button
                        type="button"
                        class="study-button"
                        data-role="sermon-reorder-open"
                        on:click=move |_| {
                            nav::open_sheet(s, nav::sheet_route(s, StudySheet::Reorder));
                        }
                    >
                        "Poradie"
                    </button>
                    <button
                        type="button"
                        class="study-button study-button--primary"
                        data-role="sermon-done"
                        on:click=move |_| finish(e)
                    >
                        "HOTOVO"
                    </button>
                </div>
            </section>
        </main>
    }
}
