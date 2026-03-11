use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Library list modal + library edit/create modal.
#[component]
pub fn LibraryModals(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let libraries = ctx.libraries;
    let open_modal_sig = op.open_modal;
    let _modal_target = op.modal_target_id;
    let op_clone = op.clone();
    let _ctx_clone = ctx.clone();

    let is_lib_list_open = move || open_modal_sig.get().as_deref() == Some("library-list");
    let is_lib_edit_open = move || {
        matches!(
            open_modal_sig.get().as_deref(),
            Some("library-edit") | Some("library-create")
        )
    };
    let edit_mode = move || match open_modal_sig.get().as_deref() == Some("library-create") {
        true => "create",
        false => "edit",
    };

    view! {
        <div class="operator__library-modal" data-role="library-modal"
            style:display=move || if is_lib_list_open() { "flex" } else { "none" }
            on:click={
                let op = op_clone.clone();
                move |ev| {
                    // Close on backdrop click
                    let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
                    if let Some(el) = target {
                        if el.class_list().contains("operator__library-modal") {
                            crate::components::modal::close_modal(&op);
                        }
                    }
                }
            }
        >
            <div class="operator__library-modal-panel">
                <header class="operator__library-modal-header">
                    <h3>"All Libraries"</h3>
                    <button
                        type="button"
                        class="operator__library-modal-close"
                        data-role="library-modal-close"
                        aria-label="Close"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::close_modal(&op)
                        }
                    >{"\u{00D7}"}</button>
                </header>
                <div class="operator__library-modal-body" data-role="library-modal-list">
                    {move || {
                        libraries.get().into_iter().map(|lib| {
                            let name = lib.name.clone();
                            view! {
                                <div class="operator__library-modal-item">{name}</div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
        </div>
        <div class="operator__library-edit" data-role="library-edit-modal"
            data-mode=edit_mode
            style:display=move || if is_lib_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="library-edit-form"
                    on:submit=move |ev| ev.prevent_default()
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="library-edit-title">
                            {move || if edit_mode() == "create" { "Create Library" } else { "Edit Library" }}
                        </h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span>"Library name"</span>
                            <input type="text" data-role="library-edit-name" autocomplete="off" required minlength="1" maxlength="120" />
                        </label>
                        <label class="operator__library-edit-favorite">
                            <input type="checkbox" data-role="library-edit-favorite" />
                            <span>"Show in dashboard"</span>
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="library-edit-delete"
                        >"Delete library"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="library-edit-cancel"
                                on:click={
                                    let op = op_clone.clone();
                                    move |_| crate::components::modal::close_modal(&op)
                                }
                            >"Cancel"</button>
                            <button type="submit" data-role="library-edit-save">"Save changes"</button>
                        </div>
                    </footer>
                </form>
            </div>
        </div>
    }
}
