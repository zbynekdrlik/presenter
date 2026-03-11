use leptos::prelude::*;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn PresentationList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let _op = use_context::<OperatorState>().expect("OperatorState");

    let select_presentation = move |id: String| {
        ctx.selected_presentation_id.set(Some(id.clone()));
        crate::state::session::set("currentPresentationId", &id);

        let id_clone = id.clone();
        leptos::task::spawn_local(async move {
            if let Ok(detail) = crate::api::presentations::get_presentation(&id_clone).await {
                let ctx = use_context::<AppContext>().expect("AppContext");
                ctx.selected_presentation.set(Some(detail.presentation));
            }
        });
    };

    let on_create = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        op.modal_mode.set("create".to_string());
        modal::open_modal(&op, "presentation-create");
    };

    view! {
        <div class="operator__presentations" data-dropzone-target="presentations">
            <div class="operator__presentations-header">
                <h2 data-role="context-title" class="operator__context-title">
                    {move || ctx.context_title.get()}
                </h2>
                <span data-role="presentation-count" class="operator__presentation-count">
                    {move || ctx.presentations.get().len().to_string()}
                </span>
                <button
                    data-role="presentation-create"
                    class="operator__presentation-create"
                    on:click=on_create
                >
                    "+"
                </button>
            </div>
            <ul data-role="presentation-list" class="operator__presentation-list">
                {move || {
                    let presentations = ctx.presentations.get();
                    let active_id = ctx.selected_presentation_id.get();

                    if presentations.is_empty() {
                        return view! {
                            <li class="empty">"Select a library or playlist..."</li>
                        }.into_any();
                    }

                    presentations.into_iter().map(|pres| {
                        let id = pres.id.to_string();
                        let name = pres.name.clone();
                        let is_active = active_id.as_deref() == Some(&id);
                        let id_for_click = id.clone();
                        let id_for_rename = id.clone();
                        let id_for_drag = id.clone();

                        view! {
                            <li>
                                <button
                                    data-role="presentation-item"
                                    data-presentation-id=id.clone()
                                    data-type="presentation"
                                    attr:data-active=move || if is_active { "true" } else { "false" }
                                    class="operator__presentation-btn"
                                    draggable="true"
                                    on:click={
                                        let id = id_for_click.clone();
                                        move |_| select_presentation(id.clone())
                                    }
                                    on:dragstart=move |ev: web_sys::DragEvent| {
                                        if let Some(dt) = ev.data_transfer() {
                                            let _ = dt.set_data("text/plain", &id_for_drag);
                                            let _ = dt.set_data("application/x-presentation-id", &id_for_drag);
                                        }
                                    }
                                >
                                    <span class="operator__presentation-label">{name}</span>
                                </button>
                                <button
                                    data-action="presentation-rename"
                                    data-presentation-id=id_for_rename.clone()
                                    class="operator__presentation-rename"
                                    on:click=move |ev: leptos::ev::MouseEvent| {
                                        ev.stop_propagation();
                                        let op = use_context::<OperatorState>().expect("OperatorState");
                                        op.modal_mode.set("edit".to_string());
                                        op.modal_target_id.set(Some(id_for_rename.clone()));
                                        modal::open_modal(&op, "presentation-edit");
                                    }
                                >
                                    "\u{270e}"
                                </button>
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}
