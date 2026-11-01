//! The sermon list (spec §7.6): upcoming and older sermons, a new one, delete.

use chrono::NaiveDate;
use leptos::prelude::*;

use super::add_to_sermon::set_active;
use super::scroll::ScrollPlan;
use super::{nav, sermon_error, snapshot_translation, Study};
use crate::api::sermons::{self, SermonList, SermonSummary};
use crate::state::sermon_list::{count_text, only_mine, short_date, split_sermons};
use crate::state::study::StudyRoute;

#[derive(Clone)]
enum Loaded {
    Waiting,
    Ready(SermonList),
    Failed(String),
}

fn fetch(list: RwSignal<Loaded>) {
    leptos::task::spawn_local(async move {
        list.set(match sermons::list_sermons().await {
            Ok(answer) => Loaded::Ready(answer),
            Err(err) => Loaded::Failed(sermon_error(&err)),
        });
    });
}

fn open_sermon(s: Study, id: &str) {
    let translation = s.route.with_untracked(|route| route.translation.clone());
    nav::go(s, StudyRoute::sermon(translation, id), ScrollPlan::Top);
}

fn delete_sermon(s: Study, list: RwSignal<Loaded>, sermon: &SermonSummary) {
    let question = format!("Zmazať kázeň „{}“?", sermon.title);
    let confirmed = crate::utils::window::window()
        .confirm_with_message(&question)
        .unwrap_or(false);
    if !confirmed {
        return;
    }
    let id = sermon.id.clone();
    leptos::task::spawn_local(async move {
        match sermons::delete_sermon(&id).await {
            Ok(()) => {
                if s.active_sermon.get_untracked().as_deref() == Some(&id) {
                    set_active(s, None);
                }
                s.tell("Kázeň je zmazaná".to_string());
                fetch(list);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

#[component]
fn SermonRow(s: Study, sermon: SermonSummary, list: RwSignal<Loaded>) -> impl IntoView {
    let id = sermon.id.clone();
    let for_delete = sermon.clone();
    let active = {
        let id = sermon.id.clone();
        move || {
            s.active_sermon
                .with(|active| active.as_deref() == Some(id.as_str()))
        }
    };
    let detail = match &sermon.author_email {
        Some(author) => format!(
            "{} · {author}",
            count_text(sermon.item_count, sermon.slide_count)
        ),
        None => count_text(sermon.item_count, sermon.slide_count),
    };
    view! {
        <div
            class="sermon-row"
            data-role="sermon-row"
            data-sermon-id=sermon.id.clone()
            data-date=sermon.service_date.to_string()
            data-active=move || if active() { "true" } else { "false" }
        >
            <button
                type="button"
                class="sermon-row__open"
                data-role="sermon-open"
                on:click=move |_| open_sermon(s, &id)
            >
                <span class="sermon-row__date">{short_date(sermon.service_date)}</span>
                <span class="sermon-row__title">{sermon.title.clone()}</span>
                <span class="sermon-row__detail">{detail}</span>
            </button>
            <button
                type="button"
                class="study-icon"
                data-role="sermon-delete"
                aria-label="Zmazať kázeň"
                on:click=move |_| delete_sermon(s, list, &for_delete)
            >
                "\u{1F5D1}"
            </button>
        </div>
    }
}

#[component]
fn NewSermon(s: Study, answer: SermonList) -> impl IntoView {
    let date_text = RwSignal::new(answer.default_service_date.to_string());
    let title = RwSignal::new(String::new());
    let default_title =
        presenter_core::bible::sermon::default_sermon_title(answer.default_service_date);
    let hint = default_title.clone();
    let sending = RwSignal::new(false);
    let create = move || {
        if sending.get_untracked() {
            return;
        }
        sending.set(true);
        let date = date_text.get_untracked().parse::<NaiveDate>().ok();
        let typed = title.get_untracked();
        leptos::task::spawn_local(async move {
            let typed = Some(typed.trim().to_string()).filter(|text| !text.is_empty());
            // A date the preacher moved must not keep the default's date in its title.
            let name =
                typed.or_else(|| date.map(presenter_core::bible::sermon::default_sermon_title));
            let reading = s.route.with_untracked(|route| route.translation.clone());
            let default_translation = reading.as_deref().and_then(snapshot_translation);
            match sermons::create_sermon(name.as_deref(), date, default_translation).await {
                Ok(sermon) => {
                    set_active(s, Some(sermon.id.clone()));
                    open_sermon(s, &sermon.id);
                }
                Err(err) => {
                    sending.set(false);
                    s.tell(sermon_error(&err));
                }
            }
        });
    };
    view! {
        <form
            class="sermon-new"
            data-role="sermon-new"
            on:submit=move |ev| {
                ev.prevent_default();
                create();
            }
        >
            <label class="sermon-new__field">
                "Dátum"
                <input
                    class="study-input"
                    data-role="sermon-new-date"
                    type="date"
                    prop:value=move || date_text.get()
                    on:input=move |ev| date_text.set(event_target_value(&ev))
                />
            </label>
            <label class="sermon-new__field">
                "Názov"
                <input
                    class="study-input"
                    data-role="sermon-new-title"
                    type="text"
                    placeholder=hint
                    prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))
                />
            </label>
            <button
                type="submit"
                class="study-button study-button--primary"
                data-role="sermon-new-create"
            >
                "Vytvoriť"
            </button>
        </form>
    }
}

#[component]
fn Section(
    s: Study,
    title: &'static str,
    role: &'static str,
    rows: Vec<SermonSummary>,
    list: RwSignal<Loaded>,
) -> impl IntoView {
    (!rows.is_empty()).then(|| {
        view! {
            <section class="sermon-section" data-role=role>
                <h2 class="sermon-section__title">{title}</h2>
                {rows
                    .into_iter()
                    .map(|sermon| view! { <SermonRow s=s sermon=sermon list=list /> })
                    .collect_view()}
            </section>
        }
    })
}

#[component]
pub(super) fn SermonListView(s: Study) -> impl IntoView {
    let list = RwSignal::new(Loaded::Waiting);
    let creating = RwSignal::new(false);
    let mine = RwSignal::new(false);
    let email = RwSignal::new(None);
    leptos::task::spawn_local(async move {
        // Without the identity, "Moje" just is not offered.
        email.set(sermons::identity().await.ok().flatten());
    });
    let known_email = move || email.get();
    Effect::new(move |_| {
        s.retry.track();
        fetch(list);
    });
    let state = move || {
        list.with(|loaded| match loaded {
            Loaded::Waiting => "loading",
            Loaded::Ready(_) => "ready",
            Loaded::Failed(_) => "error",
        })
    };
    let body = move || match list.get() {
        Loaded::Waiting => ().into_any(),
        Loaded::Failed(message) => view! {
            <div class="study-error" data-role="sermon-list-error" role="alert">
                <p>{message}</p>
                <button
                    type="button"
                    class="study-button"
                    data-role="study-retry"
                    on:click=move |_| s.retry.update(|n| *n = n.wrapping_add(1))
                >
                    "Skúsiť znova"
                </button>
            </div>
        }
        .into_any(),
        Loaded::Ready(answer) => {
            let rows = match known_email().filter(|_| mine.get()) {
                Some(address) => only_mine(&answer.sermons, &address),
                None => answer.sermons.clone(),
            };
            let (upcoming, older) = split_sermons(&rows, answer.today);
            let empty = upcoming.is_empty() && older.is_empty();
            view! {
                {creating.get().then(|| view! { <NewSermon s=s answer=answer.clone() /> })}
                {empty
                    .then(|| {
                        view! {
                            <p class="study-empty" data-role="sermon-list-empty">
                                "Zatiaľ žiadne kázne"
                            </p>
                        }
                    })}
                <Section s=s title="Najbližšie" role="sermons-upcoming" rows=upcoming list=list />
                <Section s=s title="Staršie" role="sermons-older" rows=older list=list />
            }
            .into_any()
        }
    };
    view! {
        <main class="study-scroll study-scroll--results">
            <section class="sermon-list" data-role="sermon-list" data-state=state>
                <div class="sermon-list__head">
                    <button
                        type="button"
                        class="study-button"
                        data-role="sermon-new-open"
                        on:click=move |_| creating.update(|open| *open = !*open)
                    >
                        "+ Nová kázeň"
                    </button>
                    {move || {
                        known_email()
                            .map(|_| {
                                view! {
                                    <button
                                        type="button"
                                        class="study-button"
                                        data-role="sermons-mine"
                                        aria-pressed=move || if mine.get() { "true" } else { "false" }
                                        on:click=move |_| mine.update(|on| *on = !*on)
                                    >
                                        "Moje"
                                    </button>
                                }
                            })
                    }}
                </div>
                {body}
            </section>
        </main>
    }
}
