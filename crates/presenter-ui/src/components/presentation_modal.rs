use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Presentation create/edit/rename modals.
#[component]
pub fn PresentationModals() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let open_modal_sig = op.open_modal;

    // Edit modal signals
    let edit_name = RwSignal::new(String::new());

    // Create modal signals
    let create_name = RwSignal::new(String::new());
    let create_step = RwSignal::new("options".to_string()); // "options", "paste", "import"

    let is_edit_open = move || open_modal_sig.get().as_deref() == Some("presentation-edit");
    let is_create_open = move || open_modal_sig.get().as_deref() == Some("presentation-create");

    // Populate edit name when modal opens
    {
        let op = op.clone();
        let ctx = ctx.clone();
        Effect::new(move || {
            let modal = op.open_modal.get();
            if modal.as_deref() == Some("presentation-edit") {
                if let Some(target_id) = op.modal_target_id.get_untracked() {
                    let presentations = ctx.presentations.get_untracked();
                    if let Some(pres) = presentations.iter().find(|p| p.id.to_string() == target_id)
                    {
                        edit_name.set(pres.name.clone());
                    }
                }
            } else if modal.as_deref() == Some("presentation-create") {
                create_name.set(String::new());
                create_step.set("options".to_string());
                op.paste_text.set(String::new());
            }
        });
    }

    // Edit: save (rename)
    let on_edit_save = {
        let op = op.clone();
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            let name = edit_name.get_untracked().trim().to_string();
            if name.is_empty() {
                return;
            }
            let target_id = op.modal_target_id.get_untracked().unwrap_or_default();
            op.submitting.set(true);

            let presentations = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_lib_id = ctx.selected_library_id;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                let result = crate::api::presentations::rename(&target_id, &name).await;

                // Reload presentations for current library
                if let Some(lib_id) = selected_lib_id.get_untracked() {
                    if let Ok(pres) = crate::api::libraries::list_presentations(&lib_id).await {
                        presentations.set(pres);
                    }
                }
                // Also refresh libraries list
                if let Ok(libs) = crate::api::libraries::list_libraries().await {
                    libraries.set(libs);
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
                        toast_message.set(Some("Presentation renamed".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Error: {e}")));
                    }
                }
            });
        }
    };

    // Edit: delete
    let on_edit_delete = {
        let op = op.clone();
        move |_| {
            let target_id = op.modal_target_id.get_untracked().unwrap_or_default();
            let presentations = ctx.presentations.get_untracked();
            let pres_name = presentations
                .iter()
                .find(|p| p.id.to_string() == target_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();

            if !crate::components::modal::confirm(&format!("Delete presentation \"{pres_name}\"?"))
            {
                return;
            }

            let presentations_sig = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_lib_id = ctx.selected_library_id;
            let selected_pres_id = ctx.selected_presentation_id;
            let selected_pres = ctx.selected_presentation;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;

            leptos::task::spawn_local(async move {
                let _ = crate::api::presentations::delete_presentation(&target_id).await;

                // Clear selection if we deleted the selected one
                if selected_pres_id.get_untracked().as_deref() == Some(&target_id) {
                    selected_pres_id.set(None);
                    selected_pres.set(None);
                    crate::state::session::remove("currentPresentationId");
                }

                // Reload presentations for current library
                if let Some(lib_id) = selected_lib_id.get_untracked() {
                    if let Ok(pres) = crate::api::libraries::list_presentations(&lib_id).await {
                        presentations_sig.set(pres);
                    }
                }
                if let Ok(libs) = crate::api::libraries::list_libraries().await {
                    libraries.set(libs);
                }

                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
                toast_variant.set("success".to_string());
                toast_message.set(Some("Presentation deleted".to_string()));
            });
        }
    };

    // Edit: cancel
    let on_edit_cancel = {
        let op = op.clone();
        move |_| {
            crate::components::modal::close_modal(&op);
        }
    };

    // Create: blank
    let on_create_blank = {
        let op = op.clone();
        move |_| {
            let lib_id = match ctx.selected_library_id.get_untracked() {
                Some(id) => id,
                None => return,
            };
            let name = create_name.get_untracked().trim().to_string();
            let name = if name.is_empty() {
                "New Presentation".to_string()
            } else {
                name
            };
            op.submitting.set(true);

            let presentations = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_pres_id = ctx.selected_presentation_id;
            let selected_pres = ctx.selected_presentation;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                match crate::api::presentations::create_blank(&lib_id, &name).await {
                    Ok(resp) => {
                        let new_id = resp.presentation.id.to_string();
                        selected_pres_id.set(Some(new_id.clone()));
                        selected_pres.set(Some(resp.presentation));
                        crate::state::session::set("currentPresentationId", &new_id);

                        if let Some(summary) = resp.library_summary {
                            presentations.set(summary.presentations);
                        }
                        if let Ok(libs) = crate::api::libraries::list_libraries().await {
                            libraries.set(libs);
                        }

                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Presentation created".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Error: {e}")));
                    }
                }

                submitting.set(false);
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
            });
        }
    };

    // Create: paste confirm
    let on_paste_confirm = {
        let op = op.clone();
        move |_| {
            let lib_id = match ctx.selected_library_id.get_untracked() {
                Some(id) => id,
                None => return,
            };
            let name = create_name.get_untracked().trim().to_string();
            let name = if name.is_empty() {
                "New Presentation".to_string()
            } else {
                name
            };
            let text = op.paste_text.get_untracked();
            let slides = crate::utils::song_parser::parse_song_text(&text);
            if slides.is_empty() {
                return;
            }
            op.submitting.set(true);

            let presentations = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_pres_id = ctx.selected_presentation_id;
            let selected_pres = ctx.selected_presentation;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                match crate::api::presentations::create_from_paste(&lib_id, &name, slides).await {
                    Ok(resp) => {
                        let new_id = resp.presentation.id.to_string();
                        selected_pres_id.set(Some(new_id.clone()));
                        selected_pres.set(Some(resp.presentation));
                        crate::state::session::set("currentPresentationId", &new_id);

                        if let Some(summary) = resp.library_summary {
                            presentations.set(summary.presentations);
                        }
                        if let Ok(libs) = crate::api::libraries::list_libraries().await {
                            libraries.set(libs);
                        }

                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Presentation created from paste".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Error: {e}")));
                    }
                }

                submitting.set(false);
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
            });
        }
    };

    // Create: import confirm
    let on_import_confirm = {
        let op = op.clone();
        move |_| {
            let lib_id = match ctx.selected_library_id.get_untracked() {
                Some(id) => id,
                None => return,
            };

            // Get the file input element
            let doc = crate::utils::window::document();
            let file_input = doc
                .query_selector("[data-role='presentation-create-import-file']")
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok());

            let file_input = match file_input {
                Some(el) => el,
                None => return,
            };

            let files = match file_input.files() {
                Some(f) if f.length() > 0 => f,
                _ => return,
            };

            let file = match files.get(0) {
                Some(f) => f,
                None => return,
            };

            let form_data = match web_sys::FormData::new() {
                Ok(fd) => fd,
                Err(_) => return,
            };
            if form_data.append_with_blob("file", &file).is_err() {
                return;
            }

            op.submitting.set(true);

            let presentations = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_pres_id = ctx.selected_presentation_id;
            let selected_pres = ctx.selected_presentation;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                match crate::api::presentations::import_pro(&lib_id, &form_data).await {
                    Ok(resp) => {
                        let new_id = resp.presentation.id.to_string();
                        selected_pres_id.set(Some(new_id.clone()));
                        selected_pres.set(Some(resp.presentation));
                        crate::state::session::set("currentPresentationId", &new_id);

                        if let Some(summary) = resp.library_summary {
                            presentations.set(summary.presentations);
                        }
                        if let Ok(libs) = crate::api::libraries::list_libraries().await {
                            libraries.set(libs);
                        }

                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Presentation imported".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Import error: {e}")));
                    }
                }

                submitting.set(false);
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
            });
        }
    };

    // Create: cancel
    let on_create_cancel = {
        let op = op.clone();
        move |_| {
            crate::components::modal::close_modal(&op);
        }
    };

    let paste_text = op.paste_text;

    view! {
        // Presentation edit (rename) modal
        <div class="operator__library-edit operator__presentation-edit" data-role="presentation-edit-modal"
            data-mode="presentation"
            data-open=move || if is_edit_open() { "true" } else { "false" }
            style:display=move || if is_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="presentation-edit-form"
                    on:submit=on_edit_save
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="presentation-edit-title">"Rename Presentation"</h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span data-role="presentation-edit-label">"Presentation name"</span>
                            <input
                                type="text"
                                data-role="presentation-edit-name"
                                autocomplete="off"
                                required
                                minlength="1"
                                maxlength="160"
                                prop:value=move || edit_name.get()
                                on:input=move |ev| edit_name.set(event_target_value(&ev))
                            />
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="presentation-edit-delete"
                            on:click=on_edit_delete
                        >"Delete presentation"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="presentation-edit-cancel"
                                on:click=on_edit_cancel
                            >"Cancel"</button>
                            <button type="submit" data-role="presentation-edit-save">"Save changes"</button>
                        </div>
                    </footer>
                </form>
            </div>
        </div>

        // Presentation create modal
        <div class="operator__library-edit operator__presentation-create" data-role="presentation-create-modal"
            data-open=move || if is_create_open() { "true" } else { "false" }
            style:display=move || if is_create_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <div class="operator__library-edit-form" data-role="presentation-create-form">
                    <header class="operator__library-edit-header">
                        <h3>"Create Presentation"</h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span>"Presentation name"</span>
                            <input
                                type="text"
                                data-role="presentation-create-name"
                                autocomplete="off"
                                maxlength="160"
                                placeholder="New Presentation"
                                prop:value=move || create_name.get()
                                on:input=move |ev| create_name.set(event_target_value(&ev))
                            />
                        </label>

                        // Options panel
                        <div class="operator__presentation-create-options" data-role="presentation-create-options"
                            style:display=move || if create_step.get() == "options" { "flex" } else { "none" }
                        >
                            <button type="button" class="operator__create-option" data-role="presentation-create-blank"
                                on:click=on_create_blank
                            >
                                <span class="operator__create-option-label">"Blank"</span>
                                <span class="operator__create-option-desc">"Empty presentation"</span>
                            </button>
                            <button type="button" class="operator__create-option" data-role="presentation-create-paste"
                                on:click=move |_| create_step.set("paste".to_string())
                            >
                                <span class="operator__create-option-label">"Paste"</span>
                                <span class="operator__create-option-desc">"Paste song text"</span>
                            </button>
                            <button type="button" class="operator__create-option" data-role="presentation-create-import"
                                on:click=move |_| create_step.set("import".to_string())
                            >
                                <span class="operator__create-option-label">"Import"</span>
                                <span class="operator__create-option-desc">".pro file"</span>
                            </button>
                        </div>

                        // Paste area
                        <div class="operator__presentation-create-paste-area" data-role="presentation-create-paste-area"
                            style:display=move || if create_step.get() == "paste" { "block" } else { "none" }
                        >
                            <textarea
                                data-role="presentation-create-paste-text"
                                rows="10"
                                placeholder="Paste song text here..."
                                prop:value=move || paste_text.get()
                                on:input=move |ev| paste_text.set(event_target_value(&ev))
                            ></textarea>
                            <div class="operator__presentation-create-sub-actions">
                                <button type="button" data-role="presentation-create-paste-back"
                                    on:click=move |_| create_step.set("options".to_string())
                                >"Back"</button>
                                <button type="button" data-role="presentation-create-paste-confirm"
                                    on:click=on_paste_confirm
                                >"Create"</button>
                            </div>
                        </div>

                        // Import area
                        <div class="operator__presentation-create-import-area" data-role="presentation-create-import-area"
                            style:display=move || if create_step.get() == "import" { "block" } else { "none" }
                        >
                            <input type="file" data-role="presentation-create-import-file" accept=".pro" />
                            <div class="operator__presentation-create-sub-actions">
                                <button type="button" data-role="presentation-create-import-back"
                                    on:click=move |_| create_step.set("options".to_string())
                                >"Back"</button>
                                <button type="button" data-role="presentation-create-import-confirm"
                                    on:click=on_import_confirm
                                >"Import"</button>
                            </div>
                        </div>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="presentation-create-cancel"
                                on:click=on_create_cancel
                            >"Cancel"</button>
                        </div>
                    </footer>
                </div>
            </div>
        </div>
    }
}
