use crate::state::AppContext;
use gloo_timers::callback::Timeout;
use leptos::prelude::*;

/// How long a toast stays on screen.
const TOAST_MS: u32 = 3_500;

/// Hides a `visible` + `message` toast `hide_ms` after its LAST message: every
/// new message restarts the one timer (#875). Call once, in the component that
/// owns the signals.
pub fn hide_after_last_message(message: RwSignal<String>, visible: RwSignal<bool>, hide_ms: u32) {
    Effect::new(move |previous: Option<Option<Timeout>>| {
        drop(previous);
        message.track();
        visible
            .get()
            .then(|| Timeout::new(hide_ms, move || visible.set(false)))
    });
}

/// Toast notification component. It owns the toast's lifetime, whoever set the
/// message (#875): every new message restarts the one timer (dropping a
/// `Timeout` cancels it).
#[component]
pub fn Toast() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let toast_message = ctx.toast_message;

    Effect::new(move |previous: Option<Option<Timeout>>| {
        drop(previous);
        toast_message
            .with(Option::is_some)
            .then(|| Timeout::new(TOAST_MS, move || toast_message.set(None)))
    });

    view! {
        <div
            data-role="toast"
            data-visible=move || if ctx.toast_message.get().is_some() { "true" } else { "false" }
            data-variant=move || ctx.toast_variant.get()
            class="operator__toast"
            class:operator__toast--visible=move || ctx.toast_message.get().is_some()
        >
            {move || ctx.toast_message.get().unwrap_or_default()}
        </div>
    }
}
