//! Slide drag-reorder drop handling + the draggable song bubble (#552/#556).
//!
//! Extracted from `slide_list.rs` (which sits near the 1000-line file cap):
//! the `.operator__slides` container's drop handler — including the #552
//! failure-visibility badge, the `slide_edit_seq` staleness guard, the
//! #556 F1 order-merge reconciliation and the F4 in-flight-typing flush /
//! focus restore — and the in-flow sticky song bubble (F5).

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

use super::slide_list_utils::reorder_slide_ids;
use super::slide_save::{
    finish_save_status_err, finish_save_status_ok, focused_slide_field_in_dom,
    reconcile_after_seq_mismatch, restore_pending_focus, save_all_fields_from_dom,
    start_save_status,
};

/// Renders the draggable song bubble above the slide list.
/// Returns a reactive closure that shows the bubble when a presentation is
/// selected, or nothing otherwise.
pub(super) fn render_song_bubble(ctx: AppContext, op: OperatorState) -> impl IntoView {
    move || {
        let presentation = ctx.selected_presentation.get();
        let Some(pres) = presentation else {
            return "".into_any();
        };
        let pres_id = pres.id.to_string();
        let pres_id_drag = pres_id.clone();
        let pres_name = pres.name.clone();
        let op_drag = op.clone();
        let op_end = op.clone();
        view! {
            <div
                class="operator__slides-bubble"
                data-role="slides-song-bubble"
                data-presentation-id=pres_id
                draggable="true"
                title="Drag into a playlist"
                on:dragstart=move |ev: web_sys::DragEvent| {
                    if let Some(dt) = ev.data_transfer() {
                        let _ = dt.set_data("text/plain", &pres_id_drag);
                        let _ = dt.set_data("application/x-presentation-id", &pres_id_drag);
                        dt.set_effect_allowed("copy");
                    }
                    op_drag.search_dragging.set(true);
                    op_drag.dragging_from_search.set(true);
                }
                on:dragend=move |_| {
                    op_end.search_dragging.set(false);
                    op_end.dragging_from_search.set(false);
                }
            >
                <span class="operator__slides-bubble-name">{pres_name}</span>
            </div>
        }
        .into_any()
    }
}

/// The `.operator__slides` container's drop handler: resolves the drop
/// target card, computes the new order, and drives the guarded reorder
/// round-trip.
///
/// #552: a reorder used to swallow any network/server error silently (no
/// `else` branch at all) — a WiFi blip on the tablet made the drop look
/// exactly like "nothing happened", with no visible failure and no way to
/// tell the drag from a no-op. It reuses the same per-slide save-status
/// badge the content-edit fields already show, keyed on the dragged slide,
/// so a failed reorder is visible instead of silent. It also bumps
/// `slide_edit_seq` the same way a content edit does (update() above the
/// async call, checked after it resolves) so a slower, overlapping EARLIER
/// drag's response can't silently clobber a newer one that already landed.
pub(super) fn handle_slide_drop(
    ev: web_sys::DragEvent,
    selected_pres: RwSignal<Option<presenter_core::Presentation>>,
    op_drop: &OperatorState,
) {
    ev.prevent_default();
    if let Some(dragged_id) = op_drop.dragging_slide_id.get_untracked() {
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                if let Some(card) = el.closest("[data-slide-id]").ok().flatten() {
                    let target_id = card.get_attribute("data-slide-id").unwrap_or_default();
                    if let Some(p) = selected_pres.get_untracked() {
                        let pres_id = p.id.to_string();
                        let ids: Vec<String> = p.slides.iter().map(|s| s.id.to_string()).collect();
                        if let Some(new_ids) = reorder_slide_ids(ids, &dragged_id, &target_id) {
                            // #556 F4: a reorder's own tracked apply below MUST
                            // rebuild the (unkeyed) slide list — the new ORDER
                            // changes which cards render where, unlike a content
                            // save. Flush whatever the user is mid-typing BEFORE
                            // that rebuild (both now, at drop time, and again
                            // right before the response is applied below) so
                            // nothing typed is silently lost, and restore focus
                            // to the same field once the DOM exists again.
                            if let Some((focus_sid, focus_field)) = focused_slide_field_in_dom() {
                                save_all_fields_from_dom(
                                    &pres_id,
                                    &focus_sid,
                                    &focus_field,
                                    0,
                                    0,
                                    selected_pres,
                                    op_drop,
                                );
                            }

                            op_drop.slide_edit_seq.update(|seq| *seq += 1);
                            let my_seq = op_drop.slide_edit_seq.get_untracked();
                            let slide_edit_seq = op_drop.slide_edit_seq;
                            let save_status = op_drop.save_status;
                            let op_for_apply = op_drop.clone();
                            let token = start_save_status(&dragged_id, save_status);
                            leptos::task::spawn_local(async move {
                                match api::presentations::reorder_slides(&pres_id, new_ids).await {
                                    Ok(slides) => {
                                        // #556 F1: on a seq mismatch, never silently
                                        // discard the response while still showing
                                        // "Saved ✓" over a stale client order/content.
                                        //
                                        // #556 F4: flush + restore focus around whichever
                                        // apply path runs below (direct or reconciled) —
                                        // BOTH are tracked updates that rebuild the DOM.
                                        let focused = focused_slide_field_in_dom();
                                        if let Some((ref fid, ref ffield)) = focused {
                                            save_all_fields_from_dom(
                                                &pres_id,
                                                fid,
                                                ffield,
                                                0,
                                                0,
                                                selected_pres,
                                                &op_for_apply,
                                            );
                                        }
                                        if slide_edit_seq.get_untracked() == my_seq {
                                            selected_pres.update(|p| {
                                                if let Some(pres) = p.as_mut() {
                                                    pres.slides = slides;
                                                }
                                            });
                                        } else {
                                            reconcile_after_seq_mismatch(
                                                pres_id,
                                                slides,
                                                selected_pres,
                                                slide_edit_seq,
                                                save_status,
                                            )
                                            .await;
                                        }
                                        if let Some((fid, ffield)) = focused {
                                            gloo_timers::callback::Timeout::new(0, move || {
                                                restore_pending_focus(&fid, &ffield);
                                            })
                                            .forget();
                                        }
                                        finish_save_status_ok(dragged_id, save_status, token).await;
                                    }
                                    Err(_) => {
                                        finish_save_status_err(dragged_id, save_status, token);
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
    }
    op_drop.dragging_slide_id.set(None);
}
