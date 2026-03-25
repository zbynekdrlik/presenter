use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::api::bible as bible_api;
use crate::components::stage_preview::StagePreview;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn Header() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    // Debounce timer for bible search
    let bible_timer: Rc<RefCell<Option<gloo_timers::callback::Timeout>>> =
        Rc::new(RefCell::new(None));

    // Search form handlers
    let on_search_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
    };

    let on_search_input = {
        let bible_timer = Rc::clone(&bible_timer);
        move |ev| {
            let val: String = event_target_value(&ev);
            let is_bible = ctx.view.get_untracked() == "bible";

            if is_bible {
                ctx.bible_search_query.set(val.clone());
                op.search_open.set(!val.is_empty());

                // Cancel pending bible timer
                bible_timer.borrow_mut().take();

                if val.len() < 3 {
                    ctx.bible_search_results.set(Vec::new());
                    ctx.bible_has_searched.set(false);
                    ctx.bible_searching.set(false);
                    return;
                }

                ctx.bible_searching.set(true);

                let search_results = ctx.bible_search_results;
                let searching = ctx.bible_searching;
                let has_searched = ctx.bible_has_searched;

                // Get the bible translation from context if BibleState is available
                let translation = leptos::prelude::use_context::<crate::state::bible::BibleState>()
                    .and_then(|bs| bs.selected_translation.get_untracked())
                    .unwrap_or_default();

                let timer = gloo_timers::callback::Timeout::new(300, move || {
                    leptos::task::spawn_local(async move {
                        match bible_api::search(&val, &translation, Some(20)).await {
                            Ok(hits) => {
                                search_results.set(hits);
                                has_searched.set(true);
                            }
                            Err(_) => {
                                search_results.set(Vec::new());
                                has_searched.set(true);
                            }
                        }
                        searching.set(false);
                    });
                });
                *bible_timer.borrow_mut() = Some(timer);
            } else {
                op.search_query.set(val.clone());
                op.search_open.set(!val.is_empty());
            }
        }
    };

    let on_search_clear = move |_| {
        op.search_query.set(String::new());
        op.search_open.set(false);
        ctx.search_results.set(Vec::new());
        // Also clear bible search
        ctx.bible_search_query.set(String::new());
        ctx.bible_search_results.set(Vec::new());
        ctx.bible_has_searched.set(false);
        ctx.bible_searching.set(false);
    };

    // View toggle — updates URL pathname so browser back/forward works
    let set_view = move |view: &str| {
        let v = view.to_string();
        ctx.view.set(v.clone());
        crate::state::session::set("view", &v);
        // Build URL path: "worship" → /ui/operator, others → /ui/operator/{view}
        let url = if v == "worship" {
            "/ui/operator".to_string()
        } else {
            format!("/ui/operator/{v}")
        };
        let window = crate::utils::window::window();
        let state = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&state, &"view".into(), &v.clone().into());
        let _ = window
            .history()
            .and_then(|h| h.push_state_with_url(&state, "", Some(&url)));
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

    // Fetch version from /healthz
    let version_text = RwSignal::new(String::new());
    {
        leptos::task::spawn_local(async move {
            if let Ok(health) =
                crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await
            {
                let text = if health.channel.is_empty() || health.channel == "release" {
                    format!("v{}", health.version)
                } else {
                    format!("v{} ({})", health.version, health.channel)
                };
                version_text.set(text);
            }
        });
    }

    view! {
        <header class="operator__header">
            <div class="operator__header-left">
                <h1>"Presenter"</h1>
                <span class="operator__version-badge">{move || version_text.get()}</span>
                <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off"
                    on:submit=on_search_submit
                >
                    <span class="operator__search-icon" aria-hidden="true"></span>
                    <input
                        type="search"
                        placeholder=move || {
                            if ctx.view.get() == "bible" { "Search Bible verses" } else { "Search libraries, songs, slides" }
                        }
                        data-role="global-search-query"
                        aria-label="Search presenter content"
                        autocomplete="off"
                        prop:value=move || {
                            if ctx.view.get() == "bible" {
                                ctx.bible_search_query.get()
                            } else {
                                op.search_query.get()
                            }
                        }
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
                {["worship", "bible", "timers", "ai", "settings"].into_iter().map(|v| {
                    let view_name = v.to_string();
                    let label = match v {
                        "worship" => "Worship",
                        "bible" => "Bible",
                        "timers" => "Timers",
                        "ai" => "AI",
                        "settings" => "Settings",
                        _ => v,
                    };
                    let vn = view_name.clone();
                    view! {
                        <button
                            type="button"
                            data-role="view-toggle"
                            data-view=view_name.clone()
                            data-active=move || if ctx.view.get() == vn { "true" } else { "false" }
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
                                data-active=move || if ctx.mode.get() == mn { "true" } else { "false" }
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
