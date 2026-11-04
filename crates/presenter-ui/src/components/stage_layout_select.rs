//! The Stage Output select (#850 spec §6.3). On a desktop a change applies at
//! once, as before; from a phone (`confirm`) it first asks "Prepnúť stage na
//! …?", so a slip of the thumb never switches every stage display. Only
//! "Prepnúť" posts `/stage/layout`.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::AppContext;

#[component]
pub fn StageLayoutSelect(#[prop(optional)] confirm: bool) -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let layouts = ctx.stage_layouts;
    let current = ctx.stage_layout_code;
    let pending = RwSignal::new(None::<String>);

    let apply = move |code: String| {
        let previous = current.get_untracked();
        current.set(code.clone());
        leptos::task::spawn_local(async move {
            if crate::api::stage::set_layout(&code).await.is_err() {
                // The server never took it: don't claim a layout the displays
                // are not showing.
                leptos::logging::warn!("failed to set stage layout {code}");
                current.set(previous);
            }
        });
    };
    let on_change = move |ev: web_sys::Event| {
        let select = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
        let Some(select) = select else {
            return;
        };
        let code = select.value();
        if !confirm {
            apply(code);
            return;
        }
        // The select keeps showing the layout on air until "Prepnúť".
        select.set_value(&current.get_untracked());
        if code != current.get_untracked() {
            pending.set(Some(code));
        }
    };
    let pending_name = move || {
        let code = pending.get()?;
        layouts.with(|all| {
            all.iter()
                .find(|layout| layout.code == code)
                .map(|layout| layout.name.clone())
        })
    };

    view! {
        <div class="operator__stage-layout" aria-label="Stage display mode">
            <label class="operator__stage-layout-label" for="stage-layout-select">
                "Stage Output"
            </label>
            <select id="stage-layout-select" data-role="stage-layout-select" on:change=on_change>
                {move || {
                    layouts
                        .get()
                        .into_iter()
                        .map(|layout| {
                            let selected = current.get() == layout.code;
                            view! {
                                <option value=layout.code prop:selected=selected>
                                    {layout.name}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
        </div>
        <Show when=move || pending.with(Option::is_some)>
            <div class="stage-confirm" data-role="stage-layout-confirm" role="alertdialog">
                <p>{move || format!("Prepnúť stage na {}?", pending_name().unwrap_or_default())}</p>
                <button
                    type="button"
                    class="stage-confirm__yes"
                    data-role="stage-layout-confirm-yes"
                    on:click=move |_| {
                        if let Some(code) = pending.get_untracked() {
                            pending.set(None);
                            apply(code);
                        }
                    }
                >
                    "Prepnúť"
                </button>
                <button
                    type="button"
                    class="stage-confirm__no"
                    data-role="stage-layout-confirm-no"
                    on:click=move |_| pending.set(None)
                >
                    "Zrušiť"
                </button>
            </div>
        </Show>
    }
}
