//! Scene authoring UI for the stream editor (#713): the OVERLAY row on top,
//! BASE scenes as columns left→right below, and the add-scene form. Every
//! sub-component reads the live `def`/`active` signals from [`StreamEditorCtx`]
//! REACTIVELY by scene id, so a keyed `<For>` (keyed by the unique scene id,
//! per the Leptos `<For>` gotcha) never leaves a captured name/active flag
//! stale after a rename / reorder / activation.

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamOutputDef, STREAM_TRANSITION_MAX_MS};

use super::StreamEditorCtx;

/// Initial ms shown in a kind-transition input before the def loads (or when the
/// layer is currently a cut and has no stored fade value) — matches the output's
/// migration-seeded `default_transition_ms`.
const DEFAULT_TRANSITION_DRAFT_MS: u32 = 400;

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
            <div class="stream-editor__bases-head">
                <h2 class="stream-editor__section-title">"Overlay scény"</h2>
                <KindTransitionControl ctx=ctx kind=SceneKind::Overlay />
            </div>
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
                <KindTransitionControl ctx=ctx kind=SceneKind::Base />
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
    let active_str = move || super::bool_attr(is_active());
    let selected_str = move || super::bool_attr(ctx.selected_scene.get() == Some(id));
    view! {
        <div
            class="stream-editor__scene stream-editor__scene--base"
            data-role="stream-scene"
            data-kind="base"
            data-scene-id=id.to_string()
            data-active=active_str
            data-selected=selected_str
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
    let active_str = move || super::bool_attr(is_active());
    let selected_str = move || super::bool_attr(ctx.selected_scene.get() == Some(id));
    view! {
        <div
            class="stream-editor__scene stream-editor__scene--overlay"
            data-role="stream-scene"
            data-kind="overlay"
            data-scene-id=id.to_string()
            data-active=active_str
            data-selected=selected_str
        >
            <SceneHead ctx=ctx id=id />
            <button
                type="button"
                class="stream-editor__activate stream-editor__activate--toggle"
                data-role="stream-overlay-toggle"
                on:click=move |_| {
                    let now = ctx.active.get_untracked().active_overlay_ids.contains(&id);
                    ctx.toggle_overlay(id, !now);
                }
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
        let current = ctx
            .def
            .get_untracked()
            .and_then(|d| d.scenes.iter().find(|s| s.id == id).map(|s| s.name.clone()))
            .unwrap_or_default();
        draft.set(current);
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

/// Reorder (up/down within kind) + edit-elements + delete controls, shared by
/// base + overlay.
#[component]
fn SceneControls(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    view! {
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--primary"
            data-role="stream-scene-edit"
            title="Upraviť prvky scény"
            on:click=move |_| ctx.select_scene(id)
        >
            "Upraviť prvky"
        </button>
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--ghost"
            data-role="stream-scene-up"
            title="Posunúť skôr"
            on:click=move |_| ctx.move_scene(id, true)
        >
            "←"
        </button>
        <button
            type="button"
            class="stream-editor__btn stream-editor__btn--ghost"
            data-role="stream-scene-down"
            title="Posunúť neskôr"
            on:click=move |_| ctx.move_scene(id, false)
        >
            "→"
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

/// LAYER-level scene-transition control (#752): one zap/vyp toggle + čas (ms)
/// input that applies to ALL scenes of one kind at once (base OR overlay).
/// Toggle OFF stores `0` = cut (no extra boolean flag — 0 means cut per #716).
/// The number auto-commits on change; the toggle auto-commits immediately.
#[component]
fn KindTransitionControl(ctx: StreamEditorCtx, kind: SceneKind) -> impl IntoView {
    let slot = match kind {
        SceneKind::Base => "base",
        SceneKind::Overlay => "overlay",
    };
    let wrap_role = format!("stream-{slot}-transition");
    let toggle_role = format!("stream-{slot}-transition-toggle");
    let ms_role = format!("stream-{slot}-transition-ms");

    // Local draft for the ms input. A persisted fade (col > 0) always seeds it;
    // otherwise it seeds ONCE from the output's real `default_transition_ms`
    // (#752 review 🔵 — not a hardcoded guess) and is then kept across a
    // toggle-off so re-enabling restores the last value. Seeding None only once
    // (via `seeded`) avoids a reactive clobber of an in-progress edit on an
    // unrelated def refetch.
    let draft = RwSignal::new(DEFAULT_TRANSITION_DRAFT_MS);
    let seeded = RwSignal::new(false);
    Effect::new(move |_| {
        if let Some(def) = ctx.def.get() {
            match kind_transition_col(&def, kind) {
                Some(n) if n > 0 => draft.set(n),
                _ if !seeded.get_untracked() => draft.set(def.default_transition_ms),
                _ => {}
            }
            seeded.set(true);
        }
    });

    let on_toggle = move |_| {
        if kind_transition_enabled(ctx, kind, false) {
            ctx.set_kind_transition(kind, 0); // turning OFF -> cut
        } else {
            ctx.set_kind_transition(kind, draft.get_untracked().max(1)); // ON -> restore fade
        }
    };

    let on_ms_change = move |ev: leptos::ev::Event| {
        let value = event_target_value(&ev)
            .trim()
            .parse::<u32>()
            .unwrap_or(0)
            .min(STREAM_TRANSITION_MAX_MS);
        draft.set(value);
        if kind_transition_enabled(ctx, kind, false) {
            ctx.set_kind_transition(kind, value);
        }
    };

    view! {
        <div class="stream-editor__transition-ctl" data-role=wrap_role>
            <label class="stream-editor__transition-toggle">
                <input
                    type="checkbox"
                    data-role=toggle_role
                    prop:checked=move || kind_transition_enabled(ctx, kind, true)
                    on:change=on_toggle
                />
                "Prechod"
            </label>
            <input
                type="number"
                class="stream-editor__transition-ms"
                data-role=ms_role
                min="0"
                max=STREAM_TRANSITION_MAX_MS.to_string()
                prop:value=move || draft.get().to_string()
                prop:disabled=move || !kind_transition_enabled(ctx, kind, true)
                on:change=on_ms_change
            />
            <span class="stream-editor__transition-unit">"ms"</span>
        </div>
    }
}

// ---- Reactive readers (track the def signal) ------------------------------

/// The stored kind-level column for a scene kind (`None` = inherit default).
fn kind_transition_col(def: &StreamOutputDef, kind: SceneKind) -> Option<u32> {
    match kind {
        SceneKind::Base => def.base_transition_ms,
        SceneKind::Overlay => def.overlay_transition_ms,
    }
}

/// Whether the kind's transition is a FADE (enabled) rather than a cut. A `None`
/// column inherits `default_transition_ms` (a fade), so it reads as enabled;
/// `Some(0)` is a cut (disabled). `tracked` picks a reactive vs untracked read.
fn kind_transition_enabled(ctx: StreamEditorCtx, kind: SceneKind, tracked: bool) -> bool {
    let def = if tracked {
        ctx.def.get()
    } else {
        ctx.def.get_untracked()
    };
    def.map(|d| kind_transition_col(&d, kind).map(|n| n > 0).unwrap_or(true))
        .unwrap_or(true)
}

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
