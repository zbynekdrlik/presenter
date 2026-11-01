//! Poradie (spec §7.6): the items as rows, moved by a drag on the handle or with
//! ▲ ▼. Leaving the sheet sends the full id list once, through the sermon's
//! queue. Drag = pointer events on a `touch-action: none` handle, as the plate
//! list's (`components/nameplates/drag.rs`); the decisions are `order`'s.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

use super::sermon_editor::{refuse, run_write, Editor, OrderDraft, WriteEnd};
use super::sheet::Sheet;
use crate::api::sermons;
use crate::components::nameplates::order::{
    drop_marker, drop_slot, reorder_one_step, reorder_to_slot,
};

const PRIMARY_BUTTON: i16 = 0;

const ORDER_REFUSED: &str = "Poradie sa medzitým zmenilo — skús znova";

pub(super) fn order_moved(e: Editor) -> bool {
    e.order
        .with_untracked(|draft| draft.as_ref().is_some_and(|draft| draft.ids != draft.base))
}

/// The order goes out once, as Poradie leaves. The rows started from `base`: a
/// sermon reordered since was seen by the socket, so its revision would pass,
/// and this list would silently undo that order. A failure keeps the order for
/// the retry button; a sheet still open keeps collecting moves.
pub(super) fn send_order(e: Editor) {
    let Some(draft) = e.order.get_untracked() else {
        return;
    };
    if e.order_sending.get_value() {
        return;
    }
    if draft.ids == draft.base {
        e.order.set(None);
        e.order_failed.set(false);
        return;
    }
    if e.held_order() != draft.base {
        e.order.set(None);
        e.order_failed.set(false);
        refuse(e, ORDER_REFUSED);
        return;
    }
    e.order_sending.set_value(true);
    leptos::task::spawn_local(async move {
        let ids = draft.ids;
        let end = run_write(e, ORDER_REFUSED, move |id, revision| async move {
            sermons::reorder_items(&id, revision, &ids).await
        })
        .await;
        e.order_sending.set_value(false);
        e.order_failed.set(end == WriteEnd::Failed);
        if end != WriteEnd::Failed {
            e.order.set(None);
        }
    });
}

fn handle_of(ev: &PointerEvent) -> Option<web_sys::Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())?
        .closest("[data-role=\"sermon-drag-handle\"]")
        .ok()
        .flatten()
}

/// The slot under the pointer, measured on the rows on screen.
fn slot_under(ev: &PointerEvent) -> Option<usize> {
    let list = handle_of(ev)?
        .closest("[data-role=\"sermon-order\"]")
        .ok()
        .flatten()?;
    let rows = list
        .query_selector_all(":scope > [data-role=\"sermon-order-row\"]")
        .ok()?;
    let middles: Vec<f64> = (0..rows.length())
        .filter_map(|i| rows.item(i))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .map(|row| {
            let rect = row.get_bounding_client_rect();
            rect.top() + rect.height() / 2.0
        })
        .collect();
    Some(drop_slot(&middles, f64::from(ev.client_y())))
}

fn ids_now(e: Editor) -> Vec<String> {
    e.order
        .with_untracked(|draft| draft.as_ref().map(|held| held.ids.clone()))
        .unwrap_or_default()
}

/// The rows move: the draft takes the new order.
fn set_ids(e: Editor, ids: Vec<String>) {
    e.order.update(|draft| {
        if let Some(draft) = draft {
            draft.ids = ids;
        }
    });
}

#[component]
fn OrderRow(e: Editor, id: String, drag: RwSignal<Option<(String, usize)>>) -> impl IntoView {
    let label = e
        .item(&id)
        .map(|item| {
            e.drafts.with_untracked(|drafts| {
                drafts.get(&id).map_or_else(
                    || item.book.clone(),
                    |draft| draft.label(&item.book, item.chapter, &item.translation),
                )
            })
        })
        .unwrap_or_default();
    let position = {
        let id = id.clone();
        Memo::new(move |_| {
            e.order
                .with(|draft| draft.as_ref()?.ids.iter().position(|held| *held == id))
        })
    };
    let count = Memo::new(move |_| {
        e.order
            .with(|draft| draft.as_ref().map_or(0, |held| held.ids.len()))
    });
    let marker = move || {
        let (dragged, slot) = drag.get()?;
        let from = ids_now(e).iter().position(|held| *held == dragged)?;
        drop_marker(from, slot, position.get()?, count.get())
    };
    let step = {
        let id = id.clone();
        move |up: bool| {
            if let Some(moved) = reorder_one_step(&ids_now(e), &id, up) {
                set_ids(e, moved);
            }
        }
    };
    let step_up = step.clone();
    let dragged_id = id.clone();
    let row_id = id.clone();
    let on_down = move |ev: PointerEvent| {
        if ev.button() != PRIMARY_BUTTON {
            return;
        }
        let Some(handle) = handle_of(&ev) else {
            return;
        };
        ev.prevent_default();
        let _ = handle.set_pointer_capture(ev.pointer_id());
        let slot = position.get_untracked().unwrap_or(0);
        drag.set(Some((dragged_id.clone(), slot)));
    };
    let on_move = move |ev: PointerEvent| {
        if ev.buttons() == 0 {
            drag.set(None);
            return;
        }
        if let Some(slot) = slot_under(&ev) {
            drag.update(|held| {
                if let Some(held) = held {
                    held.1 = slot;
                }
            });
        }
    };
    let on_up = move |_: PointerEvent| {
        let Some((id, slot)) = drag.get_untracked() else {
            return;
        };
        drag.set(None);
        if let Some(moved) = reorder_to_slot(&ids_now(e), &id, slot) {
            set_ids(e, moved);
        }
    };
    view! {
        <div
            class="sermon-order__row"
            data-role="sermon-order-row"
            data-item-id=row_id
            data-drop=marker
            data-dragging=move || {
                drag.with(|held| held.as_ref().is_some_and(|(dragged, _)| *dragged == id))
                    .then_some("true")
            }
        >
            <button
                type="button"
                class="sermon-order__handle"
                data-role="sermon-drag-handle"
                aria-label="Presunúť položku"
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=move |_| drag.set(None)
                on:lostpointercapture=move |_| drag.set(None)
            >
                "\u{2261}"
            </button>
            <span class="sermon-order__label">{label}</span>
            <button
                type="button"
                class="study-icon"
                data-role="sermon-order-up"
                aria-label="Vyššie"
                disabled=move || position.get() == Some(0)
                on:click=move |_| step_up(true)
            >
                "\u{25B2}"
            </button>
            <button
                type="button"
                class="study-icon"
                data-role="sermon-order-down"
                aria-label="Nižšie"
                disabled=move || position.get().is_some_and(|at| at + 1 == count.get())
                on:click=move |_| step(false)
            >
                "\u{25BC}"
            </button>
        </div>
    }
}

#[component]
pub(super) fn ReorderSheet(e: Editor) -> impl IntoView {
    let drag = RwSignal::new(None);
    // A sheet opened from its URL is here before the sermon; a failed order
    // waits here to be moved on.
    Effect::new(move |_| {
        if e.order.with_untracked(Option::is_none) && e.detail.with(Option::is_some) {
            let base = e.held_order();
            e.order.set(Some(OrderDraft {
                ids: base.clone(),
                base,
            }));
        }
    });
    Owner::on_cleanup(move || send_order(e));
    let shown = move || {
        e.order
            .with(|draft| draft.as_ref().map(|held| held.ids.clone()))
            .unwrap_or_default()
    };
    let empty = move || {
        e.order
            .with(|draft| draft.as_ref().is_some_and(|held| held.ids.is_empty()))
    };
    view! {
        <Sheet s=e.s role="sermon-reorder" modal=true title=Signal::derive(|| "Poradie".to_string())>
            {move || {
                empty()
                    .then(|| {
                        view! {
                            <p class="study-empty" data-role="sermon-order-empty">
                                "Zatiaľ žiadne položky"
                            </p>
                        }
                    })
            }}
            <div class="sermon-order" data-role="sermon-order">
                <For
                    each=shown
                    key=|id: &String| id.clone()
                    children=move |id| view! { <OrderRow e=e id=id drag=drag /> }
                />
            </div>
        </Sheet>
    }
}
