//! The back list (long press on ◀, spec §6.2): the places behind this one,
//! nearest first. A row is `history.go(-n)`.

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{nav, Study};

#[component]
pub(super) fn HistorySheet(s: Study) -> impl IntoView {
    let rows = move || s.history.with(|history| history.back_list());
    view! {
        <Sheet
            s=s
            role="history-list"
            title=Signal::derive(|| "Kde si bol".to_string())
            modal=true
        >
            <div class="study-list">
                <For
                    each=rows
                    key=|(steps, entry)| (*steps, entry.url.clone())
                    children=move |(steps, entry)| {
                        view! {
                            <button
                                type="button"
                                class="study-row"
                                data-role="history-entry"
                                on:click=move |_| nav::step_history(steps)
                            >
                                <span class="study-row__label">{entry.label.clone()}</span>
                            </button>
                        }
                    }
                />
            </div>
        </Sheet>
    }
}
