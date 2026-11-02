//! "Pripravené kazateľom" (#851 H1, spec §8.1): today's sermons under the
//! field. A tap previews the item's first STORED slide, nothing goes to the
//! wall; ZOBRAZIŤ plays it through the stored-slide trigger, ĎALŠÍ and ◀ / ▶
//! walk the sermon in the prepared order. The list comes on mount, on every
//! `bible_presentations_version` bump (a sermon write publishes one) and when
//! the local date changes. Hidden while the field has text.

use std::time::Duration;

use leptos::prelude::*;

use super::{Console, Preview};
use crate::api::sermons;
use crate::state::bible_console_prepared::{
    prepared_sermons, PreparedPick, PreparedRow, PreparedSermon,
};
use crate::state::AppContext;

const DATE_CHECK: Duration = Duration::from_secs(60);

fn local_date() -> String {
    js_sys::Date::new_0()
        .to_date_string()
        .as_string()
        .unwrap_or_default()
}

/// Apply today's list: an older answer never replaces a newer one, and a pick
/// moves with its slide or is let go with its sermon.
pub(super) fn setup_prepared(c: Console) {
    let version = use_ctx!(AppContext).bible_presentations_version;
    let asked = StoredValue::new(0u64);
    let date = RwSignal::new(local_date());
    if let Ok(check) = set_interval_with_handle(
        move || {
            let today = local_date();
            if date.with_untracked(|held| *held != today) {
                date.set(today);
            }
        },
        DATE_CHECK,
    ) {
        on_cleanup(move || check.clear());
    }
    Effect::new(move |_| {
        version.track();
        date.track();
        let mine = asked.get_value().wrapping_add(1);
        asked.set_value(mine);
        leptos::task::spawn_local(async move {
            let answer = sermons::todays_sermons().await;
            if asked.try_get_value() != Some(mine) {
                return;
            }
            match answer {
                Ok(details) => {
                    let fresh = prepared_sermons(&details);
                    if let Some(pick) = c.prepared.get_untracked() {
                        c.prepared.set(moved_pick(c, &pick, &fresh));
                    }
                    c.sermons.set(fresh);
                }
                Err(err) => {
                    leptos::logging::warn!("todays sermons: {err}");
                }
            }
        });
    });
}

/// The pick in `fresh`, from where the operator is now (a show or a step may
/// have moved the preview past `pick.index`).
fn moved_pick(c: Console, pick: &PreparedPick, fresh: &[PreparedSermon]) -> Option<PreparedPick> {
    let index = c.preview.with_untracked(|p| match p {
        Preview::Ready { key, index, .. } if *key == pick.key() => *index,
        _ => pick.index,
    });
    PreparedPick {
        index,
        ..pick.clone()
    }
    .refreshed(fresh)
}

fn choose(c: Console, sermon: PreparedSermon, index: usize) {
    c.picked.set(None);
    c.after_show.set(false);
    c.at_end.set(false);
    c.input.set(String::new());
    let key = sermon.preview_key();
    let same = c.preview.with_untracked(|p| p.key() == Some(key.as_str()));
    c.prepared.set(Some(PreparedPick { sermon, index }));
    if same {
        c.preview.update(|p| {
            if let Preview::Ready { index: current, .. } = p {
                *current = index;
            }
        });
    }
}

#[component]
pub(super) fn PreparedSection(console: Console) -> impl IntoView {
    let c = console;
    let visible = move || c.input.with(String::is_empty) && c.sermons.with(|s| !s.is_empty());
    view! {
        <Show when=visible>
            <section class="console-prepared" data-role="console-prepared">
                <h3 class="console-prepared__heading">"Pripravené kazateľom"</h3>
                <For
                    each=move || c.sermons.get()
                    key=|sermon| sermon.preview_key()
                    children=move |sermon| view! { <PreparedGroup console=c sermon=sermon /> }
                />
            </section>
        </Show>
    }
}

#[component]
fn PreparedGroup(console: Console, sermon: PreparedSermon) -> impl IntoView {
    let c = console;
    let title = sermon.title.clone();
    let rows: Vec<PreparedRow> = sermon.rows.clone();
    view! {
        <p class="console-prepared__title" data-role="console-prepared-title">{title}</p>
        {rows
            .into_iter()
            .map(|row| view! { <PreparedButton console=c sermon=sermon.clone() row=row /> })
            .collect_view()}
    }
}

#[component]
fn PreparedButton(console: Console, sermon: PreparedSermon, row: PreparedRow) -> impl IntoView {
    let c = console;
    let first = row.first_slide;
    let last = sermon
        .rows
        .iter()
        .map(|other| other.first_slide)
        .find(|start| *start > first)
        .unwrap_or(sermon.slides.len());
    let key = sermon.preview_key();
    let selected = Memo::new(move |_| {
        c.preview.with(|p| match p {
            Preview::Ready {
                key: loaded, index, ..
            } => *loaded == key && (first..last).contains(index),
            _ => false,
        })
    });
    let on_tap = move |_| choose(c, sermon.clone(), first);
    view! {
        <button
            type="button"
            class="console-row"
            data-role="console-prepared-row"
            data-key=row.key.clone()
            aria-pressed=move || if selected.get() { "true" } else { "false" }
            on:click=on_tap
        >
            <span class="console-row__label">{row.label.clone()}</span>
            <small class="console-row__detail">{row.detail.clone()}</small>
        </button>
    }
}
