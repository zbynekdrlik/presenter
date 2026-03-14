use leptos::prelude::*;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn LibraryList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let op = use_context::<OperatorState>().expect("OperatorState");

    // Track loading state for library presentations
    let is_loading_presentations = RwSignal::new(false);

    let select_library = move |id: String, name: String| {
        ctx.selected_library_id.set(Some(id.clone()));
        ctx.selected_playlist_id.set(None);
        ctx.selected_playlist.set(None);
        ctx.context_title.set(name);
        crate::state::session::set("activeLibraryId", &id);
        crate::state::session::remove("activePlaylistId");

        // Capture signals OUTSIDE async block - context may not be available inside spawn_local
        let presentations_signal = ctx.presentations;
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;
        let id_clone = id.clone();
        is_loading_presentations.set(true);
        leptos::task::spawn_local(async move {
            match crate::api::libraries::list_presentations(&id_clone).await {
                Ok(presentations) => {
                    presentations_signal.set(presentations);
                }
                Err(e) => {
                    // Show error toast on API failure
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Failed to load presentations: {e}")));
                    gloo_timers::callback::Timeout::new(3_500, move || {
                        toast_message.set(None);
                    })
                    .forget();
                }
            }
            is_loading_presentations.set(false);
        });
    };

    let on_create = {
        let op = op.clone();
        move |_| {
            op.modal_mode.set("create".to_string());
            op.modal_target_id.set(None);
            modal::open_modal(&op, "library-edit");
        }
    };

    let on_more = {
        let op = op.clone();
        move |_| {
            modal::open_modal(&op, "library-list");
        }
    };

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
                        on:click=on_more
                    >
                        {move || {
                            let total = ctx.libraries.get().len();
                            total.to_string()
                        }}
                    </button>
                    <button
                        type="button"
                        data-role="library-create"
                        aria-label="Create library"
                        title="Create library"
                        on:click=on_create
                    >"+"</button>
                </div>
            </header>
            <ul class="operator__list" data-role="library-list">
                {move || {
                    let libs = ctx.libraries.get();
                    let favs = ctx.favorite_library_ids.get();
                    let active_id = ctx.selected_library_id.get();

                    // Show loading state while libraries are being fetched
                    if libs.is_empty() && ctx.ws_connected.get() {
                        // Libraries haven't loaded yet but WS is connected (initial load)
                        return view! {
                            <li class="operator__favorites-empty">
                                "Loading libraries..."
                            </li>
                        }.into_any();
                    }

                    if libs.is_empty() {
                        return view! {
                            <li class="operator__favorites-empty">
                                "No libraries found."
                            </li>
                        }.into_any();
                    }

                    // Match JS behavior: if no favorites configured, show ALL libraries sorted
                    // Otherwise show favorites + active library
                    let visible: Vec<_> = if favs.is_empty() {
                        // No favorites - show all libraries sorted alphabetically
                        let mut all = libs.clone();
                        all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                        all
                    } else {
                        // Show favorites + active library (JS behavior)
                        libs.into_iter().filter(|lib| {
                            let id = lib.id.to_string();
                            favs.contains(&id) || active_id.as_deref() == Some(&id)
                        }).collect()
                    };

                    if visible.is_empty() {
                        return view! {
                            <li class="operator__favorites-empty">
                                "Star libraries in settings to keep them handy."
                            </li>
                        }.into_any();
                    }

                    view! {
                        <div class="operator__favorites">
                            {visible.into_iter().map(|lib| {
                                let id = lib.id.to_string();
                                let name = lib.name.clone();
                                let count = lib.presentation_count;
                                let id_for_click = id.clone();
                                let name_for_click = name.clone();
                                let id_for_edit = id.clone();
                                let id_for_row = id.clone();
                                let id_for_btn = id.clone();
                                let id_for_modal = id.clone();
                                let id_for_active = id.clone();
                                let selected_library_id = ctx.selected_library_id;
                                let op_for_edit = op.clone();

                                let is_active = Signal::derive(move || {
                                    selected_library_id.get().as_deref() == Some(id_for_active.as_str())
                                });

                                view! {
                                    <li class="operator__list-item operator__list-row" data-library-id=id_for_row>
                                        <button
                                            type="button"
                                            class="operator__list-button"
                                            data-role="library-item"
                                            data-library-id=id_for_btn
                                            data-active=move || if is_active.get() { "true" } else { "false" }
                                            on:click=move |_| {
                                                select_library(id_for_click.clone(), name_for_click.clone());
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
                                                data-library-id=id_for_edit
                                                aria-label="Edit library"
                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                    ev.stop_propagation();
                                                    op_for_edit.modal_mode.set("edit".to_string());
                                                    op_for_edit.modal_target_id.set(Some(id_for_modal.clone()));
                                                    modal::open_modal(&op_for_edit, "library-edit");
                                                }
                                            >
                                                "\u{22ee}"
                                            </button>
                                        </div>
                                    </li>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }}
            </ul>
        </section>
    }
}
