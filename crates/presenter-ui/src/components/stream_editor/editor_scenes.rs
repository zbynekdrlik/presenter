//! Scene authoring UI for the stream editor (#713): the OVERLAY row on top,
//! BASE scenes as columns left→right below, and the add-scene form. Every
//! sub-component reads the live `def`/`active` signals from [`StreamEditorCtx`]
//! REACTIVELY by scene id, so a keyed `<For>` (keyed by the unique scene id,
//! per the Leptos `<For>` gotcha) never leaves a captured name/active flag
//! stale after a rename / reorder / activation.

use leptos::prelude::*;
use presenter_core::SceneKind;

use super::StreamEditorCtx;

/// Root of the editor body. Renders only once the def has loaded.
#[component]
pub fn EditorScenes(ctx: StreamEditorCtx) -> impl IntoView {
    let has_def = move || ctx.def.get().is_some();
    view! {
        <Show
            when=has_def
            fallback=|| view! { <p class="stream-editor__loading" data-role="stream-loading">"Načítavam…"</p> }
        >
            <div class="stream-editor__body" data-role="stream-editor">
                <OverlayRow ctx=ctx />
                <BaseColumns ctx=ctx />
                <AddSceneForm ctx=ctx />
            </div>
        </Show>
    }
}

/// The overlay-scene row (independent on/off toggles).
#[component]
fn OverlayRow(ctx: StreamEditorCtx) -> impl IntoView {
    let overlays = move || scene_ids_of_kind(ctx, SceneKind::Overlay);
    let empty = move || scene_ids_of_kind(ctx, SceneKind::Overlay).is_empty();
    view! {
        <section class="stream-editor__overlays" data-role="stream-overlay-row">
            <h2 class="stream-editor__section-title">"Overlay scény"</h2>
            <div class="stream-editor__overlay-list">
                <For each=overlays key=|id| *id children=move |id| view! { <OverlayChip ctx=ctx id=id /> } />
                <Show when=empty>
                    <p class="stream-editor__empty" data-role="stream-overlay-empty">"Žiadne overlay scény."</p>
                </Show>
            </div>
        </section>
    }
}

/// The base-scene columns (left→right by position; click activates exclusively).
#[component]
fn BaseColumns(ctx: StreamEditorCtx) -> impl IntoView {
    let bases = move || scene_ids_of_kind(ctx, SceneKind::Base);
    let empty = move || scene_ids_of_kind(ctx, SceneKind::Base).is_empty();
    view! {
        <section class="stream-editor__bases" data-role="stream-base-section">
            <div class="stream-editor__bases-head">
                <h2 class="stream-editor__section-title">"Base scény"</h2>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    data-role="stream-clear-base"
                    on:click=move |_| ctx.activate_base(None)
                >
                    "Vyčistiť (transparentné)"
                </button>
            </div>
            <div class="stream-editor__columns" data-role="stream-base-columns">
                <For each=bases key=|id| *id children=move |id| view! { <BaseColumn ctx=ctx id=id /> } />
                <Show when=empty>
                    <p class="stream-editor__empty" data-role="stream-base-empty">"Žiadne base scény."</p>
                </Show>
            </div>
        </section>
    }
}

/// One base scene rendered as a column: exclusive activate + shared controls.
#[component]
fn BaseColumn(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    let is_active = move || ctx.active.get().active_scene_id == Some(id);
    let active_str = move || if is_active() { "true" } else { "false" };
    view! {
        <div
            class="stream-editor__scene stream-editor__scene--base"
            data-role="stream-scene"
            data-kind="base"
            data-scene-id=id.to_string()
            data-active=active_str
        >
            <SceneHead ctx=ctx id=id />
            <button
                type="button"
                class="stream-editor__activate"
                data-role="stream-activate"
                on:click=move |_| ctx.activate_base(Some(id))
            >
                {move || if is_active() { "Aktívna" } else { "Aktivovať" }}
            </button>
            <div class="stream-editor__scene-actions">
                <SceneControls ctx=ctx id=id />
            </div>
        </div>
    }
}

/// One overlay scene rendered as a chip: on/off toggle + shared controls.
#[component]
fn OverlayChip(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    let is_active = move || ctx.active.get().active_overlay_ids.contains(&id);
    let active_str = move || if is_active() { "true" } else { "false" };
    view! {
        <div
            class="stream-editor__scene stream-editor__scene--overlay"
            data-role="stream-scene"
            data-kind="overlay"
            data-scene-id=id.to_string()
            data-active=active_str
        >
            <SceneHead ctx=ctx id=id />
            <button
                type="button"
                class="stream-editor__activate stream-editor__activate--toggle"
                data-role="stream-overlay-toggle"
                on:click=move |_| ctx.toggle_overlay(id, !is_active())
            >
                {move || if is_active() { "Vypnúť" } else { "Zapnúť" }}
            </button>
            <div class="stream-editor__scene-actions">
                <SceneControls ctx=ctx id=id />
            </div>
        </div>
    }
}

/// Scene name + inline rename + read-only element count.
#[component]
fn SceneHead(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let name = move || scene_name(ctx, id);
    let count = move || scene_element_count(ctx, id);
    let start = move |_| {
        draft.set(scene_name(ctx, id));
        editing.set(true);
    };
    let save = move |_| {
        ctx.rename_scene(id, draft.get_untracked());
        editing.set(false);
    };
    view! {
        <div class="stream-editor__scene-head">
            <Show
                when=move || editing.get()
                fallback=move || view! {
                    <span class="stream-editor__scene-name" data-role="stream-scene-name">{name}</span>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-scene-rename"
                        on:click=start
                    >
                        "Premenovať"
                    </button>
                }
            >
                <input
                    type="text"
                    class="stream-editor__rename-input"
                    data-role="stream-scene-name-input"
                    prop:value=move || draft.get()
                    on:input=move |ev| draft.set(event_target_value(&ev))
                />
                <button
                    type="button"
                    class="stream-editor__btn"
                    data-role="stream-scene-rename-save"
                    on:click=save
                >
                    "Uložiť"
                </button>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    on:click=move |_| editing.set(false)
                >
                    "Zrušiť"
                </button>
            </Show>
            <span class="stream-editor__scene-count" data-role="stream-scene-element-count">
                {move || format!("{} prvkov", count())}
            </span>
        </div>
    }
}

/// Reorder (up/down within kind) + delete controls, shared by base + overlay.
#[component]
fn SceneControls(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    view! {
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--ghost"
            data-role="stream-scene-up"
            on:click=move |_| ctx.move_scene(id, true)
        >
            "↑"
        </button>
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--ghost"
            data-role="stream-scene-down"
            on:click=move |_| ctx.move_scene(id, false)
        >
            "↓"
        </button>
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--danger"
            data-role="stream-scene-delete"
            on:click=move |_| ctx.delete_scene(id)
        >
            "Zmazať"
        </button>
    }
}

/// Add-scene form: name + kind (base/overlay).
#[component]
fn AddSceneForm(ctx: StreamEditorCtx) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let kind = RwSignal::new(String::from("base"));
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let k = if kind.get_untracked() == "overlay" {
            SceneKind::Overlay
        } else {
            SceneKind::Base
        };
        ctx.add_scene(name.get_untracked(), k);
        name.set(String::new());
    };
    view! {
        <form class="stream-editor__add" data-role="stream-add-form" on:submit=on_submit>
            <input
                type="text"
                class="stream-editor__add-name"
                data-role="stream-add-name"
                placeholder="Názov scény"
                prop:value=move || name.get()
                on:input=move |ev| name.set(event_target_value(&ev))
            />
            <select
                class="stream-editor__add-kind"
                data-role="stream-add-kind"
                prop:value=move || kind.get()
                on:change=move |ev| kind.set(event_target_value(&ev))
            >
                <option value="base">"Base scéna"</option>
                <option value="overlay">"Overlay scéna"</option>
            </select>
            <button
                type="submit"
                class="stream-editor__btn stream-editor__btn--primary"
                data-role="stream-add-submit"
            >
                "Pridať scénu"
            </button>
        </form>
    }
}

// ---- Reactive readers (track the def signal) ------------------------------

/// Scene ids of one kind, in current def order (base<overlay, then position).
fn scene_ids_of_kind(ctx: StreamEditorCtx, kind: SceneKind) -> Vec<i64> {
    ctx.def
        .get()
        .map(|d| {
            d.scenes
                .iter()
                .filter(|s| s.kind == kind)
                .map(|s| s.id)
                .collect()
        })
        .unwrap_or_default()
}

fn scene_name(ctx: StreamEditorCtx, id: i64) -> String {
    ctx.def
        .get()
        .and_then(|d| d.scenes.iter().find(|s| s.id == id).map(|s| s.name.clone()))
        .unwrap_or_default()
}

fn scene_element_count(ctx: StreamEditorCtx, id: i64) -> usize {
    ctx.def
        .get()
        .and_then(|d| {
            d.scenes
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.elements.len())
        })
        .unwrap_or(0)
}
