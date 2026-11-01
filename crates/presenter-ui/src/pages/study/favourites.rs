//! The five translation chips (spec §6.3): one tap switches and keeps the
//! place, a long press picks which translation sits in that slot, per device.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use super::long_press::LongPress;
use super::scroll::{self, ScrollPlan};
use super::sheet::Sheet;
use super::{nav, Study};
use crate::state::session;
use crate::state::study::{
    set_favourite, visible_favourites, StudySheet, StudyView, KEY_FAVOURITES,
};

fn switch(s: Study, code: &str) {
    let mut route = s.route.get_untracked().without_sheet();
    if route.translation.as_deref() == Some(code) {
        return;
    }
    route.translation = Some(code.to_string());
    let plan = match route.view {
        StudyView::Reader { .. } => {
            scroll::verse_in_view(s).map_or(ScrollPlan::Keep, ScrollPlan::Verse)
        }
        _ => ScrollPlan::Top,
    };
    nav::go(s, route, plan);
}

#[component]
pub(super) fn Favourites(s: Study) -> impl IntoView {
    let chips = move || {
        visible_favourites(&s.favourites.get(), &{
            s.translations.track();
            s.installed_codes()
        })
    };
    view! {
        <nav class="study-favs" data-role="study-favs" aria-label="Preklady">
            <For
                each=chips
                key=|chip| chip.clone()
                children=move |(slot, code)| view! { <FavouriteChip s=s index=slot code=code /> }
            />
        </nav>
    }
}

#[component]
fn FavouriteChip(s: Study, index: usize, code: String) -> impl IntoView {
    let press = LongPress::new();
    let open_picker = move || {
        nav::open_sheet(s, nav::sheet_route(s, StudySheet::Favourite(index)));
    };
    let label = translation_short_code(&code);
    let on_click = {
        let code = code.clone();
        move |_| {
            if !press.take_fired() {
                switch(s, &code);
            }
        }
    };
    let mine = code.clone();
    let pressed = Memo::new(move |_| {
        s.route
            .with(|route| route.translation.as_deref() == Some(mine.as_str()))
    });
    view! {
        <button
            type="button"
            class="study-fav"
            data-role="study-fav"
            data-translation=code
            aria-pressed=move || if pressed.get() { "true" } else { "false" }
            on:click=on_click
            on:pointerdown=move |ev| press.begin(&ev, open_picker)
            on:pointermove=move |ev| press.moved(&ev)
            on:pointerup=move |_| press.cancel()
            on:pointercancel=move |_| press.cancel()
            on:contextmenu=move |ev| press.context_menu(&ev, open_picker)
        >
            {label}
        </button>
    }
}

/// Every installed translation (MIL included) for one slot.
#[component]
pub(super) fn FavouriteSheet(s: Study, index: usize) -> impl IntoView {
    let held = move || {
        let chips = visible_favourites(&s.favourites.get(), &s.installed_codes());
        chips
            .into_iter()
            .find(|(chip, _)| *chip == index)
            .map(|(_, code)| code)
    };
    let rows = move || s.translations.get();
    view! {
        <Sheet
            s=s
            role="study-fav-sheet"
            title=Signal::derive(|| "Preklad na tomto mieste".to_string())
            modal=true
        >
            <div class="study-list">
                <For
                    each=rows
                    key=|translation| translation.code.clone()
                    children=move |translation| {
                        let code = translation.code.clone();
                        let picked = code.clone();
                        let on_pick = move |_| {
                            let slots = set_favourite(&s.favourites.get_untracked(), index, &picked);
                            session::set_persistent(KEY_FAVOURITES, &slots.join(","));
                            s.favourites.set(slots);
                            nav::close_sheet(s);
                        };
                        let current = code.clone();
                        view! {
                            <button
                                type="button"
                                class="study-row"
                                data-role="study-fav-option"
                                data-translation=code.clone()
                                aria-pressed=move || {
                                    if held().as_deref() == Some(current.as_str()) {
                                        "true"
                                    } else {
                                        "false"
                                    }
                                }
                                on:click=on_pick
                            >
                                <span class="study-row__label">
                                    {translation_short_code(&code)}
                                </span>
                                <small class="study-row__detail">{translation.name.clone()}</small>
                            </button>
                        }
                    }
                />
            </div>
        </Sheet>
    }
}
