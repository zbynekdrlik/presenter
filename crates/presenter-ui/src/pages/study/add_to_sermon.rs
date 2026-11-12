//! Pridať do kázne (spec §7.6): a passage goes to the end of the sermon the
//! preacher picks in the sheet, EVERY time (#936, owner 2026-11-12: "Musí mi
//! vždy dať na výber a ponúknuť vytvoriť novú") — "Nová kázeň" first, then the
//! sermons by the newest service date. `study.activeSermon` only marks the last
//! one used ("naposledy"), it never picks. The page read the sermon just now,
//! so the revision is fresh; another device moving it in between is answered
//! by one more try.

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{nav, sermon_error, snapshot_translation, Study, Undo};
use crate::api::sermons::{self, is_conflict, is_not_found, SermonDetail, SermonPassage};
use crate::api::ApiError;
use crate::state::sermon_list::{add_choices, short_date};
use crate::state::session;
use crate::state::study::{verse_reference, StudySheet, KEY_ACTIVE_SERMON};

pub(super) fn set_active(s: Study, id: Option<String>) {
    match &id {
        Some(id) => session::set_persistent(KEY_ACTIVE_SERMON, id),
        None => session::remove_persistent(KEY_ACTIVE_SERMON),
    }
    s.active_sermon.set(id);
}

/// Add `passage`: the sheet always asks which sermon (#936).
pub(super) fn add(s: Study, passage: SermonPassage) {
    if snapshot_translation(&passage.translation).is_none() {
        s.tell("Z prekladu NLT sa do kázne pridať nedá — prepni preklad".to_string());
        return;
    }
    ask_where(s, passage);
}

fn ask_where(s: Study, passage: SermonPassage) {
    s.pending_passage.set(Some(passage));
    let open = s.route.with_untracked(|route| route.sheet);
    // The cross-references list comes back after the add: a preacher adds
    // several references from it.
    if open == Some(StudySheet::CrossRefs) {
        s.add_return.set_value(open);
    } else if open != Some(StudySheet::AddToSermon) {
        s.add_return.set_value(None);
    }
    let route = nav::sheet_route(s, StudySheet::AddToSermon);
    if open.is_some() {
        nav::replace(s, route);
    } else {
        nav::open_sheet(s, route);
    }
}

/// "1 Samuelova 4:1-11 (SEB)": the passage as its translation names the book.
fn passage_label(s: Study, passage: &SermonPassage) -> String {
    let book = s
        .structures
        .with(|map| {
            map.get(&passage.translation)
                .and_then(|structure| structure.book(&passage.book_code))
                .map(|book| book.name.clone())
        })
        .unwrap_or_else(|| passage.book_code.clone());
    verse_reference(
        &book,
        passage.chapter,
        (passage.verse_start, passage.verse_end),
        &passage.translation,
    )
}

/// The sermon's answer and the new item's id. `position` is kept inside the
/// list: another device may have shortened it.
pub(super) async fn add_with_retry(
    id: &str,
    position: Option<usize>,
    passage: &SermonPassage,
) -> Result<(SermonDetail, String), ApiError> {
    let at = |held: &SermonDetail| position.map(|wanted| wanted.min(held.items.len()));
    let before = sermons::get_sermon(id).await?;
    let (before, answer) = match sermons::add_item(id, before.revision, at(&before), passage).await
    {
        Err(err) if is_conflict(&err) => {
            let fresh = sermons::get_sermon(id).await?;
            let answer = sermons::add_item(id, fresh.revision, at(&fresh), passage).await?;
            (fresh, answer)
        }
        other => (before, other?),
    };
    let added = answer
        .items
        .iter()
        .find(|item| !before.items.iter().any(|held| held.id == item.id))
        .map(|item| item.id.clone())
        .unwrap_or_default();
    Ok((answer, added))
}

/// False while this passage is on its way: a double tap adds it once.
pub(super) fn begin_add(s: Study, passage: &SermonPassage) -> bool {
    if s.adding.with_value(|held| held.as_ref() == Some(passage)) {
        return false;
    }
    s.adding.set_value(Some(passage.clone()));
    true
}

async fn add_now(s: Study, id: String, passage: SermonPassage) {
    match add_with_retry(&id, None, &passage).await {
        Ok((sermon, item)) => {
            s.tell_undo(
                format!("Pridané do kázne „{}“", sermon.title),
                Undo::Added { sermon: id, item },
            );
            let sheet = s.route.with_untracked(|route| route.sheet);
            if sheet == Some(StudySheet::AddToSermon) {
                match s.add_return.try_update_value(Option::take).flatten() {
                    Some(back) => nav::replace(s, nav::sheet_route(s, back)),
                    None => nav::close_sheet(s),
                }
            }
        }
        // The sermon was deleted elsewhere: this device must choose again.
        Err(err) if is_not_found(&err) => {
            set_active(s, None);
            ask_where(s, passage);
        }
        Err(err) => s.tell(sermon_error(&err)),
    }
}

fn add_to(s: Study, id: String, passage: SermonPassage) {
    if begin_add(s, &passage) {
        leptos::task::spawn_local(async move {
            add_now(s, id, passage).await;
            s.adding.set_value(None);
        });
    }
}

#[component]
pub(super) fn AddToSermonSheet(s: Study) -> impl IntoView {
    let list = RwSignal::new(None);
    leptos::task::spawn_local(async move {
        let answer = sermons::list_sermons().await;
        list.set(Some(answer.map_err(|err| sermon_error(&err))));
    });
    let choose = move |id: String| {
        if let Some(passage) = s.pending_passage.get_untracked() {
            set_active(s, Some(id.clone()));
            add_to(s, id, passage);
        }
    };
    let on_new = move |_| {
        let Some(passage) = s.pending_passage.get_untracked() else {
            return;
        };
        if !begin_add(s, &passage) {
            return;
        }
        leptos::task::spawn_local(async move {
            match sermons::create_sermon(None, None, snapshot_translation(&passage.translation))
                .await
            {
                Ok(sermon) => {
                    set_active(s, Some(sermon.id.clone()));
                    add_now(s, sermon.id, passage).await;
                }
                Err(err) => s.tell(sermon_error(&err)),
            }
            s.adding.set_value(None);
        });
    };
    let passage = move || {
        s.pending_passage
            .with(|held| held.as_ref().map(|passage| passage_label(s, passage)))
            .unwrap_or_default()
    };
    let last = move |id: &str| s.active_sermon.with(|active| active.as_deref() == Some(id));
    view! {
        <Sheet s=s role="add-to-sermon" title=Signal::derive(|| "Pridať do kázne".to_string())>
            <p class="study-add__passage" data-role="add-to-sermon-passage">{passage}</p>
            {move || {
                list.get()
                    .map(|answer| match answer {
                        Ok(answer) => {
                            let choices = add_choices(&answer.sermons, answer.today);
                            let new_label = format!(
                                "+ Nová kázeň ({})",
                                short_date(answer.default_service_date),
                            );
                            view! {
                                <div class="study-list">
                                    <button
                                        type="button"
                                        class="study-button study-button--primary"
                                        data-role="add-to-new-sermon"
                                        on:click=on_new
                                    >
                                        {new_label}
                                    </button>
                                    {choices
                                        .into_iter()
                                        .map(|sermon| {
                                            let id = sermon.id.clone();
                                            let detail = if last(&sermon.id) {
                                                format!("{} · naposledy", short_date(sermon.service_date))
                                            } else {
                                                short_date(sermon.service_date)
                                            };
                                            view! {
                                                <button
                                                    type="button"
                                                    class="study-row"
                                                    data-role="add-to-sermon-row"
                                                    data-sermon-id=sermon.id.clone()
                                                    on:click=move |_| choose(id.clone())
                                                >
                                                    <span class="study-row__label">{sermon.title.clone()}</span>
                                                    <span class="study-row__detail">{detail}</span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }
                        Err(message) => {
                            view! {
                                <p class="study-empty" data-role="add-to-sermon-error" role="alert">
                                    {message}
                                </p>
                            }
                                .into_any()
                        }
                    })
            }}
        </Sheet>
    }
}
