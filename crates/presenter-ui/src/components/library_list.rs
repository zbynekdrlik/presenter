use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Library sidebar list component.
#[component]
pub fn LibraryList(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let libraries = ctx.libraries;
    let selected_lib = ctx.selected_library_id;
    let selected_pl = ctx.selected_playlist_id;
    let presentations = ctx.presentations;
    let context_title = ctx.context_title;
    let selected_pres_id = ctx.selected_presentation_id;
    let selected_pres = ctx.selected_presentation;

    let on_select = move |id: String, name: String| {
        // Selecting a library clears playlist selection
        selected_pl.set(None);
        crate::state::session::remove("activePlaylistId");
        selected_lib.set(Some(id.clone()));
        crate::state::session::set("activeLibraryId", &id);
        context_title.set(name);
        // Clear presentation selection
        selected_pres_id.set(None);
        selected_pres.set(None);
        // Load presentations
        let presentations = presentations;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = crate::api::libraries::list_presentations(&id).await {
                presentations.set(pres);
            }
        });
    };

    let lib_count = move || libraries.get().len();
    let op_clone = op.clone();

    view! {
        <section class="operator__group operator__group--libraries">
            <header class="operator__group-header">
                <h2>"Libraries"</h2>
                <div class="operator__group-controls">
                    <button
                        type="button"
                        class="operator__group-count"
                        data-role="library-more"
                        aria-label="Show all libraries"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::open_modal(&op, "library-list")
                        }
                    >{move || lib_count().to_string()}</button>
                    <button
                        type="button"
                        data-role="library-create"
                        aria-label="Create library"
                        title="Create library"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::open_modal(&op, "library-create")
                        }
                    >"+"</button>
                </div>
            </header>
            <ul class="operator__list" data-role="library-list">
                {move || {
                    libraries.get().into_iter().map(|lib| {
                        let id = lib.id.to_string();
                        let name = lib.name.clone();
                        let count = lib.presentation_count;
                        let id_click = id.clone();
                        let name_click = name.clone();
                        let id_cmp = id.clone();
                        view! {
                            <li class="operator__list-item">
                                <button
                                    type="button"
                                    class="operator__list-button"
                                    data-role="library-item"
                                    data-library-id={id.clone()}
                                    data-active=move || if selected_lib.get().as_deref() == Some(&id_cmp) { "true" } else { "false" }
                                    on:click={
                                        let id = id_click.clone();
                                        let name = name_click.clone();
                                        move |_| on_select(id.clone(), name.clone())
                                    }
                                >
                                    <span class="operator__list-label">{name}</span>
                                    <span class="operator__list-meta" data-role="library-count">{count}</span>
                                </button>
                                <div class="operator__list-actions">
                                    <button
                                        type="button"
                                        class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                        data-action="library-edit"
                                        data-library-id={id.clone()}
                                        aria-label="Edit library"
                                        on:click={
                                            let op = op_clone.clone();
                                            let id = id.clone();
                                            move |_| {
                                                op.modal_target_id.set(Some(id.clone()));
                                                crate::components::modal::open_modal(&op, "library-edit");
                                            }
                                        }
                                    >{"\u{22EE}"}</button>
                                </div>
                            </li>
                        }
                    }).collect::<Vec<_>>()
                }}
            </ul>
        </section>
    }
}
