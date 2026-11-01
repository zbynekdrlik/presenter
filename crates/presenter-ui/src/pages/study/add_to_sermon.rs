//! Pridať do kázne (spec §7.6): a passage goes to the end of the device's
//! active sermon (`study.activeSermon`); with none, a sheet asks which one.
//! The page read the sermon just now, so the revision is fresh; another device
//! moving it in between is answered by one more try.

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{nav, sermon_error, snapshot_translation, Study, Undo};
use crate::api::sermons::{self, is_conflict, is_not_found, SermonDetail, SermonPassage};
use crate::api::ApiError;
use crate::state::sermon_list::{short_date, split_sermons};
use crate::state::session;
use crate::state::study::{StudySheet, KEY_ACTIVE_SERMON};

pub(super) fn set_active(s: Study, id: Option<String>) {
    match &id {
        Some(id) => session::set_persistent(KEY_ACTIVE_SERMON, id),
        None => session::remove_persistent(KEY_ACTIVE_SERMON),
    }
    s.active_sermon.set(id);
}

/// Add `passage`, asking where when no sermon is active.
pub(super) fn add(s: Study, passage: SermonPassage) {
    match s.active_sermon.get_untracked() {
        Some(id) => add_to(s, id, passage),
        None => ask_where(s, passage),
    }
}

fn ask_where(s: Study, passage: SermonPassage) {
    s.pending_passage.set(Some(passage));
    let route = nav::sheet_route(s, StudySheet::AddToSermon);
    if s.route.with_untracked(|route| route.sheet.is_some()) {
        nav::replace(s, route);
    } else {
        nav::open_sheet(s, route);
    }
}

async fn add_with_retry(
    id: &str,
    passage: &SermonPassage,
) -> Result<(SermonDetail, String), ApiError> {
    let before = sermons::get_sermon(id).await?;
    let answer = match sermons::add_item(id, before.revision, None, passage).await {
        Err(err) if is_conflict(&err) => {
            let fresh = sermons::get_sermon(id).await?;
            sermons::add_item(id, fresh.revision, None, passage).await?
        }
        other => other?,
    };
    let added = answer
        .items
        .last()
        .map(|item| item.id.clone())
        .unwrap_or_default();
    Ok((answer, added))
}

fn add_to(s: Study, id: String, passage: SermonPassage) {
    leptos::task::spawn_local(async move {
        match add_with_retry(&id, &passage).await {
            Ok((sermon, item)) => {
                s.tell_undo(
                    format!("Pridané do kázne „{}“", sermon.title),
                    Undo::Added { sermon: id, item },
                );
                if s.route.with_untracked(|route| route.sheet.is_some()) {
                    nav::close_sheet(s);
                }
            }
            // The sermon was deleted elsewhere: this device must choose again.
            Err(err) if is_not_found(&err) => {
                set_active(s, None);
                ask_where(s, passage);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

/// Späť on the toast: the added item goes again.
pub(super) fn undo_added(s: Study, sermon: String, item: String) {
    leptos::task::spawn_local(async move {
        let done = async {
            let held = sermons::get_sermon(&sermon).await?;
            sermons::delete_item(&sermon, &item, held.revision).await
        };
        match done.await {
            Ok(_) => s.tell("Odstránené z kázne".to_string()),
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
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
        leptos::task::spawn_local(async move {
            match sermons::create_sermon(None, None, snapshot_translation(&passage.translation))
                .await
            {
                Ok(sermon) => {
                    set_active(s, Some(sermon.id.clone()));
                    add_to(s, sermon.id, passage);
                }
                Err(err) => s.tell(sermon_error(&err)),
            }
        });
    };
    view! {
        <Sheet s=s role="add-to-sermon" title=Signal::derive(|| "Pridať do kázne".to_string())>
            {move || {
                list.get()
                    .map(|answer| match answer {
                        Ok(answer) => {
                            let (upcoming, _) = split_sermons(&answer.sermons, answer.today);
                            let new_label = format!(
                                "Nová kázeň ({})",
                                short_date(answer.default_service_date),
                            );
                            view! {
                                <div class="study-list">
                                    {upcoming
                                        .into_iter()
                                        .map(|sermon| {
                                            let id = sermon.id.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class="study-row"
                                                    data-role="add-to-sermon-row"
                                                    data-sermon-id=sermon.id.clone()
                                                    on:click=move |_| choose(id.clone())
                                                >
                                                    <span class="study-row__label">{sermon.title.clone()}</span>
                                                    <span class="study-row__detail">
                                                        {short_date(sermon.service_date)}
                                                    </span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                    <button
                                        type="button"
                                        class="study-button study-button--primary"
                                        data-role="add-to-new-sermon"
                                        on:click=on_new
                                    >
                                        {new_label}
                                    </button>
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
