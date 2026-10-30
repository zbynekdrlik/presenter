//! Phone tab bar (#850 spec §6.2): the sections at the bottom of the
//! phone shell (a 64 px left rail in landscape, `phone_shell.css`). It
//! replaces the burger menu, whose overlay never closed (2026-10-12).

use leptos::prelude::*;

use crate::state::view_nav::{set_view, VIEWS};
use crate::state::AppContext;

#[component]
pub fn PhoneTabBar() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let view = ctx.view;
    let phone_shell = ctx.phone_shell;
    view! {
        <Show when=move || phone_shell.get()>
            <nav class="phone-tabs" data-role="phone-tab-bar" aria-label="Sekcie">
                {VIEWS
                    .into_iter()
                    .map(|(id, label)| {
                        view! {
                            <button
                                type="button"
                                class="phone-tabs__tab"
                                data-role="phone-tab"
                                data-view=id
                                aria-current=move || (view.get() == id).then_some("page")
                                on:click=move |_| set_view(view, id)
                            >
                                {label}
                            </button>
                        }
                    })
                    .collect_view()}
            </nav>
        </Show>
    }
}
