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
        <div class="operator__catalog-bottom" data-role="catalog-bottom" data-dropzone-target="presentations">
            <header class="operator__group-header operator__presentations-header">
                <h2 data-role="context-title">
                    {move || ctx.context_title.get()}
                </h2>
                <div class="operator__group-controls">
                    <span class="operator__group-count operator__group-count--static" data-role="presentation-count">
                        {move || {
                            let count = ctx.presentations.get().len();
                            if count > 0 { count.to_string() } else { "\u{2014}".to_string() }
                        }}
                    </span>
                    <button
                        type="button"
                        data-role="presentation-create"
                        aria-label="Add presentation or separator"
                        title="Add"
                        on:click=on_create
                    >
                        "+"
                    </button>
                </div>
            </header>
            <ul class="operator__presentation-list" data-role="presentation-list">
                {move || {
                    let presentations = ctx.presentations.get();
                    let active_id = ctx.selected_presentation_id.get();
                    let mode = ctx.mode.get();
                    let is_edit = mode == "edit";
                    let stage_pres_id = ctx.stage_snapshot.get()
                        .and_then(|s| s.presentation_id.map(|id| id.to_string()));

                    if presentations.is_empty() {
                        return view! {
                            <li class="empty">"Select a library or playlist to view presentations."</li>
                        }.into_any();
                    }

                    presentations.into_iter().map(|pres| {
                        let id = pres.id.to_string();
                        let name = pres.name.clone();
                        let is_active = active_id.as_deref() == Some(&id);
                        let is_stage_active = stage_pres_id.as_deref() == Some(&id);
                        let id_for_click = id.clone();
                        let id_for_rename = id.clone();
                        let id_for_drag = id.clone();
                        let id_for_li = id.clone();

                        // Get library name from context if available
                        let lib_name = ctx.context_title.get();

                        view! {
                            <li
                                class=move || {
                                    let mut c = "operator__presentation-item".to_string();
                                    if is_active { c.push_str(" is-active"); }
                                    if is_stage_active { c.push_str(" is-stage-active"); }
                                    c
                                }
                                data-role="presentation-item"
                                data-type="presentation"
                                data-presentation-id=id_for_li
                                attr:data-active=move || if is_active { "true" } else { "false" }
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
                                <span>{name}</span>
                                <span class="operator__presentation-meta">{lib_name}</span>
                                {is_edit.then(|| view! {
                                    <div class="operator__presentation-actions">
                                        <button
                                            type="button"
                                            class="operator__presentation-action"
                                            data-action="presentation-rename"
                                            data-presentation-id=id_for_rename.clone()
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
                                    </div>
                                })}
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}
