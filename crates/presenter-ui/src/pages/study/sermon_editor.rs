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

use super::scroll::ScrollPlan;
use super::sermon_add::AddVerse;
use super::sermon_item::SermonItemCard;
use super::sermon_live::listen_live;
use super::sermon_reorder::{order_moved, send_order};
use super::{ensure_structure, nav, sermon_error, Study, Undo};
use crate::api::sermons::{
    self, is_conflict, is_not_found, is_refused_edit, SermonDetail, SermonItem,
};
use crate::api::ApiError;
use crate::state::sermon_edit::{ItemDraft, SaveState, TapMode};
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
    /// The order on Poradie: the sheet's rows, and what is sent as it leaves. It
    /// outlives the sheet until it is stored.
    pub(super) order: RwSignal<Option<OrderDraft>>,
    /// That order failed to go out: it waits for the retry button.
    pub(super) order_failed: RwSignal<bool>,
    pub(super) order_sending: StoredValue<bool>,
    /// Poradie is on screen.
    pub(super) order_open: StoredValue<bool>,
    /// The newest change this page could not store, for HOTOVO to name.
    refused: StoredValue<Option<String>>,
    /// An event came during a write of ours: the sermon reads again after it.
    behind: StoredValue<bool>,
}

/// Poradie's rows, and the order they started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrderDraft {
    pub(super) base: Vec<String>,
    pub(super) ids: Vec<String>,
}

/// How a queued write ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteEnd {
    Landed,
    /// The server said no: trying the same again cannot help.
    Refused,
    /// It may work later.
    Failed,
}

/// This sermon's editor, created under the page's owner: no view's cleanup
/// cancels its timer or disposes what an answer on its way reads.
pub(super) fn editor_for(s: Study, id: &str) -> Editor {
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
            order: RwSignal::new(None),
            order_failed: RwSignal::new(false),
            order_sending: StoredValue::new(false),
            order_open: StoredValue::new(false),
            refused: StoredValue::new(None),
            behind: StoredValue::new(false),
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
    let save_all = move || {
        for e in every_editor(s) {
            save_now(e);
            send_order(e);
        }
    };
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
            .any(|e| !e.dirty.with_value(BTreeSet::is_empty) || order_moved(e));
        if unsaved {
            event.prevent_default();
        }
    });
}

impl Editor {
    pub(super) fn id(self) -> String {
        self.id.get_value()
    }

    /// Another device wrote: read again, after a write of ours is back.
    pub(super) fn follow(self) {
        if self.busy.get_value() {
            self.behind.set_value(true);
        } else {
            reload(self);
        }
    }

    /// The items' ids in the stored order.
    pub(super) fn held_order(self) -> Vec<String> {
        self.detail.with_untracked(|detail| {
            detail
                .as_ref()
                .map(|held| held.items.iter().map(|item| item.id.clone()).collect())
                .unwrap_or_default()
        })
    }

    fn revision(self) -> i64 {
        self.detail
            .with_untracked(|detail| detail.as_ref().map_or(0, |held| held.revision))
    }

    /// The server's sermon. An item with unsaved taps keeps them, unless its
    /// text changed: the taps named another text's words. An answer older than
    /// what is held is ignored: it would take back a saved edit.
    pub(super) fn apply(self, detail: SermonDetail) {
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
            refuse(self, CHANGED_ELSEWHERE);
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

/// Say what did not get stored, and keep it for HOTOVO.
pub(super) fn refuse(e: Editor, message: &str) {
    e.refused.set_value(Some(message.to_string()));
    e.s.tell(message.to_string());
}

const REFUSED_EDIT: &str = "Úpravu server neprijal — položka sa načítala znova";
const CHANGED_ELSEWHERE: &str =
    "Položka sa medzitým zmenila na inom zariadení — tvoja úprava sa neuložila";

fn drafted_edits(e: Editor, id: &str) -> Option<ItemEdits> {
    e.drafts
        .with_untracked(|drafts| drafts.get(id).map(|draft| draft.edits.clone()))
}

/// What another device wrote during our write.
fn catch_up(e: Editor) {
    if e.behind.get_value() {
        e.behind.set_value(false);
        reload(e);
    }
}

/// Save the next changed item, then the one after.
fn flush(e: Editor) {
    if e.busy.get_value() {
        return;
    }
    let Some(item) = e.dirty.with_value(|dirty| dirty.first().cloned()) else {
        e.save.set(SaveState::Saved);
        catch_up(e);
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
                refuse(e, CHANGED_ELSEWHERE);
                reload(e);
                flush(e);
            }
            // Retrying would only block every other write behind it.
            Err(err) if is_refused_edit(&err) => {
                e.dirty.update_value(|dirty| {
                    dirty.remove(&item);
                });
                refuse(e, REFUSED_EDIT);
                reload(e);
                flush(e);
            }
            Err(err) => {
                e.save.set(SaveState::Error);
                e.s.tell(sermon_error(&err));
                catch_up(e);
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
        refuse(e, CHANGED_ELSEWHERE);
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
pub(super) async fn take_turn(e: Editor) -> bool {
    if !settle_saves(e).await {
        return false;
    }
    e.busy.set_value(true);
    true
}

/// The queue moves on: a tap made during the write has only this to wake it.
pub(super) fn end_turn(e: Editor) {
    e.busy.set_value(false);
    flush(e);
}

pub(super) const SAVE_FIRST: &str = "Najprv sa musí uložiť úprava";

/// A refused write changes nothing, but a control the preacher moved (the
/// translation select) must read what is held again.
fn show_held(e: Editor) {
    e.detail.update(|_| {});
}

/// A write that is not an edit: never replayed after a 409 (spec §7.6).
pub(super) async fn run_write<F, Fut>(e: Editor, refused: &'static str, op: F) -> WriteEnd
where
    F: FnOnce(String, i64) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<SermonDetail, ApiError>> + 'static,
{
    if !take_turn(e).await {
        e.s.tell(SAVE_FIRST.to_string());
        show_held(e);
        return WriteEnd::Failed;
    }
    let answer = op(e.id(), e.revision()).await;
    let end = match answer {
        Ok(detail) => {
            e.apply(detail);
            WriteEnd::Landed
        }
        Err(err) if is_conflict(&err) || is_not_found(&err) => {
            refuse(e, refused);
            reload(e);
            WriteEnd::Refused
        }
        Err(err) => {
            e.s.tell(sermon_error(&err));
            show_held(e);
            WriteEnd::Failed
        }
    };
    end_turn(e);
    end
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
            // A sermon on screen stays: only a first load has nothing to show.
            Err(err) => {
                if e.detail.with_untracked(Option::is_none) {
                    e.loaded.set(Loaded::Failed(sermon_error(&err)));
                }
            }
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
        let end = run_write(
            e,
            "Položka sa medzitým zmenila — skús znova",
            move |sermon, revision| async move {
                sermons::delete_item(&sermon, &target, revision).await
            },
        )
        .await;
        if end == WriteEnd::Landed {
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

/// HOTOVO: everything is stored, or this says what is not and stays. Back to
/// the list, unless the preacher went elsewhere meanwhile.
fn finish(e: Editor) {
    send_order(e);
    leptos::task::spawn_local(async move {
        if !settle_saves(e).await {
            e.s.tell(SAVE_FIRST.to_string());
            return;
        }
        while e.order_sending.get_value() {
            TimeoutFuture::new(WAIT_STEP_MS).await;
        }
        if e.order_failed.get_untracked() {
            e.s.tell("Poradie sa neuložilo — skús znova".to_string());
            return;
        }
        if let Some(message) = e.refused.try_update_value(Option::take).flatten() {
            e.s.tell(message);
            return;
        }
        if open_editor(e.s).is_none_or(|open| open.id() != e.id()) {
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
    e.refused.set_value(None);
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
                data-live=move || s.live.get().is_some_and(|connects| connects > 0).then_some("true")
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
                {move || {
                    e.order_failed
                        .get()
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="sermon-order-retry"
                                    on:click=move |_| send_order(e)
                                >
                                    "Poradie sa neuložilo — skúsiť znova"
                                </button>
                            }
                        })
                }}
                <Show when=move || e.detail.with(Option::is_some)>
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
                </Show>
            </section>
        </main>
    }
}
