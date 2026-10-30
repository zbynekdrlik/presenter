//! The phone shell (#850 spec §6.1): ONE media query decides whether the
//! operator renders as a phone (bottom tab bar, compact header) or a desktop.
//! `install_phone_shell` mirrors it into `AppContext::phone_shell` and
//! `body[data-shell]` and keeps both in step; CSS keys on `[data-shell]` only.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// Portrait phones by width; landscape phones (844×390) by a coarse pointer
/// and a short viewport — they used to fall into the desktop layout.
pub const PHONE_SHELL_QUERY: &str = "(max-width: 768px), (pointer: coarse) and (max-height: 500px)";

/// Evaluate the query now and on every change (rotation, a resized window).
/// Safari before 14 has no `MediaQueryList.addEventListener`; the old
/// `addListener` covers it (the app still supports iOS 12).
pub fn install_phone_shell(phone_shell: RwSignal<bool>) {
    let list = crate::utils::window::window()
        .match_media(PHONE_SHELL_QUERY)
        .ok()
        .flatten();
    let Some(list) = list else {
        apply(phone_shell, false);
        return;
    };
    apply(phone_shell, list.matches());
    let watched = list.clone();
    let on_change = Closure::<dyn Fn()>::new(move || apply(phone_shell, watched.matches()));
    let callback = on_change.as_ref().unchecked_ref();
    if list
        .add_event_listener_with_callback("change", callback)
        .is_err()
    {
        let _ = list.add_listener_with_opt_callback(Some(callback));
    }
    on_change.forget();
}

fn apply(phone_shell: RwSignal<bool>, phone: bool) {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("data-shell", if phone { "phone" } else { "desktop" });
    }
    if phone_shell.get_untracked() != phone {
        phone_shell.set(phone);
    }
}
