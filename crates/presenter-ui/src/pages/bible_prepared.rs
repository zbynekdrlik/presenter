use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::state::bible::BibleState;
use crate::state::AppContext;

// ---------------------------------------------------------------------------
// Prepared tab
// ---------------------------------------------------------------------------

#[component]
pub(super) fn BiblePreparedTab() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let bible_tab = bs.bible_tab;
    let presentations = bs.presentations;
    let active_presentation_id = bs.active_presentation_id;

    let on_create = {
        let ctx = ctx.clone();
        move |_| {
            let window = crate::utils::window::window();
            let name = match window.prompt_with_message("Presentation name:") {
                Ok(Some(n)) if !n.trim().is_empty() => n.trim().to_string(),
                _ => return,
            };
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::create_presentation(&name).await {
                    Ok(detail) => {
                        let new_id = detail.id.clone();
                        toast_variant.set("success".to_string());
                        toast_message.set(Some(format!("Created \"{}\"", detail.name)));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        active_presentation_id.set(Some(new_id));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Failed to create: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="prepared"
            data-visible=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
        >
            <div class="bible__prepared-header">
                <h3>"Presentations"</h3>
                <button
                    type="button"
                    class="operator__list-action"
                    data-role="presentation-create"
                    aria-label="Create presentation"
                    on:click=on_create
                >"+"</button>
            </div>
            <div class="bible__prepared-list" data-role="presentations-list">
                {move || {
                    let pres_list = presentations.get();
                    if pres_list.is_empty() {
                        view! {
                            <p class="operator__slides-empty">"No Bible presentations yet."</p>
                        }.into_any()
                    } else {
                        pres_list.into_iter().map(|p| {
                            view! { <PresentationCard presentation=p /> }
                        }).collect_view().into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn PresentationCard(presentation: bible::BiblePresentationSummary) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let active_presentation_id = bs.active_presentation_id;
    let active_presentation_slides = bs.active_presentation_slides;

    let id = presentation.id.clone();
    let name = presentation.name.clone();
    let count = presentation.slide_count;
    let id_for_click = id.clone();
    let id_for_edit = id.clone();
    let name_for_edit = name.clone();

    let is_active = {
        let id = id.clone();
        move || active_presentation_id.get().as_ref() == Some(&id)
    };

    let on_click = move |_| {
        let id = id_for_click.clone();
        active_presentation_id.set(Some(id.clone()));
        leptos::task::spawn_local(async move {
            if let Ok(detail) = bible::get_presentation(&id).await {
                active_presentation_slides.set(detail.slides);
            }
        });
    };

    let on_edit = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        bs.modal_presentation_id.set(Some(id_for_edit.clone()));
        bs.modal_presentation_name.set(name_for_edit.clone());
    };

    view! {
        <div
            class="operator__presentation-card"
            class:is-active=is_active
            data-presentation-id=id.clone()
            data-role="presentation-card"
            on:click=on_click
        >
            <div style="display:flex;justify-content:space-between;align-items:center;">
                <strong>{name.clone()}</strong>
                <button
                    type="button"
                    class="operator__presentation-action"
                    data-role="presentation-edit"
                    on:click=on_edit
                    title="Edit presentation"
                >"\u{270F}\u{FE0F}"</button>
            </div>
            <p>{format!("{} slide(s)", count)}</p>
        </div>
    }
}

#[component]
pub(super) fn BiblePresentationModal() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let modal_id = bs.modal_presentation_id;
    let modal_name = bs.modal_presentation_name;

    let is_open = move || modal_id.get().is_some();

    let on_close = move |_: web_sys::MouseEvent| {
        modal_id.set(None);
        modal_name.set(String::new());
    };

    let on_name_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            modal_name.set(input.value());
        }
    };

    let on_save = {
        let ctx = ctx.clone();
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            let Some(id) = modal_id.get_untracked() else {
                return;
            };
            let new_name = modal_name.get_untracked().trim().to_string();
            if new_name.is_empty() {
                return;
            }
            let presentations = bs.presentations;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::rename_presentation(&id, &new_name).await {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Renamed".to_string()));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Rename failed: {e}")));
                    }
                }
            });
        }
    };

    let on_delete = {
        let ctx = ctx.clone();
        move |_: web_sys::MouseEvent| {
            let Some(id) = modal_id.get_untracked() else {
                return;
            };
            let window = crate::utils::window::window();
            if let Ok(confirmed) = window.confirm_with_message("Delete this presentation?") {
                if !confirmed {
                    return;
                }
            }
            let presentations = bs.presentations;
            let active_id = bs.active_presentation_id;
            let active_slides = bs.active_presentation_slides;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::delete_presentation(&id).await {
                    Ok(()) => {
                        active_id.set(None);
                        active_slides.set(Vec::new());
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Deleted".to_string()));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Delete failed: {e}")));
                    }
                }
            });
        }
    };

    let on_backdrop = move |ev: web_sys::MouseEvent| {
        // Close only if clicking the backdrop itself
        let target = ev.target();
        let current = ev.current_target();
        if target == current {
            modal_id.set(None);
            modal_name.set(String::new());
        }
    };

    view! {
        <div
            class="bible__modal-overlay"
            data-role="presentation-modal"
            style:display=move || if is_open() { "flex" } else { "none" }
            on:click=on_backdrop
        >
            <form class="bible__modal" on:submit=on_save>
                <h3>"Edit presentation"</h3>
                    <label class="operator__field">
                        <span>"Name"</span>
                        <input
                            type="text"
                            data-role="modal-presentation-name"
                            prop:value=move || modal_name.get()
                            on:input=on_name_input
                        />
                    </label>
                    <div class="bible__modal-actions">
                        <button
                            type="button"
                            class="bible__modal-btn bible__modal-btn--danger"
                            data-role="modal-delete"
                            on:click=on_delete
                        >"Delete"</button>
                        <div class="bible__modal-actions-right">
                            <button
                                type="button"
                                class="bible__modal-btn"
                                data-role="modal-cancel"
                                on:click=on_close
                            >"Cancel"</button>
                            <button
                                type="submit"
                                class="bible__modal-btn bible__modal-btn--primary"
                                data-role="modal-save"
                            >"Save"</button>
                        </div>
                    </div>
            </form>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Settings tab
// ---------------------------------------------------------------------------

#[component]
pub(super) fn BibleSettingsTab() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let bible_tab = bs.bible_tab;
    let character_limit = bs.character_limit;

    let on_char_limit_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            if let Ok(val) = input.value().parse::<u32>() {
                character_limit.set(val.clamp(1, 4000));
            }
        }
    };

    let on_save = {
        let ctx = ctx.clone();
        move |_| {
            let limit = character_limit.get_untracked();
            let main = bs.selected_translation.get_untracked();
            let sec = bs.secondary_translation.get_untracked();
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;

            let draft = presenter_core::BiblePreferencesDraft {
                main_translation: main,
                secondary_translation: sec,
                character_limit: Some(limit),
            };

            leptos::task::spawn_local(async move {
                match bible::update_preferences(&draft).await {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Preferences saved".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Save failed: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="settings"
            data-visible=move || if bible_tab.get() == "settings" { "true" } else { "false" }
        >
            <div class="operator__form-group">
                <label class="operator__field">
                    <span>"Character limit"</span>
                    <input
                        type="number"
                        data-role="char-limit"
                        min="1"
                        max="4000"
                        prop:value=move || character_limit.get().to_string()
                        on:change=on_char_limit_change
                    />
                </label>
                <button
                    type="button"
                    class="operator__list-action operator__list-action--primary"
                    data-role="save-preferences"
                    on:click=on_save
                >"Save preferences"</button>
            </div>
        </div>
    }
}
