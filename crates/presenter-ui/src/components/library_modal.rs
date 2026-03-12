use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Library list modal + library edit/create modal.
#[component]
pub fn LibraryModals() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let op = use_context::<OperatorState>().expect("OperatorState");

    let libraries = ctx.libraries;
    let open_modal_sig = op.open_modal;
    let favorite_ids = ctx.favorite_library_ids;

    // Edit modal form signals
    let name_value = RwSignal::new(String::new());
    let favorite_value = RwSignal::new(false);

    let is_lib_list_open = move || open_modal_sig.get().as_deref() == Some("library-list");
    let is_lib_edit_open = move || {
        matches!(
            open_modal_sig.get().as_deref(),
            Some("library-edit") | Some("library-create")
        )
    };
    let edit_mode = move || {
        if open_modal_sig.get().as_deref() == Some("library-create") {
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
                Some("library-edit") => {
                    if let Some(target_id) = op.modal_target_id.get_untracked() {
                        let libs = libraries.get_untracked();
                        if let Some(lib) = libs.iter().find(|l| l.id.to_string() == target_id) {
                            name_value.set(lib.name.clone());
                            let favs = favorite_ids.get_untracked();
                            favorite_value.set(favs.contains(&target_id));
                        }
                    }
                }
                Some("library-create") => {
                    name_value.set(String::new());
                    favorite_value.set(false);
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
            let fav = favorite_value.get_untracked();
            let target_id = op.modal_target_id.get_untracked().unwrap_or_default();
            op.submitting.set(true);

            let libraries = ctx.libraries;
            let favorite_ids = ctx.favorite_library_ids;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                let result = if mode == "create" {
                    match crate::api::libraries::create_library(&name).await {
                        Ok(lib) => {
                            let id = lib.id.to_string();
                            if fav {
                                let _ = crate::api::libraries::set_favorite(&id, true).await;
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    let _ = crate::api::libraries::rename_library(&target_id, &name).await;
                    let _ = crate::api::libraries::set_favorite(&target_id, fav).await;
                    Ok(())
                };

                // Reload
                if let Ok(libs) = crate::api::libraries::list_libraries().await {
                    libraries.set(libs);
                }
                if let Ok(favs) = crate::api::libraries::get_favorites().await {
                    favorite_ids.set(favs.into_iter().collect());
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
                        toast_message.set(Some("Library saved".to_string()));
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
            let libs = libraries.get_untracked();
            let lib_name = libs
                .iter()
                .find(|l| l.id.to_string() == target_id)
                .map(|l| l.name.clone())
                .unwrap_or_default();

            if !crate::components::modal::confirm(&format!("Delete library \"{lib_name}\"?")) {
                return;
            }

            let libraries = ctx.libraries;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;

            leptos::task::spawn_local(async move {
                let _ = crate::api::libraries::delete_library(&target_id).await;
                if let Ok(libs) = crate::api::libraries::list_libraries().await {
                    libraries.set(libs);
                }
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
                toast_variant.set("success".to_string());
                toast_message.set(Some("Library deleted".to_string()));
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

    // Library list: select a library
    let on_list_select = {
        let op = op.clone();
        move |id: String, name: String| {
            ctx.selected_library_id.set(Some(id.clone()));
            ctx.selected_playlist_id.set(None);
            ctx.context_title.set(name);
            crate::state::session::set("activeLibraryId", &id);
            crate::state::session::remove("activePlaylistId");
            crate::components::modal::close_modal(&op);

            let presentations = ctx.presentations;
            let libs_sig = ctx.libraries;
            let id_clone = id.clone();
            leptos::task::spawn_local(async move {
                if let Ok(summaries) = crate::api::libraries::list_libraries().await {
                    if let Some(lib) = summaries.iter().find(|l| l.id.to_string() == id_clone) {
                        presentations.set(lib.presentations.clone());
                    }
                    libs_sig.set(summaries);
                }
            });
        }
    };

    // Backdrop close handlers
    let close_list = {
        let op = op.clone();
        move |ev: web_sys::MouseEvent| {
            let target = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
            if let Some(el) = target {
                if el.class_list().contains("operator__library-modal") {
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
        // Library list modal
        <div class="operator__library-modal" data-role="library-modal"
            attr:data-open=move || if is_lib_list_open() { "true" } else { "false" }
            style:display=move || if is_lib_list_open() { "flex" } else { "none" }
            on:click=close_list
        >
            <div class="operator__library-modal-panel">
                <header class="operator__library-modal-header">
                    <h3>"All Libraries"</h3>
                    <button
                        type="button"
                        class="operator__library-modal-close"
                        data-role="library-modal-close"
                        aria-label="Close"
                        on:click=close_list_btn
                    >{"\u{00D7}"}</button>
                </header>
                <div class="operator__library-modal-body" data-role="library-modal-list">
                    {move || {
                        let favs = favorite_ids.get();
                        libraries.get().into_iter().map(|lib| {
                            let id = lib.id.to_string();
                            let name = lib.name.clone();
                            let count = lib.presentation_count;
                            let is_fav = favs.contains(&id);
                            let id_click = id.clone();
                            let name_click = name.clone();
                            let id_fav = id.clone();
                            let on_select = on_list_select.clone();
                            view! {
                                <div class="operator__library-modal-item operator__list-row"
                                    data-role="library-row"
                                    data-library-id=id
                                >
                                    <button
                                        type="button"
                                        class="operator__list-action operator__list-action--icon operator__list-action--star"
                                        data-action="library-toggle-favorite"
                                        attr:aria-pressed=if is_fav { "true" } else { "false" }
                                        on:click=move |ev: leptos::ev::MouseEvent| {
                                            ev.stop_propagation();
                                            let id = id_fav.clone();
                                            let new_fav = !is_fav;
                                            let fav_ids = favorite_ids;
                                            let libs = libraries;
                                            leptos::task::spawn_local(async move {
                                                let _ = crate::api::libraries::set_favorite(&id, new_fav).await;
                                                if let Ok(favs) = crate::api::libraries::get_favorites().await {
                                                    fav_ids.set(favs.into_iter().collect());
                                                }
                                                if let Ok(ls) = crate::api::libraries::list_libraries().await {
                                                    libs.set(ls);
                                                }
                                            });
                                        }
                                    >
                                        {if is_fav { "\u{2605}" } else { "\u{2606}" }}
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

        // Library edit/create modal
        <div class="operator__library-edit" data-role="library-edit-modal"
            data-mode=edit_mode
            attr:data-open=move || if is_lib_edit_open() { "true" } else { "false" }
            style:display=move || if is_lib_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="library-edit-form"
                    on:submit=on_save
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="library-edit-title">
                            {move || if edit_mode() == "create" { "Create Library" } else { "Edit Library" }}
                        </h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span>"Library name"</span>
                            <input
                                type="text"
                                data-role="library-edit-name"
                                autocomplete="off"
                                required
                                minlength="1"
                                maxlength="120"
                                prop:value=move || name_value.get()
                                on:input=move |ev| name_value.set(event_target_value(&ev))
                            />
                        </label>
                        <label class="operator__library-edit-favorite">
                            <input
                                type="checkbox"
                                data-role="library-edit-favorite"
                                prop:checked=move || favorite_value.get()
                                on:change=move |ev| {
                                    favorite_value.set(checkbox_checked(&ev));
                                }
                            />
                            <span>"Show in dashboard"</span>
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="library-edit-delete"
                            style:display=move || if edit_mode() == "edit" { "inline-block" } else { "none" }
                            on:click=on_delete
                        >"Delete library"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="library-edit-cancel"
                                on:click=on_cancel
                            >"Cancel"</button>
                            <button type="submit" data-role="library-edit-save">"Save changes"</button>
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
