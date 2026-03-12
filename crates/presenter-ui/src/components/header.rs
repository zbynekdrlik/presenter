use leptos::prelude::*;

use crate::components::stage_preview::StagePreview;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn Header() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let op = use_context::<OperatorState>().expect("OperatorState");

    // Search form handlers
    let on_search_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
    };

    let on_search_input = move |ev| {
        let val: String = event_target_value(&ev);
        op.search_query.set(val.clone());
        op.search_open.set(!val.is_empty());
    };

    let on_search_clear = move |_| {
        op.search_query.set(String::new());
        op.search_open.set(false);
        ctx.search_results.set(Vec::new());
    };

    // View toggle
    let set_view = move |view: &str| {
        let v = view.to_string();
        ctx.view.set(v.clone());
        crate::state::session::set("view", &v);
    };

    // Mode toggle
    let set_mode = move |mode: &str| {
        let m = mode.to_string();
        ctx.mode.set(m.clone());
        crate::state::session::set("mode", &m);
    };

    // Stage layout change
    let on_layout_change = move |ev| {
        let code: String = event_target_value(&ev);
        let code_clone = code.clone();
        ctx.stage_layout_code.set(code.clone());
        leptos::task::spawn_local(async move {
            let _ = crate::api::stage::set_layout(&code_clone).await;
        });
    };

    // Mobile menu toggle
    let on_mobile_toggle = move |_| {
        op.mobile_nav_open.update(|v| *v = !*v);
    };

    view! {
        <header class="operator__header">
            <div class="operator__header-left">
                <h1>"Presenter"</h1>
                <span class="operator__version-badge"></span>
                <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off"
                    on:submit=on_search_submit
                >
                    <span class="operator__search-icon" aria-hidden="true"></span>
                    <input
                        type="search"
                        placeholder="Search libraries, songs, slides"
                        data-role="global-search-query"
                        aria-label="Search presenter content"
                        autocomplete="off"
                        prop:value=move || op.search_query.get()
                        on:input=on_search_input
                    />
                    <button
                        type="button"
                        data-role="global-search-clear"
                        aria-label="Clear search"
                        on:click=on_search_clear
                    >
                        <span aria-hidden="true">{"\u{00d7}"}</span>
                        <span class="sr-only">"Clear search"</span>
                    </button>
                </form>
            </div>
            <nav class="operator__view-nav">
                {["worship", "bible", "timers", "settings"].into_iter().map(|v| {
                    let view_name = v.to_string();
                    let label = match v {
                        "worship" => "Worship",
                        "bible" => "Bible",
                        "timers" => "Timers",
                        "settings" => "Settings",
                        _ => v,
                    };
                    let vn = view_name.clone();
                    view! {
                        <button
                            type="button"
                            data-role="view-toggle"
                            data-view=view_name.clone()
                            attr:data-active=move || if ctx.view.get() == vn { "true" } else { "false" }
                            on:click={
                                let vn2 = view_name.clone();
                                move |_| set_view(&vn2)
                            }
                        >
                            {label}
                        </button>
                    }
                }).collect_view()}
            </nav>
            <div class="operator__header-right">
                <div class="operator__stage-layout" aria-label="Stage display mode">
                    <label class="operator__stage-layout-label" for="stage-layout-select">"Stage Output"</label>
                    <select
                        id="stage-layout-select"
                        data-role="stage-layout-select"
                        on:change=on_layout_change
                    >
                        {move || ctx.stage_layouts.get().into_iter().map(|layout| {
                            let code = layout.code.clone();
                            let name = layout.name.clone();
                            let selected = ctx.stage_layout_code.get() == code;
                            view! {
                                <option value=code prop:selected=selected>{name}</option>
                            }
                        }).collect_view()}
                    </select>
                </div>
                <StagePreview />
                <div class="operator__mode-toggle">
                    {["live", "edit"].into_iter().map(|m| {
                        let mode_name = m.to_string();
                        let label = match m { "live" => "Live", "edit" => "Edit", _ => m };
                        let mn = mode_name.clone();
                        view! {
                            <button
                                type="button"
                                data-role="mode-toggle"
                                data-mode=mode_name.clone()
                                attr:data-active=move || if ctx.mode.get() == mn { "true" } else { "false" }
                                on:click={
                                    let mn2 = mode_name.clone();
                                    move |_| set_mode(&mn2)
                                }
                            >
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>
                <button
                    type="button"
                    class="operator__hamburger"
                    data-role="mobile-menu-toggle"
                    aria-label="Menu"
                    on:click=on_mobile_toggle
                >
                    "\u{2630}"
                </button>
            </div>
        </header>
    }
}
