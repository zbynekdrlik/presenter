//! The Prepared tab's presentation list (#874): the operator's own order
//! (drag a card by its handle, or the arrow keys on the handle) and the
//! archive under it (Obnoviť / Zmazať natrvalo). Archiving replaces deleting
//! an active presentation; only an archived one can be deleted for good.
//!
//! The order logic is the plate list's (`nameplates::order`, generic): the
//! server wants the FULL active id set, so every new order holds every id
//! once. The drag uses POINTER events on a `touch-action: none` handle, the
//! repo's touch-capable pattern — HTML5 drag and drop never fires on a phone.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{KeyboardEvent, MouseEvent, PointerEvent};

use crate::api::bible::{self, BiblePresentationSummary};
use crate::components::nameplates::order::{
    drop_marker, drop_slot, reorder_one_step, reorder_to_slot,
};
use crate::pages::bible_prepared_sermon::sermon_badge;
use crate::state::bible::{BibleState, PresentationDrag};
use crate::state::AppContext;

/// `MouseEvent.button` of the primary button (left mouse, a finger, a pen).
const PRIMARY_BUTTON: i16 = 0;

/// Re-read both lists: the active one in its order and the archive.
pub(crate) async fn reload_presentation_lists(bs: BibleState) {
    if let Ok(list) = bible::list_presentations().await {
        bs.presentations.set(list);
    }
    if let Ok(list) = bible::list_archived_presentations().await {
        bs.archived_presentations.set(list);
    }
}

fn toast(ctx: &AppContext, variant: &str, message: String) {
    ctx.toast_variant.set(variant.to_string());
    ctx.toast_message.set(Some(message));
}

/// Show `ids` as the new order at once and store it. A refusal (another
/// device changed the list) shows the server's list again.
fn save_order(bs: BibleState, ctx: AppContext, ids: Vec<String>) {
    bs.presentations.update(|list| {
        list.sort_by_key(|p| ids.iter().position(|id| *id == p.id).unwrap_or(usize::MAX));
    });
    leptos::task::spawn_local(async move {
        if let Err(e) = bible::reorder_presentations(&ids).await {
            toast(&ctx, "error", format!("Poradie sa neuložilo: {e}"));
            reload_presentation_lists(bs).await;
        }
    });
}

fn current_ids(bs: BibleState) -> Vec<String> {
    bs.presentations
        .with_untracked(|list| list.iter().map(|p| p.id.clone()).collect())
}

/// The drag handle a pointer event concerns.
fn handle_of(ev: &PointerEvent) -> Option<web_sys::Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())?
        .closest("[data-role=\"presentation-drag\"]")
        .ok()
        .flatten()
}

/// The drop slot under the pointer, measured on the active list's cards.
fn slot_under_pointer(ev: &PointerEvent) -> Option<usize> {
    let list = handle_of(ev)?
        .closest("[data-role=\"presentations-list\"]")
        .ok()
        .flatten()?;
    let cards = list
        .query_selector_all(":scope > [data-role=\"presentation-card\"]")
        .ok()?;
    let middles: Vec<f64> = (0..cards.length())
        .filter_map(|i| cards.item(i))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .map(|card| {
            let rect = card.get_bounding_client_rect();
            rect.top() + rect.height() / 2.0
        })
        .collect();
    Some(drop_slot(&middles, f64::from(ev.client_y())))
}

/// The active list: one draggable card per presentation.
#[component]
pub(crate) fn PresentationList() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let presentations = bs.presentations;
    view! {
        <div class="bible__prepared-list" data-role="presentations-list">
            {move || {
                let list = presentations.get();
                if list.is_empty() {
                    view! {
                        <p class="operator__slides-empty">"Zatiaľ žiadne prezentácie."</p>
                    }
                    .into_any()
                } else {
                    list.into_iter()
                        .map(|p| view! { <PresentationCard presentation=p /> })
                        .collect_view()
                        .into_any()
                }
            }}
        </div>
    }
}

/// Open a presentation: its slides go to the slide column.
fn open_presentation(bs: BibleState, id: String) {
    let active_id = bs.active_presentation_id;
    let active_slides = bs.active_presentation_slides;
    // Cards of the previous presentation must not stay clickable under the
    // new id: the server would refuse their slide ids.
    if active_id.get_untracked().as_ref() != Some(&id) {
        active_slides.set(Vec::new());
    }
    active_id.set(Some(id.clone()));
    leptos::task::spawn_local(async move {
        let answer = bible::get_presentation(&id).await;
        if active_id.get_untracked().as_ref() != Some(&id) {
            return;
        }
        if let Ok(detail) = answer {
            active_slides.set(detail.slides);
        }
    });
}

#[component]
fn PresentationCard(presentation: BiblePresentationSummary) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let drag = bs.presentation_drag;
    let id = presentation.id.clone();
    let name = presentation.name.clone();
    let count = presentation.slide_count;
    let badge = sermon_badge(&presentation);

    // The card's place, read from the list as it is NOW (never captured).
    let position = {
        let id = id.clone();
        Memo::new(move |_| {
            bs.presentations
                .with(|list| list.iter().position(|p| p.id == id))
        })
    };
    let is_active = {
        let id = id.clone();
        move || bs.active_presentation_id.get().as_ref() == Some(&id)
    };
    let dragging = {
        let id = id.clone();
        move || drag.with(|d| d.as_ref().is_some_and(|d| d.id == id))
    };
    let marker = {
        let id = id.clone();
        move || {
            let d = drag.get().filter(|d| d.id != id)?;
            let pos = position.get()?;
            bs.presentations.with(|list| {
                let from = list.iter().position(|p| p.id == d.id)?;
                drop_marker(from, d.slot, pos, list.len())
            })
        }
    };

    let on_click = {
        let id = id.clone();
        move |_| open_presentation(bs, id.clone())
    };
    let on_edit = {
        let id = id.clone();
        let name = name.clone();
        move |ev: MouseEvent| {
            ev.stop_propagation();
            bs.modal_presentation_id.set(Some(id.clone()));
            bs.modal_presentation_name.set(name.clone());
        }
    };
    let on_down = {
        let id = id.clone();
        move |ev: PointerEvent| {
            if ev.button() != PRIMARY_BUTTON {
                return;
            }
            let Some(handle) = handle_of(&ev) else {
                return;
            };
            let _ = handle.set_pointer_capture(ev.pointer_id());
            ev.prevent_default();
            if let Some(slot) = position.get_untracked() {
                drag.set(Some(PresentationDrag {
                    id: id.clone(),
                    slot,
                }));
            }
        }
    };
    let on_move = {
        let id = id.clone();
        move |ev: PointerEvent| {
            let Some(current) = drag.get_untracked().filter(|d| d.id == id) else {
                return;
            };
            // No button held any more (a lost release): the drag ends.
            if ev.buttons() == 0 {
                drag.set(None);
                return;
            }
            if let Some(slot) = slot_under_pointer(&ev) {
                if slot != current.slot {
                    drag.set(Some(PresentationDrag { slot, ..current }));
                }
            }
        }
    };
    let on_up = {
        let id = id.clone();
        let ctx = ctx.clone();
        move |ev: PointerEvent| {
            let Some(current) = drag.get_untracked().filter(|d| d.id == id) else {
                return;
            };
            drag.set(None);
            let slot = slot_under_pointer(&ev).unwrap_or(current.slot);
            if let Some(ids) = reorder_to_slot(&current_ids(bs), &id, slot) {
                save_order(bs, ctx.clone(), ids);
            }
        }
    };
    let on_cancel = {
        let id = id.clone();
        move |_: PointerEvent| {
            if drag.with_untracked(|d| d.as_ref().is_some_and(|d| d.id == id)) {
                drag.set(None);
            }
        }
    };
    let on_key = {
        let id = id.clone();
        move |ev: KeyboardEvent| {
            let up = match ev.key().as_str() {
                "ArrowUp" => true,
                "ArrowDown" => false,
                _ => return,
            };
            ev.prevent_default();
            if let Some(ids) = reorder_one_step(&current_ids(bs), &id, up) {
                save_order(bs, ctx.clone(), ids);
            }
        }
    };

    view! {
        <div
            class="operator__presentation-card"
            class:is-active=is_active
            data-presentation-id=id.clone()
            data-role="presentation-card"
            data-dragging=move || if dragging() { "true" } else { "false" }
            data-drop=marker
            on:click=on_click
        >
            <div class="bible__presentation-head">
                <button
                    type="button"
                    class="operator__presentation-action bible__presentation-handle"
                    data-role="presentation-drag"
                    title="Ťahaním presuniete prezentáciu (alebo šípkami hore / dole)"
                    aria-label="Presunúť prezentáciu"
                    on:click=|ev: MouseEvent| ev.stop_propagation()
                    on:pointerdown=on_down
                    on:pointermove=on_move
                    on:pointerup=on_up
                    on:pointercancel=on_cancel.clone()
                    on:lostpointercapture=on_cancel
                    on:keydown=on_key
                >"⠿"</button>
                <strong class="bible__presentation-name">{name.clone()}</strong>
                <button
                    type="button"
                    class="operator__presentation-action"
                    data-role="presentation-edit"
                    on:click=on_edit
                    title="Upraviť prezentáciu"
                >"\u{270F}\u{FE0F}"</button>
            </div>
            {badge.map(|badge| view! { <p data-role="presentation-sermon-badge">{badge}</p> })}
            <p>{slide_count_label(count)}</p>
        </div>
    }
}

/// "1 snímka", "3 snímky", "5 snímok".
fn slide_count_label(count: usize) -> String {
    let word = match count {
        1 => "snímka",
        2..=4 => "snímky",
        _ => "snímok",
    };
    format!("{count} {word}")
}

/// The archive under the active list, collapsed until opened.
#[component]
pub(crate) fn PresentationArchive() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let archived = bs.archived_presentations;
    view! {
        <details class="bible__prepared-archive" data-role="presentations-archive">
            <summary data-role="presentations-archive-toggle">
                {move || format!("Archív ({})", archived.with(Vec::len))}
            </summary>
            <div class="bible__prepared-archive-list" data-role="presentations-archive-list">
                {move || {
                    let list = archived.get();
                    if list.is_empty() {
                        view! { <p class="operator__slides-empty">"Archív je prázdny."</p> }
                            .into_any()
                    } else {
                        list.into_iter()
                            .map(|p| view! { <ArchivedCard presentation=p /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </details>
    }
}

#[component]
fn ArchivedCard(presentation: BiblePresentationSummary) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let id = presentation.id.clone();
    let name = presentation.name.clone();
    let badge = sermon_badge(&presentation);

    let on_restore = {
        let id = id.clone();
        let name = name.clone();
        let ctx = ctx.clone();
        move |_: MouseEvent| {
            let (id, name, ctx) = (id.clone(), name.clone(), ctx.clone());
            leptos::task::spawn_local(async move {
                match bible::restore_presentation(&id).await {
                    Ok(()) => toast(&ctx, "success", format!("Obnovené: {name}")),
                    Err(e) => toast(&ctx, "error", format!("Obnovenie zlyhalo: {e}")),
                }
                reload_presentation_lists(bs).await;
            });
        }
    };
    let on_delete = {
        let id = id.clone();
        let name = name.clone();
        move |_: MouseEvent| {
            let question = format!("Natrvalo zmazať „{name}“? Toto sa nedá vrátiť.");
            let confirmed = crate::utils::window::window()
                .confirm_with_message(&question)
                .unwrap_or(false);
            if !confirmed {
                return;
            }
            let (id, name, ctx) = (id.clone(), name.clone(), ctx.clone());
            leptos::task::spawn_local(async move {
                match bible::delete_presentation(&id).await {
                    Ok(()) => {
                        if bs.active_presentation_id.get_untracked().as_ref() == Some(&id) {
                            bs.active_presentation_id.set(None);
                            bs.active_presentation_slides.set(Vec::new());
                        }
                        toast(&ctx, "success", format!("Zmazané: {name}"));
                    }
                    Err(e) => toast(&ctx, "error", format!("Mazanie zlyhalo: {e}")),
                }
                reload_presentation_lists(bs).await;
            });
        }
    };

    view! {
        <div class="bible__archived-card" data-role="archived-presentation" data-presentation-id=id>
            <div class="bible__presentation-head">
                <strong class="bible__presentation-name">{name}</strong>
            </div>
            {badge.map(|badge| view! { <p data-role="presentation-sermon-badge">{badge}</p> })}
            <div class="bible__archived-actions">
                <button
                    type="button"
                    class="bible__modal-btn"
                    data-role="archived-restore"
                    on:click=on_restore
                >"Obnoviť"</button>
                <button
                    type="button"
                    class="bible__modal-btn bible__modal-btn--danger"
                    data-role="archived-delete"
                    on:click=on_delete
                >"Zmazať natrvalo"</button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::slide_count_label;

    #[test]
    fn the_slide_count_uses_the_slovak_plural() {
        assert_eq!(slide_count_label(0), "0 snímok");
        assert_eq!(slide_count_label(1), "1 snímka");
        assert_eq!(slide_count_label(3), "3 snímky");
        assert_eq!(slide_count_label(5), "5 snímok");
    }
}
