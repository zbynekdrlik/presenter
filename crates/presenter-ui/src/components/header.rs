use leptos::prelude::*;

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
                <form data-role="global-search-form" on:submit=on_search_submit class="operator__search-form">
                    <input
                        type="text"
                        data-role="global-search-query"
                        class="operator__search-input"
                        placeholder="Search..."
                        prop:value=move || op.search_query.get()
                        on:input=on_search_input
                    />
                    <button
                        type="button"
                        data-role="global-search-clear"
                        class="operator__search-clear"
                        on:click=on_search_clear
                    >
                        "\u{00d7}"
                    </button>
                </form>
            </div>
            <nav class="operator__header-center">
                <div class="operator__view-toggles">
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
                                data-role="view-toggle"
                                data-view=view_name.clone()
                                attr:data-active=move || if ctx.view.get() == vn { "true" } else { "false" }
                                class="operator__view-btn"
                                on:click={
                                    let vn2 = view_name.clone();
                                    move |_| set_view(&vn2)
                                }
                            >
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>
            </nav>
            <div class="operator__header-right">
                <select
                    data-role="stage-layout-select"
                    class="operator__layout-select"
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
                <div class="operator__mode-toggles">
                    {["live", "edit"].into_iter().map(|m| {
                        let mode_name = m.to_string();
                        let label = match m { "live" => "Live", "edit" => "Edit", _ => m };
                        let mn = mode_name.clone();
                        view! {
                            <button
                                data-role="mode-toggle"
                                data-mode=mode_name.clone()
                                attr:data-active=move || if ctx.mode.get() == mn { "true" } else { "false" }
                                class="operator__mode-btn"
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
                    data-role="mobile-menu-toggle"
                    class="operator__mobile-menu-btn"
                    on:click=on_mobile_toggle
                >
                    "\u{2630}"
                </button>
            </div>
        </header>
    }
}
