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
//!
//! Gesture handling (#787) is the pure [`super::gesture`] state machine: a press
//! only becomes a drag after a few px of travel (a click never moves the
//! element), the drag ends on pointerup / pointercancel / lost capture / window
//! blur / any move with no button held (it can never get stuck), Escape restores
//! the frame from the drag start, and Escape when idle or a click on empty
//! canvas deselects.

use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use presenter_core::{Frame, StreamElementDef, StreamElementProps};
use web_sys::{KeyboardEvent, PointerEvent};

use super::frame_math::{self, Handle};
use super::gesture::{EscapeAction, Gesture, GestureKind, MoveAction};
use super::props_access::read_frame;
use super::StreamEditorCtx;

/// The overlay container's `data-role` — a pointerdown whose target is the
/// container itself (not an outline/handle) is a click on EMPTY canvas.
const OVERLAY_ROLE: &str = "stream-canvas-overlay";

#[component]
pub fn CanvasOverlay(ctx: StreamEditorCtx) -> impl IntoView {
    let overlay_ref = NodeRef::<leptos::html::Div>::new();
    // Idle → Pending (pressed) → Dragging (moved past the threshold); every
    // decision lives in the host-tested `gesture` module, this only feeds it.
    let gesture = StoredValue::new(Gesture::Idle);

    let element_ids = move || scene_element_ids(ctx);

    // Shared gesture start (invoked by an outline body / a handle): remember the
    // pointer + the frame at the start (Escape restores it), capture the pointer
    // on the overlay, focus for keyboard nudge/Escape. Captures only `Copy`
    // values, so it copies into every child closure below.
    let begin = move |ev: &PointerEvent, kind: GestureKind| {
        let start = read_frame(&ctx.draft.get_untracked());
        let pt = (ev.client_x() as f64, ev.client_y() as f64);
        gesture.set_value(Gesture::begin(kind, pt, start));
        if let Some(el) = overlay_ref.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
            let _ = el.focus();
        }
        ev.prevent_default();
    };

    // End any gesture (a no-op when idle). The drag must never outlive the
    // button, so this runs on pointerup / pointercancel / lost capture.
    let finish = move |ev: Option<&PointerEvent>, why: &str| {
        end_gesture(gesture, why);
        if let (Some(ev), Some(el)) = (ev, overlay_ref.get_untracked()) {
            let _ = el.release_pointer_capture(ev.pointer_id());
        }
    };

    let on_move = move |ev: PointerEvent| {
        let pt = (ev.client_x() as f64, ev.client_y() as f64);
        let buttons = ev.buttons();
        match gesture.try_update_value(|g| g.on_move(buttons, pt)) {
            Some(MoveAction::Apply { kind, dx_px, dy_px }) => {
                apply_drag(ctx, overlay_ref, kind, (dx_px, dy_px), !ev.shift_key());
            }
            Some(MoveAction::End) => {
                leptos::logging::log!("stream canvas: gesture ended (move with no button held)");
                if let Some(el) = overlay_ref.get_untracked() {
                    let _ = el.release_pointer_capture(ev.pointer_id());
                }
            }
            Some(MoveAction::None) | None => {}
        }
    };

    // A pointerdown on EMPTY canvas (the container itself) deselects.
    let on_canvas_down = move |ev: PointerEvent| {
        if !target_is_overlay(&ev) {
            return;
        }
        if let Some(el) = overlay_ref.get_untracked() {
            let _ = el.focus();
        }
        if ctx.deselect_element() {
            leptos::logging::log!("stream canvas: empty-canvas click deselected the element");
        }
    };

    let on_key = move |ev: KeyboardEvent| {
        if ev.key() == "Escape" {
            ev.prevent_default();
            on_escape(ctx, gesture);
            return;
        }
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

    // Alt-tab / focus leaving the window mid-drag: the pointerup may never reach
    // us, so end the gesture. Removed on unmount (the overlay remounts on every
    // scene open); `WindowListenerHandle` is `Send`, so `on_cleanup` accepts it
    // in the host build too.
    let blur = window_event_listener_untyped("blur", move |_| end_gesture(gesture, "window blur"));
    on_cleanup(move || blur.remove());

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
            // Only start a gesture if the selection actually took (the dirty-switch
            // guard may decline). Seed the draft synchronously so the first
            // pointermove edits THIS element, not the previously-selected one.
            if ctx.selected_element.get_untracked() == Some(id) {
                if ctx.draft_element_id.get_untracked() != Some(id) {
                    ctx.seed_draft(id, stored_props(ctx, id));
                }
                begin(&ev, GestureKind::Move);
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
                                        begin(&ev, GestureKind::Resize(h));
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
            data-role=OVERLAY_ROLE
            node_ref=overlay_ref
            tabindex="0"
            on:pointerdown=on_canvas_down
            on:pointermove=on_move
            on:pointerup=move |ev: PointerEvent| finish(Some(&ev), "pointerup")
            on:pointercancel=move |ev: PointerEvent| finish(Some(&ev), "pointercancel")
            on:lostpointercapture=move |_: PointerEvent| finish(None, "lost pointer capture")
            on:keydown=on_key
        >
            <For each=element_ids key=|id| *id children=move |id| render_outline(id) />
        </div>
    }
}

/// End the gesture if one is active, logging why (lost capture / blur / … are
/// exactly the paths a "stuck drag" report needs to see). Tolerates a disposed
/// store (a late event after unmount).
fn end_gesture(gesture: StoredValue<Gesture>, why: &str) {
    if gesture.try_update_value(Gesture::end) == Some(true) {
        leptos::logging::log!("stream canvas: gesture ended ({why})");
    }
}

/// Escape: cancel a gesture (restore the frame it started from) or, when idle,
/// deselect the element.
fn on_escape(ctx: StreamEditorCtx, gesture: StoredValue<Gesture>) {
    match gesture.try_update_value(Gesture::escape) {
        Some(EscapeAction::Restore(frame)) => {
            leptos::logging::log!("stream canvas: Escape cancelled the gesture, frame restored");
            ctx.set_draft_frame(frame);
        }
        Some(EscapeAction::Deselect) => {
            if ctx.deselect_element() {
                leptos::logging::log!("stream canvas: Escape deselected the element");
            }
        }
        None => {}
    }
}

/// Apply one drag step (pixel delta) to the draft frame: move the body or
/// resize one handle, snapping unless Shift is held.
fn apply_drag(
    ctx: StreamEditorCtx,
    overlay_ref: NodeRef<leptos::html::Div>,
    kind: GestureKind,
    (dx_px, dy_px): (f64, f64),
    snap: bool,
) {
    let (cw, ch) = canvas_size(overlay_ref);
    let dx = frame_math::px_to_pct_delta(dx_px, cw);
    let dy = frame_math::px_to_pct_delta(dy_px, ch);
    let cur = read_frame(&ctx.draft.get_untracked());
    let targets = frame_math::snap_targets(&other_element_frames(ctx));
    let next = match kind {
        GestureKind::Move => frame_math::move_by(&cur, dx, dy, snap, &targets),
        GestureKind::Resize(h) => frame_math::resize_by(&cur, h, dx, dy, snap, &targets),
    };
    ctx.set_draft_frame(next);
}

/// True when the event's target is the overlay container itself (empty canvas),
/// not an element outline or a handle inside it.
fn target_is_overlay(ev: &PointerEvent) -> bool {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.get_attribute("data-role"))
        .is_some_and(|role| role == OVERLAY_ROLE)
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

/// Element ids of the selected scene, bottom-most first (see [`ids_bottom_to_top`]).
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
                .map(|s| ids_bottom_to_top(&s.elements))
        })
        .unwrap_or_default()
}

/// Element ids in ascending z-order (ties by id). The outlines render in this
/// order, so the TOP-most element is LAST in the DOM and the browser's own hit
/// test gives it the click: a full-canvas background underneath never swallows
/// a click meant for an element above it (#787). Sorted here rather than
/// trusting the server's order, so the contract is local.
fn ids_bottom_to_top(elements: &[StreamElementDef]) -> Vec<i64> {
    let mut keyed: Vec<(i32, i64)> = elements.iter().map(|e| (e.z_order, e.id)).collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, id)| id).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn el(id: i64, z_order: i32) -> StreamElementDef {
        StreamElementDef {
            id,
            z_order,
            props: super::super::props_access::default_element_props("color"),
        }
    }

    #[test]
    fn outlines_render_bottom_to_top_by_z_order() {
        // Scrambled input: the top-most (z 5) must come out LAST.
        let els = [el(7, 5), el(3, 0), el(9, 2)];
        assert_eq!(ids_bottom_to_top(&els), vec![3, 9, 7]);
    }

    #[test]
    fn equal_z_order_ties_break_by_id() {
        let els = [el(4, 1), el(2, 1)];
        assert_eq!(ids_bottom_to_top(&els), vec![2, 4]);
    }
}
