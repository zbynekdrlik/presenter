//! Element panel for the selected scene (#714): the element list (add one of
//! each kind with sensible defaults, select for editing, z-order up/down,
//! delete-with-confirm) plus the hosted property form for the selected element.
//! Every row reads its kind/selection REACTIVELY by the unique element id
//! (keyed `<For>` gotcha), so a rename/reorder/refetch never leaves a captured
//! value stale.

use leptos::prelude::*;

use super::element_form::ElementForm;
use super::props_access::default_element_props;
use super::StreamEditorCtx;

/// The panel body — rendered by the page only when a scene is selected.
#[component]
pub fn EditorPanel(ctx: StreamEditorCtx) -> impl IntoView {
    let scene_name = move || selected_scene_name(ctx);
    let element_ids = move || selected_element_ids(ctx);
    let empty = move || selected_element_ids(ctx).is_empty();
    view! {
        <section class="stream-editor__panel" data-role="stream-element-panel">
            <header class="stream-editor__panel-head">
                <h2 class="stream-editor__section-title">
                    {move || format!("Prvky scény: {}", scene_name())}
                </h2>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    data-role="stream-panel-close"
                    on:click=move |_| ctx.close_panel()
                >
                    "Zavrieť"
                </button>
            </header>

            <div class="stream-editor__add-elements" data-role="stream-add-elements">
                <AddElementButton ctx=ctx kind="image" label="+ Obrázok" />
                <AddElementButton ctx=ctx kind="countdown" label="+ Odpočet" />
                <AddElementButton ctx=ctx kind="lyrics" label="+ Text piesne" />
                <AddElementButton ctx=ctx kind="verse" label="+ Verš" />
            </div>

            <ul class="stream-editor__element-list" data-role="stream-element-list">
                <For
                    each=element_ids
                    key=|id| *id
                    children=move |id| view! { <ElementRow ctx=ctx id=id /> }
                />
                <Show when=empty>
                    <li class="stream-editor__empty" data-role="stream-element-empty">
                        "Žiadne prvky. Pridaj prvý vyššie."
                    </li>
                </Show>
            </ul>

            <Show when=move || ctx.selected_element.get().is_some()>
                <ElementForm ctx=ctx />
            </Show>
        </section>
    }
}

/// One "add element of kind" button, seeding sensible default props.
#[component]
fn AddElementButton(
    ctx: StreamEditorCtx,
    kind: &'static str,
    label: &'static str,
) -> impl IntoView {
    let role = format!("stream-add-element-{kind}");
    view! {
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--primary"
            data-role=role
            on:click=move |_| {
                if let Some(scene_id) = ctx.selected_scene.get_untracked() {
                    ctx.add_element(scene_id, default_element_props(kind));
                }
            }
        >
            {label}
        </button>
    }
}

/// One element row: kind label + select / z-order up-down / delete.
#[component]
fn ElementRow(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    let kind = move || element_kind(ctx, id);
    let kind_label = move || kind_label_sk(&element_kind(ctx, id));
    let selected_str = move || super::bool_attr(ctx.selected_element.get() == Some(id));
    view! {
        <li
            class="stream-editor__element"
            data-role="stream-element"
            data-element-id=id.to_string()
            data-kind=kind
            data-selected=selected_str
        >
            <button
                type="button"
                class="stream-editor__element-select"
                data-role="stream-element-select"
                on:click=move |_| ctx.select_element(id)
            >
                {kind_label}
            </button>
            <div class="stream-editor__element-actions">
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    data-role="stream-element-up"
                    title="Vyššie v poradí"
                    on:click=move |_| ctx.move_element(id, true)
                >
                    "↑"
                </button>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    data-role="stream-element-down"
                    title="Nižšie v poradí"
                    on:click=move |_| ctx.move_element(id, false)
                >
                    "↓"
                </button>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--danger"
                    data-role="stream-element-delete"
                    on:click=move |_| ctx.delete_element(id)
                >
                    "Zmazať"
                </button>
            </div>
        </li>
    }
}

// ---- reactive readers (track the def signal) ------------------------------

fn selected_scene_name(ctx: StreamEditorCtx) -> String {
    let Some(scene_id) = ctx.selected_scene.get() else {
        return String::new();
    };
    ctx.def
        .get()
        .and_then(|d| {
            d.scenes
                .iter()
                .find(|s| s.id == scene_id)
                .map(|s| s.name.clone())
        })
        .unwrap_or_default()
}

/// Element ids of the selected scene, in def (z_order) order.
fn selected_element_ids(ctx: StreamEditorCtx) -> Vec<i64> {
    let Some(scene_id) = ctx.selected_scene.get() else {
        return Vec::new();
    };
    ctx.def
        .get()
        .and_then(|d| {
            d.scenes
                .iter()
                .find(|s| s.id == scene_id)
                .map(|s| s.elements.iter().map(|e| e.id).collect())
        })
        .unwrap_or_default()
}

fn element_kind(ctx: StreamEditorCtx, id: i64) -> String {
    ctx.def
        .get()
        .and_then(|d| {
            d.scenes
                .iter()
                .flat_map(|s| &s.elements)
                .find(|e| e.id == id)
                .map(|e| e.props.kind_str().to_string())
        })
        .unwrap_or_default()
}

fn kind_label_sk(kind: &str) -> &'static str {
    match kind {
        "image" => "Obrázok",
        "countdown" => "Odpočet",
        "lyrics" => "Text piesne",
        "verse" => "Verš",
        _ => "Prvok",
    }
}
