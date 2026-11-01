//! One sermon's editor (spec §7.6): the cards, the save queue and every write.
//! All writes share the sermon's revision, so they run one at a time: an edit
//! waits 300 ms after the last tap, a structural write waits for the edits
//! before it. A failed save keeps the local state and tries again on the next
//! tap, on the button or when the network returns.

use std::collections::{BTreeSet, HashMap};

use gloo_timers::callback::Timeout;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::bible::sermon::ItemEdits;

use super::scroll::ScrollPlan;
use super::sermon_item::SermonItemCard;
use super::{ensure_structure, nav, sermon_error, Study, Undo};
use crate::api::sermons::{
    self, is_conflict, is_not_found, SermonDetail, SermonItem, SermonPassage,
};
use crate::api::ApiError;
use crate::state::sermon_edit::{ItemDraft, SaveState, TapMode};
use crate::state::sermon_list::{count_text, short_date};
use crate::state::study::{suggest, StudyRoute};

const SAVE_AFTER_MS: u32 = 300;
const WAIT_STEP_MS: u32 = 50;

#[derive(Clone, PartialEq, Eq)]
enum Loaded {
    Waiting,
    Ready,
    Missing,
    Failed(String),
}

/// The open sermon. `Copy` (signals only), passed to every card.
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
    timer: StoredValue<Option<Timeout>, LocalStorage>,
}

impl Editor {
    fn id(self) -> String {
        self.id.get_value()
    }

    fn revision(self) -> i64 {
        self.detail
            .with_untracked(|detail| detail.as_ref().map_or(0, |held| held.revision))
    }

    /// The server's sermon. An item with unsaved taps keeps them.
    fn apply(self, detail: SermonDetail) {
        let dirty = self.dirty.get_value();
        self.drafts.update(|drafts| {
            let mut next = HashMap::with_capacity(detail.items.len());
            for item in &detail.items {
                let draft = match drafts.remove(&item.id) {
                    Some(held) if dirty.contains(&item.id) && held.source == item.source => held,
                    Some(mut held) => {
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
            dirty.retain(|id| detail.items.iter().any(|item| item.id == *id));
        });
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
    e.busy.set_value(true);
    e.save.set(SaveState::Saving);
    leptos::task::spawn_local(async move {
        let answer = sermons::put_edits(&e.id(), &item, e.revision(), &edits).await;
        let answer = match answer {
            Err(err) if is_conflict(&err) => resend_after_conflict(e, &item, &edits).await,
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
) -> Result<Option<SermonDetail>, ApiError> {
    let sent_source = e.item(item).map(|held| (held.translation, held.source));
    let fresh = sermons::get_sermon(&e.id()).await?;
    let same = fresh
        .items
        .iter()
        .find(|held| held.id == item)
        .is_some_and(|held| {
            sent_source.as_ref() == Some(&(held.translation.clone(), held.source.clone()))
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
        e.s.tell(
            "Položka sa medzitým zmenila na inom zariadení — tvoja úprava sa neuložila".to_string(),
        );
        Ok(None)
    }
}

/// Every edit out, nothing on its way: a structural write may start. `false`
/// when a save failed; the write waits for the preacher.
async fn settle_saves(e: Editor) -> bool {
    e.timer.set_value(None);
    flush(e);
    loop {
        if e.save.get_untracked() == SaveState::Error {
            return false;
        }
        if !e.busy.get_value() && e.dirty.with_value(BTreeSet::is_empty) {
            return true;
        }
        TimeoutFuture::new(WAIT_STEP_MS).await;
    }
}

/// A write that is not an edit: never replayed after a 409 (spec §7.6).
pub(super) fn write<F, Fut>(e: Editor, refused: &'static str, op: F)
where
    F: FnOnce(String, i64) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<SermonDetail, ApiError>> + 'static,
{
    leptos::task::spawn_local(async move {
        if !settle_saves(e).await {
            e.s.tell("Najprv sa musí uložiť úprava".to_string());
            return;
        }
        e.busy.set_value(true);
        let answer = op(e.id(), e.revision()).await;
        e.busy.set_value(false);
        match answer {
            Ok(detail) => e.apply(detail),
            Err(err) if is_conflict(&err) => {
                e.s.tell(refused.to_string());
                reload(e);
            }
            Err(err) => e.s.tell(sermon_error(&err)),
        }
    });
}

fn reload(e: Editor) {
    leptos::task::spawn_local(async move {
        match sermons::get_sermon(&e.id()).await {
            Ok(detail) => {
                e.apply(detail);
                e.loaded.set(Loaded::Ready);
            }
            Err(err) if is_not_found(&err) => e.loaded.set(Loaded::Missing),
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
    let edits = drafted_edits(e, &item.id).unwrap_or_default();
    let id = item.id.clone();
    let sermon = e.id();
    write(
        e,
        "Položka sa medzitým zmenila — skús znova",
        move |sermon_id, revision| async move {
            let answer = sermons::delete_item(&sermon_id, &id, revision).await;
            if answer.is_ok() {
                e.s.tell_undo(
                    "Odstránené".to_string(),
                    Undo::Removed {
                        sermon,
                        item: Box::new(item),
                        position,
                        edits,
                    },
                );
            }
            answer
        },
    );
}

/// Späť on "Odstránené": the same item data at the same position.
pub(super) fn undo_removed(
    s: Study,
    sermon: String,
    item: SermonItem,
    position: usize,
    edits: ItemEdits,
) {
    leptos::task::spawn_local(async move {
        let passage = SermonPassage {
            translation: item.translation.clone(),
            book_code: item.book_code.clone(),
            chapter: item.chapter,
            verse_start: item.verse_start,
            verse_end: item.verse_end,
        };
        let done = async {
            let held = sermons::get_sermon(&sermon).await?;
            let added = sermons::add_item(&sermon, held.revision, Some(position), &passage).await?;
            let new_id = added
                .items
                .iter()
                .find(|row| !held.items.iter().any(|old| old.id == row.id))
                .map(|row| row.id.clone());
            match new_id.filter(|_| edits != ItemEdits::default()) {
                Some(new_id) => sermons::put_edits(&sermon, &new_id, added.revision, &edits).await,
                None => Ok(added),
            }
        };
        match done.await {
            // The open editor shows it through its reload.
            Ok(_) => s.retry.update(|n| *n = n.wrapping_add(1)),
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

/// `+ Pridať verš`: a reference is added here, words go to the text search,
/// whose hits add to this (active) sermon.
fn add_from_field(e: Editor, input: &str) -> bool {
    let Some(translation) = e
        .detail
        .with_untracked(|detail| detail.as_ref().map(|held| held.default_translation.clone()))
    else {
        return false;
    };
    let suggestion = e.s.structures.with_untracked(|map| {
        map.get(&translation)
            .map(|structure| (suggest(input, structure, &translation), structure.clone()))
    });
    let Some((suggestion, structure)) = suggestion else {
        e.s.tell("Zoznam kníh sa ešte načítava".to_string());
        return false;
    };
    if let Some(target) = suggestion.enter_target() {
        let verses = target.verses.or_else(|| {
            let last = structure.book(&target.book)?.verse_count(target.chapter)?;
            Some((1, last))
        });
        let Some((verse_start, verse_end)) = verses else {
            return false;
        };
        let passage = SermonPassage {
            translation,
            book_code: target.book,
            chapter: target.chapter,
            verse_start,
            verse_end,
        };
        write(
            e,
            "Kázeň sa medzitým zmenila — skús znova",
            move |id, revision| async move { sermons::add_item(&id, revision, None, &passage).await },
        );
        return true;
    }
    if let Some(query) = suggestion.text_search {
        super::add_to_sermon::set_active(e.s, Some(e.id()));
        nav::go(
            e.s,
            StudyRoute::search(&translation, &query),
            ScrollPlan::Top,
        );
        return true;
    }
    e.s.tell(
        suggestion
            .view
            .hint
            .unwrap_or_else(|| "Odkaz sa nepodarilo prečítať".to_string()),
    );
    false
}

#[component]
fn AddVerse(e: Editor) -> impl IntoView {
    let text = RwSignal::new(String::new());
    view! {
        <form
            class="sermon-add"
            on:submit=move |ev| {
                ev.prevent_default();
                if add_from_field(e, &text.get_untracked()) {
                    text.set(String::new());
                }
            }
        >
            <input
                class="study-input"
                data-role="sermon-add-field"
                type="text"
                placeholder="+ Pridať verš: gal 3 13-14"
                autocomplete="off"
                autocapitalize="off"
                prop:value=move || text.get()
                on:input=move |ev| text.set(event_target_value(&ev))
            />
        </form>
    }
}

#[component]
pub(super) fn SermonEditor(s: Study, id: String) -> impl IntoView {
    let e = Editor {
        s,
        id: StoredValue::new(id),
        loaded: RwSignal::new(Loaded::Waiting),
        detail: RwSignal::new(None),
        drafts: RwSignal::new(HashMap::new()),
        mode: RwSignal::new(TapMode::Strike),
        save: RwSignal::new(SaveState::Saved),
        dirty: StoredValue::new(BTreeSet::new()),
        busy: StoredValue::new(false),
        timer: StoredValue::new_local(None),
    };
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
    let online = window_event_listener(leptos::ev::online, move |_| flush(e));
    on_cleanup(move || online.remove());

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
                data-revision=move || e.revision().to_string()
            >
                <div class="sermon-editor__head">
                    <div>
                        <h2 class="sermon-section__title" data-role="sermon-date">{header}</h2>
                        <p class="sermon-editor__title" data-role="sermon-title">{title}</p>
                    </div>
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
            </section>
        </main>
    }
}
