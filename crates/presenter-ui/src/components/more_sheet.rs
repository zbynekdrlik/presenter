//! The phone header's "Viac" sheet (#850 spec §6.3): the status chips (their
//! own top group, #573), the other surfaces, Stage Output (with a confirm) and
//! the text mode. It closes on ✕, a tap on the backdrop, Escape and the
//! browser's Back: opening pushes a history entry on the same URL, and every
//! close goes through Back, so the entry never lingers.
//!
//! Mounted once in the phone shell and only shown or hidden: the chips keep
//! their state and their polls between opens, and exactly one copy polls (the
//! header mounts its copies only on a desktop). The stage controls mount per
//! open, so a closed sheet never keeps a half-answered stage confirm.

use leptos::prelude::*;

use crate::components::ai_status::AiStatusChip;
use crate::components::resolume_status::ResolumeStatusChips;
use crate::components::stage_layout_select::StageLayoutSelect;
use crate::components::stage_text_mode_picker::StageTextModePicker;
use crate::components::surface_nav::SurfaceNav;
use crate::components::version_label::VersionLabel;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Open the sheet with its own history entry (Back closes it).
pub fn open_more_sheet(op: &OperatorState) {
    if op.more_sheet_open.get_untracked() {
        return;
    }
    op.more_sheet_open.set(true);
    let state = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&state, &"sheet".into(), &true.into());
    let _ = crate::utils::window::window()
        .history()
        .and_then(|history| history.push_state(&state, ""));
}

/// How long a closed sheet keeps catching taps: a second tap of a double tap
/// on ✕ or the backdrop lands within this.
const SHIELD_MS: u32 = 350;

/// Close it and drop its history entry. Closed at once, not on the popstate: a
/// double tap on ✕ must not go back twice and leave the page. The shield takes
/// that second tap, which would otherwise send the slide underneath to the wall.
pub fn close_more_sheet(op: &OperatorState) {
    if op.more_sheet_open.get_untracked() {
        op.more_sheet_open.set(false);
        let shield = op.more_sheet_shield;
        shield.set(true);
        gloo_timers::callback::Timeout::new(SHIELD_MS, move || shield.set(false)).forget();
        let _ = crate::utils::window::window()
            .history()
            .and_then(|history| history.back());
    }
}

/// A reload with the sheet open comes back closed but on its history entry;
/// drop it, or the next Back seems to do nothing.
fn drop_reloaded_entry() {
    let window = crate::utils::window::window();
    let Ok(history) = window.history() else {
        return;
    };
    let on_entry = history
        .state()
        .ok()
        .and_then(|state| js_sys::Reflect::get(&state, &"sheet".into()).ok())
        .is_some_and(|sheet| sheet.is_truthy());
    if on_entry {
        let _ = history.back();
    }
}

#[component]
pub fn MoreSheet() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);
    let open = op.more_sheet_open;
    let shield = op.more_sheet_shield;
    drop_reloaded_entry();
    let phone = ctx.phone_shell;
    let close = {
        let op = op.clone();
        move |_| close_more_sheet(&op)
    };

    // A window or tablet leaving the phone width takes the sheet with it.
    {
        let op = op.clone();
        Effect::new(move |_| {
            if !phone.get() {
                close_more_sheet(&op);
            }
        });
    }

    view! {
        <div class="more-sheet-shield" data-role="more-sheet-shield" hidden=move || !shield.get()></div>
        <Show when=move || phone.get()>
            <div
                class="more-sheet"
                data-role="more-sheet"
                data-open=move || if open.get() { "true" } else { "false" }
                aria-hidden=move || if open.get() { "false" } else { "true" }
            >
                <div class="more-sheet__backdrop" data-role="more-sheet-backdrop" on:click=close.clone()></div>
                <section class="more-sheet__panel" role="dialog" aria-label="Viac">
                    <header class="more-sheet__head">
                        <h2>"Viac"</h2>
                        <button
                            type="button"
                            class="more-sheet__close"
                            data-role="more-sheet-close"
                            aria-label="Zavrieť"
                            on:click=close.clone()
                        >
                            "\u{2715}"
                        </button>
                    </header>
                    <div class="more-sheet__group" data-role="more-sheet-status">
                        <ResolumeStatusChips />
                        <AiStatusChip />
                        <span class="more-sheet__version"><VersionLabel /></span>
                    </div>
                    <div class="more-sheet__group">
                        <SurfaceNav />
                    </div>
                    <Show when=move || open.get()>
                        <div class="more-sheet__group">
                            <StageLayoutSelect confirm=true />
                            <StageTextModePicker />
                        </div>
                    </Show>
                </section>
            </div>
        </Show>
    }
}
