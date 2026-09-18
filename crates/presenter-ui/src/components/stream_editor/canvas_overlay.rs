//! Interaction overlay for direct manipulation on the preview canvas (#777).
//!
//! An absolutely-positioned layer over the preview iframe (which is set
//! `pointer-events:none` while a scene is being edited). It draws an outline per
//! element of the selected scene from def+draft frames — percent maps 1:1 because
//! the box is 16:9, the same as the output canvas — with 8 resize handles on the
//! selected one. Pointer events (with pointer capture, so a tablet works too)
//! move the body / resize a handle; arrow keys nudge (0.1 %, Shift 1 %). All
//! geometry is delegated to the pure, host-tested [`frame_math`]; every change
//! writes the shared `ctx.draft`, which the numeric fields and the preview push
//! both mirror — so the preview, the fields and the outline stay in lock-step
//! with NO save round trip.

use leptos::prelude::*;
use presenter_core::{Frame, StreamElementProps};
use web_sys::{KeyboardEvent, PointerEvent};

use super::frame_math::{self, Handle};
use super::props_access::read_frame;
use super::StreamEditorCtx;

/// The active pointer gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    None,
    Move,
    Resize(Handle),
}

#[component]
pub fn CanvasOverlay(ctx: StreamEditorCtx) -> impl IntoView {
    let overlay_ref = NodeRef::<leptos::html::Div>::new();
    let mode = StoredValue::new(DragMode::None);
    let last = StoredValue::new((0.0_f64, 0.0_f64));

    let element_ids = move || scene_element_ids(ctx);

    // Shared gesture start (invoked by an outline body / a handle): remember the
    // pointer, capture it on the overlay, focus for keyboard nudge. Captures only
    // `Copy` values, so it copies into every child closure below.
    let begin = move |ev: &PointerEvent, m: DragMode| {
        mode.set_value(m);
        last.set_value((ev.client_x() as f64, ev.client_y() as f64));
        if let Some(el) = overlay_ref.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
            let _ = el.focus();
        }
        ev.prevent_default();
    };

    let on_move = move |ev: PointerEvent| {
        let m = mode.get_value();
        if m == DragMode::None {
            return;
        }
        let (cw, ch) = canvas_size(overlay_ref);
        let (lx, ly) = last.get_value();
        let dx = frame_math::px_to_pct_delta(ev.client_x() as f64 - lx, cw);
        let dy = frame_math::px_to_pct_delta(ev.client_y() as f64 - ly, ch);
        last.set_value((ev.client_x() as f64, ev.client_y() as f64));
        let snap = !ev.shift_key();
        let cur = read_frame(&ctx.draft.get_untracked());
        let targets = frame_math::snap_targets(&other_element_frames(ctx));
        let next = match m {
            DragMode::Move => frame_math::move_by(&cur, dx, dy, snap, &targets),
            DragMode::Resize(h) => frame_math::resize_by(&cur, h, dx, dy, snap, &targets),
            DragMode::None => return,
        };
        ctx.set_draft_frame(next);
    };

    let end = move |ev: PointerEvent| {
        if mode.get_value() == DragMode::None {
            return;
        }
        mode.set_value(DragMode::None);
        if let Some(el) = overlay_ref.get_untracked() {
            let _ = el.release_pointer_capture(ev.pointer_id());
        }
    };

    let on_key = move |ev: KeyboardEvent| {
        if ctx.draft_element_id.get_untracked().is_none() {
            return;
        }
        let step = if ev.shift_key() { 1.0 } else { 0.1 };
        let (dx, dy) = match ev.key().as_str() {
            "ArrowLeft" => (-step, 0.0),
            "ArrowRight" => (step, 0.0),
            "ArrowUp" => (0.0, -step),
            "ArrowDown" => (0.0, step),
            _ => return,
        };
        ev.prevent_default();
        let cur = read_frame(&ctx.draft.get_untracked());
        ctx.set_draft_frame(frame_math::nudge(&cur, dx, dy));
    };

    // One element's outline (+ handles when selected). Read frame + selection
    // REACTIVELY by id (keyed-`<For>` gotcha), so a drag / live def refetch never
    // leaves a captured value stale.
    let render_outline = move |id: i64| {
        let selected = move || ctx.draft_element_id.get() == Some(id);
        let style = move || {
            let f = display_frame(ctx, id);
            format!(
                "left:{}%;top:{}%;width:{}%;height:{}%;",
                f.x_pct, f.y_pct, f.w_pct, f.h_pct
            )
        };
        let selected_attr = move || super::bool_attr(selected());
        let on_body_down = move |ev: PointerEvent| {
            ctx.select_element(id);
            // Only start a move if the selection actually took (the dirty-switch
            // guard may decline). Seed the draft synchronously so the first
            // pointermove edits THIS element, not the previously-selected one.
            if ctx.selected_element.get_untracked() == Some(id) {
                if ctx.draft_element_id.get_untracked() != Some(id) {
                    ctx.seed_draft(id, stored_props(ctx, id));
                }
                begin(&ev, DragMode::Move);
            }
        };
        view! {
            <div
                class="stream-editor__overlay-el"
                data-role="stream-overlay-element"
                data-element-id=id.to_string()
                data-selected=selected_attr
                style=style
                on:pointerdown=on_body_down
            >
                <Show when=selected>
                    {Handle::ALL
                        .iter()
                        .map(|h| {
                            let h = *h;
                            view! {
                                <span
                                    class="stream-editor__overlay-handle"
                                    data-role="stream-overlay-handle"
                                    data-handle=h.as_str()
                                    on:pointerdown=move |ev: PointerEvent| {
                                        ev.stop_propagation();
                                        begin(&ev, DragMode::Resize(h));
                                    }
                                ></span>
                            }
                        })
                        .collect_view()}
                </Show>
            </div>
        }
    };

    view! {
        <div
            class="stream-editor__overlay"
            data-role="stream-canvas-overlay"
            node_ref=overlay_ref
            tabindex="0"
            on:pointermove=on_move
            on:pointerup=end
            on:pointercancel=end
            on:keydown=on_key
        >
            <For each=element_ids key=|id| *id children=move |id| render_outline(id) />
        </div>
    }
}

/// Canvas box size in px, read live from the overlay element.
fn canvas_size(overlay_ref: NodeRef<leptos::html::Div>) -> (f64, f64) {
    overlay_ref
        .get_untracked()
        .map(|el| {
            let r = el.get_bounding_client_rect();
            (r.width(), r.height())
        })
        .unwrap_or((0.0, 0.0))
}

/// Element ids of the selected scene, in def (z-order) order.
fn scene_element_ids(ctx: StreamEditorCtx) -> Vec<i64> {
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

/// The stored props of one element (default image props if it has vanished).
fn stored_props(ctx: StreamEditorCtx, id: i64) -> StreamElementProps {
    ctx.def
        .get_untracked()
        .and_then(|d| {
            d.scenes
                .iter()
                .flat_map(|s| &s.elements)
                .find(|e| e.id == id)
                .map(|e| e.props.clone())
        })
        .unwrap_or_else(|| super::props_access::default_element_props("image"))
}

/// The frame to DRAW for element `id`: the live draft for the selected element,
/// otherwise the element's stored frame.
fn display_frame(ctx: StreamEditorCtx, id: i64) -> Frame {
    if ctx.draft_element_id.get() == Some(id) {
        return read_frame(&ctx.draft.get());
    }
    ctx.def
        .get()
        .and_then(|d| {
            d.scenes
                .iter()
                .flat_map(|s| &s.elements)
                .find(|e| e.id == id)
                .map(|e| read_frame(&e.props))
        })
        .unwrap_or_else(super::props_access::default_frame)
}

/// Stored frames of every element in the selected scene EXCEPT the one being
/// edited — the snap targets for a drag/resize.
fn other_element_frames(ctx: StreamEditorCtx) -> Vec<Frame> {
    let selected = ctx.draft_element_id.get_untracked();
    let Some(scene_id) = ctx.selected_scene.get_untracked() else {
        return Vec::new();
    };
    ctx.def
        .get_untracked()
        .map(|d| {
            d.scenes
                .iter()
                .find(|s| s.id == scene_id)
                .map(|s| {
                    s.elements
                        .iter()
                        .filter(|e| Some(e.id) != selected)
                        .map(|e| read_frame(&e.props))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}
