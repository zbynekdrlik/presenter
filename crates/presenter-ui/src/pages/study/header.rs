//! The 48 px header: ◀ ▶ step through the history, a long press on ◀ opens the
//! list, Aa the text size, Kázne the sermon list. The sermon views add Biblia,
//! back to the last reading place: a tab reopened on a sermon has no history,
//! and the field is not shown there (owner, 2026-11-06: "už sa neviem vrátiť k
//! veršom").

use leptos::prelude::*;

use super::long_press::LongPress;
use super::scroll::{saved_verse, ScrollPlan};
use super::sermon_pane::reading_route;
use super::{nav, Study};
use crate::state::study::{StudyRoute, StudySheet, StudyView};

#[component]
pub(super) fn StudyHeader(s: Study) -> impl IntoView {
    let press = LongPress::new();
    let can_back = Memo::new(move |_| s.history.with(|history| history.can_go_back()));
    let can_forward = Memo::new(move |_| s.history.with(|history| history.can_go_forward()));
    let open_list = move || nav::open_sheet(s, nav::sheet_route(s, StudySheet::History));

    let on_back = move |_| {
        if !press.take_fired() && can_back.get_untracked() {
            nav::back();
        }
    };
    let on_forward = move |_| {
        let steps = s.history.with_untracked(|history| history.forward_steps());
        if let Some(steps) = steps {
            nav::step_history(steps);
        }
    };
    let on_sermons = move |_| {
        let translation = s.route.with_untracked(|route| route.translation.clone());
        nav::go(s, StudyRoute::sermons(translation), ScrollPlan::Top);
    };
    let on_sermon_view = Memo::new(move |_| {
        s.route
            .with(|route| matches!(route.view, StudyView::Sermons | StudyView::Sermon { .. }))
    });
    let on_bible = move |_| {
        let mut route = reading_route();
        // No place stored yet: the start route names no translation of its own.
        if route.translation.is_none() {
            route.translation = s.route.with_untracked(|shown| shown.translation.clone());
        }
        let plan = saved_verse(&route).map_or(ScrollPlan::Top, ScrollPlan::Verse);
        nav::go(s, route, plan);
    };
    let on_press = move |ev: web_sys::PointerEvent| {
        if can_back.get_untracked() {
            press.begin(&ev, open_list);
        }
    };

    view! {
        <header class="study-head">
            <h1 class="study-head__title">"Štúdium"</h1>
            <div class="study-head__actions">
                <Show when=move || on_sermon_view.get()>
                    <button
                        type="button"
                        class="study-icon study-icon--text study-icon--wide"
                        data-role="study-bible"
                        on:click=on_bible
                    >
                        "Biblia"
                    </button>
                </Show>
                <button
                    type="button"
                    class="study-icon study-icon--text study-icon--wide"
                    data-role="study-sermons"
                    on:click=on_sermons
                >
                    "Kázne"
                </button>
                <button
                    type="button"
                    class="study-icon"
                    data-role="history-back"
                    aria-label="Späť, podržaním zoznam"
                    aria-disabled=move || if can_back.get() { "false" } else { "true" }
                    on:click=on_back
                    on:pointerdown=on_press
                    on:pointermove=move |ev| press.moved(&ev)
                    on:pointerup=move |_| press.cancel()
                    on:pointercancel=move |_| press.cancel()
                    on:contextmenu=move |ev| {
                        press
                            .context_menu(
                                &ev,
                                || {
                                    if can_back.get_untracked() {
                                        open_list();
                                    }
                                },
                            )
                    }
                >
                    "\u{25C0}"
                </button>
                <button
                    type="button"
                    class="study-icon"
                    data-role="history-forward"
                    aria-label="Dopredu"
                    disabled=move || !can_forward.get()
                    on:click=on_forward
                >
                    "\u{25B6}"
                </button>
                <button
                    type="button"
                    class="study-icon study-icon--text"
                    data-role="study-text-size"
                    aria-label="Veľkosť písma"
                    on:click=move |_| {
                        nav::open_sheet(s, nav::sheet_route(s, StudySheet::TextSize))
                    }
                >
                    "Aa"
                </button>
            </div>
        </header>
    }
}
