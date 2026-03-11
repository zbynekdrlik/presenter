use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Presentation grid in catalog bottom area.
#[component]
pub fn PresentationList(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let presentations = ctx.presentations;
    let selected_pres_id = ctx.selected_presentation_id;
    let selected_pres = ctx.selected_presentation;
    let context_title = ctx.context_title;
    let selected_lib = ctx.selected_library_id;
    let selected_pl = ctx.selected_playlist_id;

    let on_select = move |id: String| {
        selected_pres_id.set(Some(id.clone()));
        crate::state::session::set("currentPresentationId", &id);
        let selected = selected_pres;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = crate::api::presentations::get_presentation(&id).await {
                selected.set(Some(pres));
            }
        });
    };

    let pres_count = move || {
        let count = presentations.get().len();
        if count > 0 {
            count.to_string()
        } else {
            "\u{2014}".to_string()
        }
    };

    let has_context = move || selected_lib.get().is_some() || selected_pl.get().is_some();
    let op_clone = op.clone();

    view! {
        <div class="operator__catalog-bottom" data-role="catalog-bottom" data-dropzone-target="presentations">
            <header class="operator__group-header operator__presentations-header">
                <h2 data-role="context-title">{move || context_title.get()}</h2>
                <div class="operator__group-controls">
                    <span class="operator__group-count operator__group-count--static" data-role="presentation-count">
                        {pres_count}
                    </span>
                    <button
                        type="button"
                        data-role="presentation-create"
                        aria-label="Add presentation or separator"
                        title="Add"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::open_modal(&op, "presentation-create")
                        }
                    >"+"</button>
                </div>
            </header>
            <ul class="operator__presentation-list" data-role="presentation-list">
                {move || {
                    if !has_context() {
                        return vec![view! {
                            <li class="empty">"Select a library or playlist to view presentations."</li>
                        }.into_any()];
                    }
                    let pres = presentations.get();
                    if pres.is_empty() {
                        return vec![view! {
                            <li class="empty">"No presentations found."</li>
                        }.into_any()];
                    }
                    pres.into_iter().map(|p| {
                        let id = p.id.to_string();
                        let name = p.name.clone();
                        let id_click = id.clone();
                        let id_cmp = id.clone();
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class="operator__presentation-button"
                                    data-role="presentation-item"
                                    data-presentation-id={id.clone()}
                                    data-type="presentation"
                                    data-active=move || if selected_pres_id.get().as_deref() == Some(&id_cmp) { "true" } else { "false" }
                                    on:click={
                                        let id = id_click.clone();
                                        move |_| on_select(id.clone())
                                    }
                                >
                                    {name}
                                </button>
                            </li>
                        }.into_any()
                    }).collect::<Vec<_>>()
                }}
            </ul>
        </div>
    }
}
