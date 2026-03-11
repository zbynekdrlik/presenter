use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Playlist list modal + playlist edit/create modal.
#[component]
pub fn PlaylistModals(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let playlists = ctx.playlists;
    let open_modal_sig = op.open_modal;
    let op_clone = op.clone();

    let is_pl_list_open = move || open_modal_sig.get().as_deref() == Some("playlist-list");
    let is_pl_edit_open = move || {
        matches!(
            open_modal_sig.get().as_deref(),
            Some("playlist-edit") | Some("playlist-create")
        )
    };
    let edit_mode = move || match open_modal_sig.get().as_deref() == Some("playlist-create") {
        true => "create",
        false => "edit",
    };

    view! {
        <div class="operator__playlist-modal" data-role="playlist-modal"
            style:display=move || if is_pl_list_open() { "flex" } else { "none" }
            on:click={
                let op = op_clone.clone();
                move |ev| {
                    let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
                    if let Some(el) = target {
                        if el.class_list().contains("operator__playlist-modal") {
                            crate::components::modal::close_modal(&op);
                        }
                    }
                }
            }
        >
            <div class="operator__playlist-modal-panel">
                <header class="operator__playlist-modal-header">
                    <h3>"All Playlists"</h3>
                    <button
                        type="button"
                        class="operator__playlist-modal-close"
                        data-role="playlist-modal-close"
                        aria-label="Close"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::close_modal(&op)
                        }
                    >{"\u{00D7}"}</button>
                </header>
                <div class="operator__playlist-modal-body" data-role="playlist-modal-list">
                    {move || {
                        playlists.get().into_iter().map(|pl| {
                            let name = pl.name.clone();
                            view! {
                                <div class="operator__playlist-modal-item">{name}</div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
        </div>
        <div class="operator__library-edit operator__playlist-edit" data-role="playlist-edit-modal"
            data-mode=edit_mode
            style:display=move || if is_pl_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="playlist-edit-form"
                    on:submit=move |ev| ev.prevent_default()
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="playlist-edit-title">
                            {move || if edit_mode() == "create" { "Create Playlist" } else { "Edit Playlist" }}
                        </h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span>"Playlist name"</span>
                            <input type="text" data-role="playlist-edit-name" autocomplete="off" required minlength="1" maxlength="160" />
                        </label>
                        <label class="operator__library-edit-favorite">
                            <input type="checkbox" data-role="playlist-edit-dashboard" />
                            <span>"Show in dashboard"</span>
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="playlist-edit-delete"
                        >"Delete playlist"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="playlist-edit-cancel"
                                on:click={
                                    let op = op_clone.clone();
                                    move |_| crate::components::modal::close_modal(&op)
                                }
                            >"Cancel"</button>
                            <button type="submit" data-role="playlist-edit-save">"Save changes"</button>
                        </div>
                    </footer>
                </form>
            </div>
        </div>
    }
}
