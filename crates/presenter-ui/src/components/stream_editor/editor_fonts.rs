//! In-editor web-font upload + delete panel (#778), a sibling of
//! `editor_assets.rs`. Uploads one or more `.ttf`/`.otf` files (each via a
//! single multipart `POST /stream/fonts`, field `file`), lists the uploaded
//! faces grouped by their parsed family/weight/italic, and deletes via
//! `DELETE /stream/fonts/{id}` — surfacing the guarded 409 (a family's last
//! face still used by a text element) as a toast. After any change it reloads
//! the ctx font list and re-injects the generated `@font-face` stylesheet so
//! the font picker's in-face previews update.

use leptos::prelude::*;
use presenter_core::StreamFont;
use wasm_bindgen::JsCast;

use super::StreamEditorCtx;

/// The assets-panel section for uploading + managing web fonts.
#[component]
pub fn FontPanel(ctx: StreamEditorCtx) -> impl IntoView {
    let font_error = RwSignal::new(String::new());

    let on_upload = move |_| {
        let doc = crate::utils::window::document();
        let input = doc
            .query_selector("[data-role='stream-font-upload']")
            .ok()
            .flatten()
            .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok());
        let Some(input) = input else {
            return;
        };
        let Some(files) = input.files() else {
            return;
        };
        if files.length() == 0 {
            font_error.set("Vyber aspoň jeden súbor fontu (.ttf / .otf).".to_string());
            return;
        }
        // Collect the selected files up front (the input may be cleared later).
        let mut selected: Vec<web_sys::File> = Vec::new();
        for i in 0..files.length() {
            if let Some(f) = files.get(i) {
                selected.push(f);
            }
        }
        font_error.set(String::new());
        leptos::task::spawn_local(async move {
            let mut ok = 0usize;
            let mut errors: Vec<String> = Vec::new();
            for file in &selected {
                let name = file.name();
                let Ok(form) = web_sys::FormData::new() else {
                    continue;
                };
                if form.append_with_blob("file", file).is_err() {
                    continue;
                }
                match crate::api::post_form_data::<StreamFont>("/stream/fonts", &form).await {
                    Ok(_) => ok += 1,
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
            if ok > 0 {
                ctx.reload_fonts();
                // Fresh `?v=` forces the picker's @font-face previews to refresh
                // (the stylesheet is served no-cache + ETag, so it revalidates).
                crate::components::stream::fonts::ensure_fonts_css_link(js_sys::Date::now() as u64);
                ctx.show_toast(&format!("Nahraných fontov: {ok}."), "success");
            }
            if errors.is_empty() {
                font_error.set(String::new());
            } else {
                font_error.set(format!("Niektoré fonty zlyhali — {}", errors.join("; ")));
            }
        });
    };

    let face_items = move || ctx.fonts.get();

    view! {
        <div class="stream-editor__font-panel" data-role="stream-font-panel">
            <h3 class="stream-editor__section-title">"Fonty (web fonty)"</h3>
            <p class="stream-editor__hint">
                "Nahraj .ttf alebo .otf. Fonty sa servírujú z Presentera, takže sa "
                "zobrazia rovnako na každom výstupe (OBS, iný počítač)."
            </p>

            <div class="stream-editor__font-upload">
                <input
                    type="file"
                    accept=".ttf,.otf,font/ttf,font/otf"
                    multiple
                    data-role="stream-font-upload"
                />
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--primary"
                    data-role="stream-font-upload-btn"
                    on:click=on_upload
                >
                    "Nahrať fonty"
                </button>
            </div>

            <Show when=move || !font_error.get().is_empty()>
                <p class="stream-editor__prop-error" data-role="stream-font-error">
                    {move || font_error.get()}
                </p>
            </Show>

            <ul class="stream-editor__font-list" data-role="stream-font-list">
                <For
                    each=face_items
                    key=|f| f.id
                    children=move |f: StreamFont| {
                        let id = f.id;
                        let family = f.family.clone();
                        let preview_style = format!(
                            "font-family:\"{}\";font-weight:{};font-style:{};",
                            family.replace('"', "").replace('\\', ""),
                            f.weight,
                            if f.italic { "italic" } else { "normal" },
                        );
                        let label = format!(
                            "{} — {}{}",
                            family,
                            f.weight,
                            if f.italic { " kurzíva" } else { "" },
                        );
                        view! {
                            <li
                                class="stream-editor__font-item"
                                data-role="stream-font-item"
                                data-font-id=id.to_string()
                                data-font-family=family.clone()
                            >
                                <span class="stream-editor__font-preview" style=preview_style>
                                    {label}
                                </span>
                                <button
                                    type="button"
                                    class="stream-editor__btn stream-editor__btn--danger"
                                    data-role="stream-font-delete"
                                    on:click=move |_| ctx.delete_font(id)
                                >
                                    "Zmazať"
                                </button>
                            </li>
                        }
                    }
                />
                <Show when=move || ctx.fonts.get().is_empty()>
                    <li class="stream-editor__empty" data-role="stream-font-empty">
                        "Žiadne nahraté fonty. Nahraj prvý vyššie."
                    </li>
                </Show>
            </ul>
        </div>
    }
}
