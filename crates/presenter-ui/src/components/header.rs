use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Operator header component.
#[component]
pub fn Header(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let view_sig = ctx.view;
    let mode_sig = ctx.mode;

    let set_view = move |v: &str| {
        let v = v.to_string();
        view_sig.set(v.clone());
        crate::state::session::set("view", &v);
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("data-view", &v);
        }
    };

    let set_mode = move |m: &str| {
        let m = m.to_string();
        mode_sig.set(m.clone());
        crate::state::session::set("mode", &m);
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("data-mode", &m);
        }
    };

    let search_query = op.search_query;
    let mobile_nav = op.mobile_nav_open;

    view! {
        <header class="operator__header">
            <div class="operator__header-left">
                <h1>"Presenter"</h1>
                <span class="operator__version-badge"></span>
                <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off"
                    on:submit=move |ev| { ev.prevent_default(); }
                >
                    <span class="operator__search-icon" aria-hidden="true"></span>
                    <input
                        type="search"
                        placeholder="Search libraries, songs, slides"
                        data-role="global-search-query"
                        aria-label="Search presenter content"
                        autocomplete="off"
                        prop:value=move || search_query.get()
                        on:input=move |ev| {
                            let val = event_target_value(&ev);
                            search_query.set(val);
                        }
                    />
                    <button
                        type="button"
                        data-role="global-search-clear"
                        aria-label="Clear search"
                        on:click=move |_| { search_query.set(String::new()); }
                    >
                        <span aria-hidden="true">{"\u{00D7}"}</span>
                        <span class="sr-only">"Clear search"</span>
                    </button>
                </form>
            </div>
            <nav class="operator__view-nav">
                {["worship", "bible", "timers", "settings"].into_iter().map(|v| {
                    let v_str = v.to_string();
                    let v_clone = v_str.clone();
                    let label = match v {
                        "worship" => "Worship",
                        "bible" => "Bible",
                        "timers" => "Timers",
                        "settings" => "Settings",
                        _ => v,
                    };
                    view! {
                        <button
                            type="button"
                            data-role="view-toggle"
                            data-view={v_str.clone()}
                            data-active=move || if view_sig.get() == v_clone { "true" } else { "false" }
                            on:click={
                                let v = v_str.clone();
                                move |_| set_view(&v)
                            }
                        >{label}</button>
                    }
                }).collect::<Vec<_>>()}
            </nav>
            <div class="operator__header-right">
                <div class="operator__stage-layout" aria-label="Stage display mode">
                    <label class="operator__stage-layout-label" for="stage-layout-select">"Stage Output"</label>
                    <select id="stage-layout-select" data-role="stage-layout-select"></select>
                </div>
                <crate::components::stage_preview::StagePreview ctx=ctx.clone() />
                <div class="operator__mode-toggle">
                    {["live", "edit"].into_iter().map(|m| {
                        let m_str = m.to_string();
                        let m_clone = m_str.clone();
                        let label = match m {
                            "live" => "Live",
                            "edit" => "Edit",
                            _ => m,
                        };
                        view! {
                            <button
                                type="button"
                                data-role="mode-toggle"
                                data-mode={m_str.clone()}
                                data-active=move || if mode_sig.get() == m_clone { "true" } else { "false" }
                                on:click={
                                    let m = m_str.clone();
                                    move |_| set_mode(&m)
                                }
                            >{label}</button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
                <button
                    type="button"
                    class="operator__hamburger"
                    data-role="mobile-menu-toggle"
                    aria-label="Menu"
                    on:click=move |_| mobile_nav.update(|v| *v = !*v)
                >{"\u{2630}"}</button>
            </div>
        </header>
    }
}
