//! Slide multi-select clipboard UI (#554): the selection panel, per-card
//! checkbox + selected/cut markers, the "paste here" insertion bars, the
//! window keyboard shortcuts, and the copy/cut/paste/clear actions. The pure
//! order/shortcut logic lives in `slide_selection_logic.rs`; this file is the
//! DOM/Leptos wiring. Reused from `slide_list.rs`.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::api;
use crate::api::ApiError;
use crate::state::operator::{ClipboardMode, OperatorState, SlideClipboard};
use crate::state::AppContext;

use super::slide_save::reconcile_after_seq_mismatch;
use super::slide_selection_logic::{
    cut_splice_order, range_select_by_anchor_id, shortcut_action, ShortcutAction,
};

/// Ordered slide ids of the currently open presentation (untracked snapshot).
fn current_slide_ids(ctx: &AppContext) -> Vec<String> {
    ctx.selected_presentation
        .get_untracked()
        .map(|p| p.slides.iter().map(|s| s.id.to_string()).collect())
        .unwrap_or_default()
}

/// The open presentation's id, or `None` when nothing is loaded.
fn current_pres_id(ctx: &AppContext) -> Option<String> {
    ctx.selected_presentation
        .get_untracked()
        .map(|p| p.id.to_string())
}

/// True while a TEXT-ENTRY field — a textarea, a `<select>`, a
/// contenteditable element, or a non-checkbox input (#558 V10) — holds
/// focus. The guard that keeps Ctrl+C/X/V from firing while the user is
/// typing/selecting slide content, or interacting with an unrelated form
/// control (e.g. the stage-layout `<select>` dropdown). A focused selection
/// CHECKBOX must NOT suppress the shortcut (you select with the checkbox,
/// then Ctrl+C), so it is the ONE input type excluded here. This is the
/// "never fire outside the slide grid" intent of the spec; `is_interactive_tag`
/// stays the guard for the per-card CLICK, where its breadth (incl. checkbox)
/// is correct.
fn text_entry_focused() -> bool {
    let doc = crate::utils::window::document();
    let Some(active) = doc.active_element() else {
        return false;
    };
    let tag = active.tag_name().to_lowercase();
    if tag == "textarea" || tag == "select" {
        return true;
    }
    if tag == "input" {
        let is_slide_select_checkbox = active.get_attribute("type").as_deref() == Some("checkbox")
            && active.get_attribute("data-role").as_deref() == Some("slide-select-checkbox");
        return !is_slide_select_checkbox;
    }
    active
        .dyn_ref::<web_sys::HtmlElement>()
        .map(|el| el.is_content_editable())
        .unwrap_or(false)
}

/// Per-card `Memo`: is this slide currently selected? Copy + `PartialEq`, so
/// the checkbox `checked` and the `--selected` class update WITHOUT a list
/// rebuild.
pub(super) fn selected_memo(op: &OperatorState, slide_id: String) -> Memo<bool> {
    let selected = op.selected_slide_ids;
    Memo::new(move |_| selected.with(|s| s.contains(&slide_id)))
}

/// Per-card `Memo`: is this slide part of a `Cut` clipboard (→ dim + badge)?
pub(super) fn cut_memo(op: &OperatorState, slide_id: String) -> Memo<bool> {
    let clipboard = op.clipboard;
    Memo::new(move |_| {
        clipboard.with(|c| {
            c.as_ref()
                .map(|cb| cb.mode == ClipboardMode::Cut && cb.slide_ids.contains(&slide_id))
                .unwrap_or(false)
        })
    })
}

/// The per-card select checkbox (edit mode only). Controlled: `prop:checked`
/// reads `selected_memo`; the click `prevent_default`s the native toggle and
/// drives the selection set. Plain click toggles + sets the anchor; Shift+click
/// selects the inclusive range from the anchor.
pub(super) fn render_select_checkbox(
    ctx: AppContext,
    op: OperatorState,
    slide_id: String,
    index: usize,
) -> impl IntoView {
    let checked = selected_memo(&op, slide_id.clone());
    view! {
        <input
            type="checkbox"
            class="operator__slide-select"
            data-role="slide-select-checkbox"
            data-slide-select-index=index
            prop:checked=move || checked.get()
            on:click=move |ev: web_sys::MouseEvent| {
                ev.prevent_default();
                let ids = current_slide_ids(&ctx);
                if ev.shift_key() {
                    if let Some(anchor_id) = op.selection_anchor_id.get_untracked() {
                        // #558 V9: resolve the anchor's CURRENT index by id —
                        // a vanished anchor (e.g. deleted since) falls through
                        // to a plain click below, never a guessed range.
                        if let Some(next) = range_select_by_anchor_id(
                            &ids,
                            &anchor_id,
                            index,
                            &op.selected_slide_ids.get_untracked(),
                        ) {
                            op.selected_slide_ids.set(next);
                            return;
                        }
                    }
                }
                // Plain click (or Shift with no anchor yet, or a vanished
                // anchor): toggle this id.
                let sid = slide_id.clone();
                op.selected_slide_ids.update(|set| {
                    if !set.remove(&sid) {
                        set.insert(sid);
                    }
                });
                op.selection_anchor_id.set(Some(slide_id.clone()));
            }
        />
    }
}

/// A "paste here" insertion bar for `gap` (0..=len). Rendered by
/// `slide_list.rs` only when the clipboard is non-empty. Click OR block-drop →
/// paste at `gap`; hover → remember `gap` for Ctrl/Cmd+V.
pub(super) fn render_insertion_bar(
    ctx: AppContext,
    op: OperatorState,
    gap: usize,
) -> impl IntoView {
    let ctx_drop = ctx.clone();
    let op_drop = op.clone();
    let op_click = op.clone();
    let paste_target_gap = op.paste_target_gap;
    let dragging_clipboard = op.dragging_clipboard;
    view! {
        <div
            class="operator__slide-insert-bar"
            data-role="slide-insert-bar"
            data-insert-index=gap
            on:mouseenter=move |_| paste_target_gap.set(Some(gap))
            on:dragover=move |ev: web_sys::DragEvent| {
                if dragging_clipboard.get_untracked() {
                    ev.prevent_default();
                }
            }
            on:drop=move |ev: web_sys::DragEvent| {
                if op_drop.dragging_clipboard.get_untracked() {
                    ev.prevent_default();
                    paste_at_gap(&ctx_drop, &op_drop, gap);
                }
            }
            on:click=move |_| paste_at_gap(&ctx, &op_click, gap)
        >
            <span class="operator__slide-insert-hint">"Paste here"</span>
        </div>
    }
}

/// Copy the current selection into the clipboard (`Copy` mode). No-op if empty.
pub(super) fn copy_selection(ctx: &AppContext, op: &OperatorState) {
    set_clipboard(ctx, op, ClipboardMode::Copy);
}

/// Cut the current selection into the clipboard (`Cut` mode). Non-destructive:
/// the originals stay until a paste succeeds. No-op if empty.
pub(super) fn cut_selection(ctx: &AppContext, op: &OperatorState) {
    set_clipboard(ctx, op, ClipboardMode::Cut);
}

fn set_clipboard(ctx: &AppContext, op: &OperatorState, mode: ClipboardMode) {
    let Some(pres_id) = current_pres_id(ctx) else {
        return;
    };
    let selected = op.selected_slide_ids.get_untracked();
    if selected.is_empty() {
        return;
    }
    // Capture in list order.
    let slide_ids: Vec<String> = current_slide_ids(ctx)
        .into_iter()
        .filter(|id| selected.contains(id))
        .collect();
    if slide_ids.is_empty() {
        return;
    }
    op.clipboard.set(Some(SlideClipboard {
        slide_ids,
        mode,
        presentation_id: pres_id,
    }));
}

/// Clear the clipboard, selection, anchor, and paste target (Escape / Clear /
/// presentation switch). Nothing is deleted — a cut is simply abandoned.
pub(super) fn clear_selection_and_clipboard(op: &OperatorState) {
    op.clipboard.set(None);
    op.selected_slide_ids.set(std::collections::HashSet::new());
    op.selection_anchor_id.set(None);
    op.paste_target_gap.set(None);
}

/// The slide id CURRENTLY at index `gap` — the anchor a paste inserted at
/// `gap` must land BEFORE; `None` when `gap` is at (or past) the end
/// (#558 V8). Resolved fresh from the live DOM-backed list, never a value
/// captured earlier.
fn anchor_slide_id_for_gap(ctx: &AppContext, gap: usize) -> Option<String> {
    current_slide_ids(ctx).get(gap).cloned()
}

/// Paste the clipboard at `gap` (0..=len). Copy → the paste endpoint (by
/// ANCHOR slide id, #558 V8); Cut → the existing reorder endpoint via
/// `cut_splice_order` (already carries the full target order, so it has no
/// raw-index race to fix).
pub(super) fn paste_at_gap(ctx: &AppContext, op: &OperatorState, gap: usize) {
    let Some(clipboard) = op.clipboard.get_untracked() else {
        return;
    };
    let Some(pres_id) = current_pres_id(ctx) else {
        return;
    };
    // A clipboard captured in a DIFFERENT presentation is stale by definition
    // (scope: one presentation for now).
    if clipboard.presentation_id != pres_id {
        clear_selection_and_clipboard(op);
        return;
    }
    match clipboard.mode {
        ClipboardMode::Copy => {
            let anchor_slide_id = anchor_slide_id_for_gap(ctx, gap);
            paste_copy(ctx, op, pres_id, clipboard.slide_ids, anchor_slide_id)
        }
        ClipboardMode::Cut => paste_cut(ctx, op, pres_id, clipboard.slide_ids, gap),
    }
}

/// Apply a server slide list to `selected_presentation` under the
/// `slide_edit_seq` staleness guard (#552/#556) — the same guard the
/// reorder/insert/duplicate paths use. `my_seq` is captured by the caller
/// BEFORE the async request.
///
/// #558 V4: the seq guard alone does not cover a PRESENTATION SWITCH — if
/// the operator switches to a DIFFERENT presentation while this request is
/// still in flight, `slide_edit_seq` may be unchanged (nothing else bumped
/// it in the meantime), so the stale seq check alone would apply THIS
/// response onto the NOW-selected (unrelated) presentation. `pres_id` is
/// captured by the caller at request time; apply only if the currently
/// selected presentation is still that same one.
async fn apply_slides_guarded(
    ctx: &AppContext,
    op: &OperatorState,
    pres_id: String,
    slides: Vec<presenter_core::Slide>,
    my_seq: u64,
) {
    let same_presentation = ctx
        .selected_presentation
        .get_untracked()
        .map(|pres| pres.id.to_string() == pres_id)
        .unwrap_or(false);
    if !same_presentation {
        return;
    }
    if op.slide_edit_seq.get_untracked() == my_seq {
        ctx.selected_presentation.update(|p| {
            if let Some(pres) = p.as_mut() {
                pres.slides = slides;
            }
        });
    } else {
        reconcile_after_seq_mismatch(
            pres_id,
            slides,
            ctx.selected_presentation,
            op.slide_edit_seq,
            op.save_status,
        )
        .await;
    }
}

fn paste_copy(
    ctx: &AppContext,
    op: &OperatorState,
    pres_id: String,
    ids: Vec<String>,
    anchor_slide_id: Option<String>,
) {
    op.slide_edit_seq.update(|s| *s += 1);
    let my_seq = op.slide_edit_seq.get_untracked();
    let ctx = ctx.clone();
    let op = op.clone();
    leptos::task::spawn_local(async move {
        match api::presentations::paste_slides(&pres_id, ids, anchor_slide_id).await {
            Ok(slides) => {
                apply_slides_guarded(&ctx, &op, pres_id, slides, my_seq).await;
                // Copy clipboard is KEPT so the user can paste again.
            }
            Err(ApiError::Status(422, _)) => {
                op.clipboard.set(None);
                ctx.show_toast("Those slides no longer exist", "error");
            }
            Err(ApiError::Status(409, _)) => {
                // #558 V8: the anchor slide vanished (a concurrent structural
                // edit removed it) — refresh from the server instead of
                // guessing a new position. Clipboard is KEPT so the user can
                // retry the paste against the fresh list.
                //
                // #558 W1: this used to apply the refetched detail
                // UNCONDITIONALLY — bypassing the V4 presentation-switch
                // guard `apply_slides_guarded` enforces on the success path.
                // If the operator switched away from `pres_id` while this
                // request was still in flight, painting a refetch of the
                // OLD (now unrelated) presentation onto whatever is now
                // selected would be exactly the V4 bug this guards against.
                // Recover ONLY if `pres_id` is STILL the selected
                // presentation; otherwise there is nothing to recover onto
                // — just drop the stale clipboard silently (the operator
                // already left the song).
                let still_here = ctx
                    .selected_presentation
                    .get_untracked()
                    .map(|pres| pres.id.to_string() == pres_id)
                    .unwrap_or(false);
                if still_here {
                    if let Ok(detail) = api::presentations::get_presentation(&pres_id).await {
                        ctx.selected_presentation.set(Some(detail.presentation));
                    }
                    ctx.show_toast("Paste position changed — refreshed, try again", "error");
                } else {
                    op.clipboard.set(None);
                }
            }
            Err(_) => {
                // Clipboard KEPT so the user can retry.
                ctx.show_toast("Paste failed — try again", "error");
            }
        }
    });
}

fn paste_cut(ctx: &AppContext, op: &OperatorState, pres_id: String, ids: Vec<String>, gap: usize) {
    let current = current_slide_ids(ctx);
    let Some(final_order) = cut_splice_order(&current, &ids, gap) else {
        // A stale cut (an id vanished): drop the clipboard + marks.
        clear_selection_and_clipboard(op);
        ctx.show_toast("Those slides no longer exist", "error");
        return;
    };
    op.slide_edit_seq.update(|s| *s += 1);
    let my_seq = op.slide_edit_seq.get_untracked();
    let ctx = ctx.clone();
    let op = op.clone();
    leptos::task::spawn_local(async move {
        match api::presentations::reorder_slides(&pres_id, final_order).await {
            Ok(slides) => {
                apply_slides_guarded(&ctx, &op, pres_id, slides, my_seq).await;
                // The cut is consumed — clear clipboard + selection.
                clear_selection_and_clipboard(&op);
            }
            Err(_) => {
                // Originals stay in place, still marked; retry or Escape.
                ctx.show_toast("Move failed — try again", "error");
            }
        }
    });
}

/// Clear selection + clipboard whenever the open presentation changes (spec
/// scope: one presentation). Called once from `SlideList`.
pub(super) fn setup_selection_clear_on_switch(ctx: AppContext, op: OperatorState) {
    let selected_presentation_id = ctx.selected_presentation_id;
    Effect::new(move |prev: Option<Option<String>>| {
        let current = selected_presentation_id.get();
        if let Some(prev) = prev {
            if prev != current {
                clear_selection_and_clipboard(&op);
            }
        }
        current
    });
}

/// Install the ONE window keydown listener for the clipboard shortcuts
/// (mirrors the operator page's existing global handler — the slide grid never
/// holds DOM focus, so a container-scoped listener could never fire). Acts only
/// in edit mode with a presentation open, and never while a text field is
/// focused. Called once from `SlideList`.
pub(super) fn setup_clipboard_keyboard(ctx: AppContext, op: OperatorState) {
    let handler =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            // Only in edit mode, with a presentation open, AND while the
            // worship editor is the ACTIVE view (#558 V5). The operator
            // page mounts every view's panel simultaneously (CSS toggles
            // visibility — see `pages/operator.rs`), so this WINDOW
            // listener stays live even while the Bible/timers/settings
            // panel is showing; without the view check, Ctrl+C on one of
            // those hidden panels hijacked native copy and silently
            // mutated the (hidden) worship song's clipboard.
            if ctx.mode.get_untracked() == "live"
                || ctx.view.get_untracked() != "worship"
                || ctx.selected_presentation.get_untracked().is_none()
            {
                return;
            }
            let ctrl_or_meta = ev.ctrl_key() || ev.meta_key();
            let Some(action) = shortcut_action(&ev.key(), ctrl_or_meta) else {
                return;
            };
            // Never fire copy/cut/paste from inside a text field. Escape is
            // allowed to clear even from a field.
            if !matches!(action, ShortcutAction::Clear) && text_entry_focused() {
                return;
            }
            match action {
                ShortcutAction::Copy => {
                    if !op.selected_slide_ids.get_untracked().is_empty() {
                        ev.prevent_default();
                        copy_selection(&ctx, &op);
                    }
                }
                ShortcutAction::Cut => {
                    if !op.selected_slide_ids.get_untracked().is_empty() {
                        ev.prevent_default();
                        cut_selection(&ctx, &op);
                    }
                }
                ShortcutAction::Paste => {
                    if op.clipboard.get_untracked().is_some() {
                        ev.prevent_default();
                        let len = current_slide_ids(&ctx).len();
                        let gap = op.paste_target_gap.get_untracked().unwrap_or(len).min(len);
                        paste_at_gap(&ctx, &op, gap);
                    }
                }
                ShortcutAction::Clear => {
                    if op.clipboard.get_untracked().is_some()
                        || !op.selected_slide_ids.get_untracked().is_empty()
                    {
                        clear_selection_and_clipboard(&op);
                    }
                }
            }
        });
    let window = crate::utils::window::window();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    // Forget: the operator page has no client-side router, so this window
    // listener lives for the page's lifetime (same pattern as the settings
    // pollers — gloo Interval/Closure cannot ride `on_cleanup`, see
    // `.claude/skills/ui/SKILL.md`).
    handler.forget();
}

/// The sticky selection panel above the slide list (edit mode). Shows the count
/// and Copy / Cut / Paste / Clear, plus a draggable block handle when the
/// clipboard is non-empty. Visible only when something is selected OR the
/// clipboard is non-empty.
#[component]
pub fn SlideSelectionPanel() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let mode = ctx.mode;
    let selected_slide_ids = op.selected_slide_ids;
    let clipboard = op.clipboard;
    let visible = move || {
        mode.get() != "live"
            && (!selected_slide_ids.with(|s| s.is_empty()) || clipboard.get().is_some())
    };

    view! {
        <Show when=visible fallback=|| ()>
            {
                let ctx = ctx.clone();
                let op = op.clone();
                move || {
                    let ctx_copy = ctx.clone();
                    let ctx_cut = ctx.clone();
                    let ctx_paste = ctx.clone();
                    let op_copy = op.clone();
                    let op_cut = op.clone();
                    let op_paste = op.clone();
                    let op_clear = op.clone();
                    let op_drag = op.clone();
                    view! {
                        <div class="operator__slide-selection" data-role="slide-selection-panel">
                            <span
                                class="operator__slide-selection-count"
                                data-role="slide-selection-count"
                            >
                                {move || format!("{} selected", selected_slide_ids.with(|s| s.len()))}
                            </span>
                            <button
                                type="button"
                                class="operator__list-action"
                                data-action="copy"
                                data-role="slide-copy"
                                on:click=move |_| copy_selection(&ctx_copy, &op_copy)
                            >"Copy"</button>
                            <button
                                type="button"
                                class="operator__list-action"
                                data-action="cut"
                                data-role="slide-cut"
                                on:click=move |_| cut_selection(&ctx_cut, &op_cut)
                            >"Cut"</button>
                            <button
                                type="button"
                                class="operator__list-action"
                                data-action="paste"
                                data-role="slide-paste"
                                prop:disabled=move || clipboard.get().is_none()
                                on:click=move |_| {
                                    let len = current_slide_ids(&ctx_paste).len();
                                    let gap = op_paste
                                        .paste_target_gap
                                        .get_untracked()
                                        .unwrap_or(len)
                                        .min(len);
                                    paste_at_gap(&ctx_paste, &op_paste, gap);
                                }
                            >"Paste"</button>
                            <button
                                type="button"
                                class="operator__list-action"
                                data-action="clear"
                                data-role="slide-clear"
                                on:click=move |_| clear_selection_and_clipboard(&op_clear)
                            >"Clear"</button>
                            <Show when=move || clipboard.get().is_some() fallback=|| ()>
                                {
                                    let dragging_clipboard = op_drag.dragging_clipboard;
                                    view! {
                                        <span
                                            class="operator__slide-selection-drag"
                                            data-role="clipboard-drag"
                                            draggable="true"
                                            title="Drag the block onto a gap"
                                            on:dragstart=move |ev: web_sys::DragEvent| {
                                                if let Some(dt) = ev.data_transfer() {
                                                    let _ = dt.set_data("application/x-slide-clipboard", "1");
                                                    dt.set_effect_allowed("move");
                                                }
                                                dragging_clipboard.set(true);
                                            }
                                            on:dragend=move |_| dragging_clipboard.set(false)
                                        >"\u{2195} Drag block"</span>
                                    }
                                }
                            </Show>
                        </div>
                    }
                }
            }
        </Show>
    }
}
