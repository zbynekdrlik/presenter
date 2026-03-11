use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Presentation create/edit/rename modals.
#[component]
pub fn PresentationModals(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let _ctx = ctx;
    let open_modal_sig = op.open_modal;
    let op_clone = op.clone();

    let is_edit_open = move || open_modal_sig.get().as_deref() == Some("presentation-edit");
    let is_create_open = move || open_modal_sig.get().as_deref() == Some("presentation-create");

    view! {
        <div class="operator__library-edit operator__presentation-edit" data-role="presentation-edit-modal" data-mode="presentation"
            style:display=move || if is_edit_open() { "flex" } else { "none" }
        >
            <div class="operator__library-edit-panel">
                <form class="operator__library-edit-form" data-role="presentation-edit-form"
                    on:submit=move |ev| ev.prevent_default()
                >
                    <header class="operator__library-edit-header">
                        <h3 data-role="presentation-edit-title">"Rename Presentation"</h3>
                    </header>
                    <div class="operator__library-edit-body">
                        <label>
                            <span data-role="presentation-edit-label">"Presentation name"</span>
                            <input type="text" data-role="presentation-edit-name" autocomplete="off" required minlength="1" maxlength="160" />
                        </label>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <button
                            type="button"
                            class="operator__library-edit-delete"
                            data-role="presentation-edit-delete"
                        >"Delete presentation"</button>
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="presentation-edit-cancel"
                                on:click={
                                    let op = op_clone.clone();
                                    move |_| crate::components::modal::close_modal(&op)
                                }
                            >"Cancel"</button>
                            <button type="submit" data-role="presentation-edit-save">"Save changes"</button>
                        </div>
                    </footer>
                </form>
            </div>
        </div>
        <div class="operator__library-edit operator__presentation-create" data-role="presentation-create-modal"
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
                            <input type="text" data-role="presentation-create-name" autocomplete="off" maxlength="160" placeholder="New Presentation" />
                        </label>
                        <div class="operator__presentation-create-options" data-role="presentation-create-options">
                            <button type="button" class="operator__create-option" data-role="presentation-create-blank">
                                <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4C4}"}</span>
                                <span class="operator__create-option-label">"Blank"</span>
                                <span class="operator__create-option-desc">"Empty presentation"</span>
                            </button>
                            <button type="button" class="operator__create-option" data-role="presentation-create-paste">
                                <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4CB}"}</span>
                                <span class="operator__create-option-label">"Paste"</span>
                                <span class="operator__create-option-desc">"Paste song text"</span>
                            </button>
                            <button type="button" class="operator__create-option" data-role="presentation-create-import">
                                <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4C1}"}</span>
                                <span class="operator__create-option-label">"Import"</span>
                                <span class="operator__create-option-desc">".pro file"</span>
                            </button>
                        </div>
                        <div class="operator__presentation-create-paste-area" data-role="presentation-create-paste-area" style="display:none">
                            <textarea data-role="presentation-create-paste-text" rows="10" placeholder="Paste song text here..."></textarea>
                            <div class="operator__presentation-create-sub-actions">
                                <button type="button" data-role="presentation-create-paste-back">"Back"</button>
                                <button type="button" data-role="presentation-create-paste-confirm">"Create"</button>
                            </div>
                        </div>
                        <div class="operator__presentation-create-import-area" data-role="presentation-create-import-area" style="display:none">
                            <input type="file" data-role="presentation-create-import-file" accept=".pro" />
                            <div class="operator__presentation-create-sub-actions">
                                <button type="button" data-role="presentation-create-import-back">"Back"</button>
                                <button type="button" data-role="presentation-create-import-confirm">"Import"</button>
                            </div>
                        </div>
                    </div>
                    <footer class="operator__library-edit-footer">
                        <div class="operator__library-edit-actions">
                            <button type="button" data-role="presentation-create-cancel"
                                on:click={
                                    let op = op_clone.clone();
                                    move |_| crate::components::modal::close_modal(&op)
                                }
                            >"Cancel"</button>
                        </div>
                    </footer>
                </div>
            </div>
        </div>
    }
}
