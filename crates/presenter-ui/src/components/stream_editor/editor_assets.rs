//! In-editor image-asset upload + picker for image elements (#715). Opened from
//! the image property form; uploads a PNG/JPEG/WebP via multipart to
//! `POST /stream/assets` (#708 contract, field `file`), lists
//! `GET /stream/api/assets` with served-image thumbnails, sets the draft's
//! `asset_id` on pick, and deletes via `DELETE /stream/assets/{id}` — surfacing
//! the guarded 409 (`ConflictDetail`, which names the referencing scenes)
//! inline. The asset REST surface is a PARALLEL lane (#708, not on this tree);
//! this builds against its documented shapes and reconciles at integration.

use leptos::prelude::*;
use presenter_core::{StreamAsset, StreamElementProps};
use wasm_bindgen::JsCast;

use super::StreamEditorCtx;

/// The "Vybrať obrázok" button next to the image asset_id field + the picker
/// overlay it opens.
#[component]
pub fn AssetPickerButton(
    draft: RwSignal<StreamElementProps>,
    ctx: StreamEditorCtx,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let assets = RwSignal::new(Vec::<StreamAsset>::new());
    let asset_error = RwSignal::new(String::new());

    // Re-fetch the asset list (Copy signals → this closure is Copy, reusable).
    let load = move || {
        leptos::task::spawn_local(async move {
            match crate::api::get_json::<Vec<StreamAsset>>("/stream/api/assets").await {
                Ok(list) => assets.set(list),
                Err(e) => asset_error.set(format!("Načítanie assetov zlyhalo: {e}")),
            }
        });
    };

    let on_open = move |_| {
        asset_error.set(String::new());
        open.set(true);
        load();
    };

    let on_upload = move |_| {
        let doc = crate::utils::window::document();
        let file_input = doc
            .query_selector("[data-role='stream-asset-upload']")
            .ok()
            .flatten()
            .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok());
        let Some(file_input) = file_input else {
            return;
        };
        let Some(files) = file_input.files() else {
            return;
        };
        if files.length() == 0 {
            return;
        }
        let Some(file) = files.get(0) else {
            return;
        };
        let Ok(form_data) = web_sys::FormData::new() else {
            return;
        };
        if form_data.append_with_blob("file", &file).is_err() {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::api::post_form_data::<StreamAsset>("/stream/assets", &form_data).await {
                Ok(_) => {
                    asset_error.set(String::new());
                    load();
                    ctx.show_toast("Obrázok nahraný.", "success");
                }
                Err(e) => asset_error.set(format!("Nahrávanie zlyhalo: {e}")),
            }
        });
    };

    let asset_items = move || assets.get();

    view! {
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--ghost"
            data-role="stream-asset-pick"
            on:click=on_open
        >
            "Vybrať obrázok"
        </button>

        <Show when=move || open.get()>
            <div class="stream-editor__asset-picker" data-role="stream-asset-picker">
                <div class="stream-editor__asset-picker-head">
                    <h3 class="stream-editor__section-title">"Obrázky"</h3>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-asset-picker-close"
                        on:click=move |_| open.set(false)
                    >
                        "Zavrieť"
                    </button>
                </div>

                <div class="stream-editor__asset-upload">
                    <input
                        type="file"
                        accept="image/png,image/jpeg,image/webp"
                        data-role="stream-asset-upload"
                    />
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--primary"
                        data-role="stream-asset-upload-btn"
                        on:click=on_upload
                    >
                        "Nahrať"
                    </button>
                </div>

                <Show when=move || !asset_error.get().is_empty()>
                    <p class="stream-editor__prop-error" data-role="stream-asset-error">
                        {move || asset_error.get()}
                    </p>
                </Show>

                <ul class="stream-editor__asset-list" data-role="stream-asset-list">
                    <For
                        each=asset_items
                        key=|a| a.id
                        children=move |a| {
                            let id = a.id;
                            let name = a.original_filename.clone();
                            let del = move |_| {
                                leptos::task::spawn_local(async move {
                                    match crate::api::delete_detail(&format!("/stream/assets/{id}")).await {
                                        Ok(()) => {
                                            asset_error.set(String::new());
                                            load();
                                        }
                                        Err(e) => asset_error.set(e.to_string()),
                                    }
                                });
                            };
                            view! {
                                <li
                                    class="stream-editor__asset-item"
                                    data-role="stream-asset-item"
                                    data-asset-id=id.to_string()
                                >
                                    <img
                                        class="stream-editor__asset-thumb"
                                        data-role="stream-asset-thumb"
                                        src=format!("/stream/assets/{id}")
                                        alt=name.clone()
                                    />
                                    <span class="stream-editor__asset-name">{name}</span>
                                    <div class="stream-editor__asset-actions">
                                        <button
                                            type="button"
                                            class="stream-editor__btn stream-editor__btn--primary"
                                            data-role="stream-asset-select"
                                            on:click=move |_| {
                                                draft.update(|p| {
                                                    if let StreamElementProps::Image { asset_id, .. } = p {
                                                        *asset_id = id;
                                                    }
                                                });
                                                open.set(false);
                                            }
                                        >
                                            "Vybrať"
                                        </button>
                                        <button
                                            type="button"
                                            class="stream-editor__btn stream-editor__btn--danger"
                                            data-role="stream-asset-delete"
                                            on:click=del
                                        >
                                            "Zmazať"
                                        </button>
                                    </div>
                                </li>
                            }
                        }
                    />
                    <Show when=move || assets.get().is_empty()>
                        <li class="stream-editor__empty" data-role="stream-asset-empty">
                            "Žiadne obrázky. Nahraj prvý vyššie."
                        </li>
                    </Show>
                </ul>
            </div>
        </Show>
    }
}
