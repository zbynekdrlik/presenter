use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::pages::bible_prepared_list::{
    reload_presentation_lists, PresentationArchive, PresentationList,
};
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
    let active_presentation_id = bs.active_presentation_id;

    let on_create = {
        let ctx = ctx.clone();
        move |_| {
            let window = crate::utils::window::window();
            let name = match window.prompt_with_message("Názov prezentácie:") {
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
                        toast_message.set(Some(format!("Vytvorené: {}", detail.name)));
                        reload_presentation_lists(bs).await;
                        active_presentation_id.set(Some(new_id));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Vytvorenie zlyhalo: {e}")));
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
                <h3>"Prezentácie"</h3>
                <button
                    type="button"
                    class="operator__list-action"
                    data-role="presentation-create"
                    aria-label="Nová prezentácia"
                    title="Nová prezentácia"
                    on:click=on_create
                >"+"</button>
            </div>
            <PresentationList />
            <PresentationArchive />
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
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::rename_presentation(&id, &new_name).await {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Premenované".to_string()));
                        reload_presentation_lists(bs).await;
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Premenovanie zlyhalo: {e}")));
                    }
                }
            });
        }
    };

    // #874: an active presentation is archived, never deleted; it can be
    // deleted for good only from the archive.
    let on_archive = {
        let ctx = ctx.clone();
        move |_: web_sys::MouseEvent| {
            let Some(id) = modal_id.get_untracked() else {
                return;
            };
            let active_id = bs.active_presentation_id;
            let active_slides = bs.active_presentation_slides;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::archive_presentation(&id).await {
                    Ok(()) => {
                        if active_id.get_untracked().as_ref() == Some(&id) {
                            active_id.set(None);
                            active_slides.set(Vec::new());
                        }
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Archivované — nájdete v Archíve dole".to_string()));
                        reload_presentation_lists(bs).await;
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Archivovanie zlyhalo: {e}")));
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
                <h3>"Upraviť prezentáciu"</h3>
                    <label class="operator__field">
                        <span>"Názov"</span>
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
                            class="bible__modal-btn"
                            data-role="modal-archive"
                            title="Presunie prezentáciu do Archívu; odtiaľ sa dá obnoviť"
                            on:click=on_archive
                        >"Archivovať"</button>
                        <div class="bible__modal-actions-right">
                            <button
                                type="button"
                                class="bible__modal-btn"
                                data-role="modal-cancel"
                                on:click=on_close
                            >"Zrušiť"</button>
                            <button
                                type="submit"
                                class="bible__modal-btn bible__modal-btn--primary"
                                data-role="modal-save"
                            >"Uložiť"</button>
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
