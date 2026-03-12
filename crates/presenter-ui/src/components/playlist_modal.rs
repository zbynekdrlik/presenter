use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Playlist list modal + playlist edit/create modal.
#[component]
pub fn PlaylistModals() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let op = use_context::<OperatorState>().expect("OperatorState");

    let playlists = ctx.playlists;
    let open_modal_sig = op.open_modal;

    // Edit modal form signals
    let name_value = RwSignal::new(String::new());
    let dashboard_value = RwSignal::new(false);

    let is_pl_list_open = move || open_modal_sig.get().as_deref() == Some("playlist-list");
    let is_pl_edit_open = move || {
        matches!(
            open_modal_sig.get().as_deref(),
            Some("playlist-edit") | Some("playlist-create")
        )
    };
    let edit_mode = move || {
        if open_modal_sig.get().as_deref() == Some("playlist-create") {
            "create"
        } else {
            "edit"
        }
    };

    // Populate form when modal opens
    {
        let op = op.clone();
        Effect::new(move || {
            let modal = op.open_modal.get();
            match modal.as_deref() {
                Some("playlist-edit") => {
                    if let Some(target_id) = op.modal_target_id.get_untracked() {
                        let pls = playlists.get_untracked();
                        if let Some(pl) = pls.iter().find(|p| p.id.to_string() == target_id) {
                            name_value.set(pl.name.clone());
                            dashboard_value.set(pl.show_in_dashboard);
                        }
                    }
                }
                Some("playlist-create") => {
                    name_value.set(String::new());
                    dashboard_value.set(false);
                }
                _ => {}
            }
        });
    }

    // Save handler
    let on_save = {
        let op = op.clone();
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            let name = name_value.get_untracked().trim().to_string();
            if name.is_empty() {
                return;
            }
            let mode = edit_mode();
            let dashboard = dashboard_value.get_untracked();
            let target_id = op.modal_target_id.get_untracked().unwrap_or_default();
            op.submitting.set(true);

            let playlists = ctx.playlists;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                let result = if mode == "create" {
                    crate::api::playlists::create_playlist(&name, dashboard)
                        .await
                        .map(|_| ())
                } else {
                    crate::api::playlists::update_playlist(&target_id, Some(name), Some(dashboard))
                        .await
                        .map(|_| ())
                };

                // Reload playlists
                if let Ok(pls) = crate::api::playlists::list_playlists().await {
                    playlists.set(pls);
                }

                submitting.set(false);
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }

                match result {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Playlist saved".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Error: {e}")));
                    }
                }
            });
        }
    };

    // Delete handler
    let on_delete = {
        let op = op.clone();
        move |_| {
            let target_id = op.modal_target_id.get_untracked().unwrap_or_default();
            let pls = playlists.get_untracked();
            let pl_name = pls
                .iter()
                .find(|p| p.id.to_string() == target_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();

            if !crate::components::modal::confirm(&format!("Delete playlist \"{pl_name}\"?")) {
                return;
            }

            let playlists = ctx.playlists;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;

            leptos::task::spawn_local(async move {
                let _ = crate::api::playlists::delete_playlist(&target_id).await;
                if let Ok(pls) = crate::api::playlists::list_playlists().await {
                    playlists.set(pls);
                }
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
                toast_variant.set("success".to_string());
                toast_message.set(Some("Playlist deleted".to_string()));
            });
        }
    };

    // Cancel handler
    let on_cancel = {
        let op = op.clone();
        move |_| {
            crate::components::modal::close_modal(&op);
        }
    };

    // Playlist list: select a playlist
    let on_list_select = {
        let op = op.clone();
        move |id: String, name: String| {
            ctx.selected_playlist_id.set(Some(id.clone()));
            ctx.selected_library_id.set(None);
            ctx.context_title.set(name);
            crate::state::session::set("activePlaylistId", &id);
            crate::state::session::remove("activeLibraryId");
            crate::components::modal::close_modal(&op);
        }
    };

    // Backdrop close
    let close_list = {
        let op = op.clone();
        move |ev: web_sys::MouseEvent| {
            let target = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
            if let Some(el) = target {
                if el.class_list().contains("operator__playlist-modal") {
                    crate::components::modal::close_modal(&op);
                }
            }
        }
    };

    let close_list_btn = {
        let op = op.clone();
        move |_| crate::components::modal::close_modal(&op)
    };

    view! {
        // Playlist list modal
        <div class="operator__playlist-modal" data-role="playlist-modal"
            attr:data-open=move || if is_pl_list_open() { "true" } else { "false" }
            style:display=move || if is_pl_list_open() { "flex" } else { "none" }
            on:click=close_list
        >
            <div class="operator__playlist-modal-panel">
                <header class="operator__playlist-modal-header">
                    <h3>"All Playlists"</h3>
                    <button
                        type="button"
                        class="operator__playlist-modal-close"
                        data-role="playlist-modal-close"
                        aria-label="Close"
                        on:click=close_list_btn
                    >{"\u{00D7}"}</button>
                </header>
                <div class="operator__playlist-modal-body" data-role="playlist-modal-list">
                    {move || {
                        playlists.get().into_iter().map(|pl| {
                            let id = pl.id.to_string();
                            let name = pl.name.clone();
                            let count = pl.entries.len();
                            let is_dashboard = pl.show_in_dashboard;
                            let id_click = id.clone();
                            let name_click = name.clone();
                            let id_toggle = id.clone();
                            let on_select = on_list_select.clone();
                            view! {
                                <div class="operator__playlist-modal-item operator__list-row"
                                    data-role="playlist-row"
                                    data-playlist-id=id
                                >
                                    <button
                                        type="button"
                                        class="operator__list-action operator__list-action--icon operator__list-action--star"
                                        data-action="playlist-toggle-dashboard"
                                        attr:aria-pressed=if is_dashboard { "true" } else { "false" }
                                        on:click=move |ev: leptos::ev::MouseEvent| {
                                            ev.stop_propagation();
                                            let id = id_toggle.clone();
                                            let new_dashboard = !is_dashboard;
                                            let pls = playlists;
                                            leptos::task::spawn_local(async move {
                                                let _ = crate::api::playlists::update_playlist(
                                                    &id, None, Some(new_dashboard)
                                                ).await;
                                                if let Ok(updated_pls) = crate::api::playlists::list_playlists().await {
                                                    pls.set(updated_pls);
                                                }
                                            });
                                        }
                                    >
                                        {if is_dashboard { "\u{2605}" } else { "\u{2606}" }}
                                    </button>
                                    <button type="button" class="operator__list-button" on:click=move |_| {
                                        on_select(id_click.clone(), name_click.clone());
                                    }>
                                        <span class="operator__list-label">{name}</span>
                                        <span class="operator__list-meta">{count}</span>
                                    </button>
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
        </div>

        // Playlist edit/create modal
        <div class="operator__library-edit operator__playlist-edit" data-role="playlist-edit-modal"
            data-mode=edit_mode
            attr:data-open=move || if is_pl_edit_open() { "true" } else { "false" }
            style:display=move || if is_pl_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="playlist-edit-form"
                    on:submit=on_save
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="playlist-edit-title">
                            {move || if edit_mode() == "create" { "Create Playlist" } else { "Edit Playlist" }}
                        </h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span>"Playlist name"</span>
                            <input
                                type="text"
                                data-role="playlist-edit-name"
                                autocomplete="off"
                                required
                                minlength="1"
                                maxlength="160"
                                prop:value=move || name_value.get()
                                on:input=move |ev| name_value.set(event_target_value(&ev))
                            />
                        </label>
                        <label class="operator__library-edit-favorite">
                            <input
                                type="checkbox"
                                data-role="playlist-edit-dashboard"
                                prop:checked=move || dashboard_value.get()
                                on:change=move |ev| {
                                    dashboard_value.set(checkbox_checked(&ev));
                                }
                            />
                            <span>"Show in dashboard"</span>
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="playlist-edit-delete"
                            style:display=move || if edit_mode() == "edit" { "inline-block" } else { "none" }
                            on:click=on_delete
                        >"Delete playlist"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="playlist-edit-cancel"
                                on:click=on_cancel
                            >"Cancel"</button>
                            <button type="submit" data-role="playlist-edit-save">"Save changes"</button>
                        </div>
                    </footer>
                </form>
            </div>
        </div>
    }
}

fn checkbox_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}
