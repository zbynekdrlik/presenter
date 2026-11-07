//! The GUI's press counters (#913): every click on a `[data-role]` control is
//! counted and sent in batches to `POST /ui/usage`, so what gets a place on a
//! small screen is decided from use (`.claude/rules/ui-usage.md`). Only the
//! control's role, the page and the shell leave the browser, never text. A lost
//! batch costs a few counts and nothing else, so nothing here can fail the page.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gloo_timers::callback::Timeout;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const URL: &str = "/ui/usage";
const SEND_AFTER_MS: u32 = 10_000;
/// The server's bound on a row's name.
const MAX_NAME: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Control {
    page: String,
    shell: String,
    role: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Row<'a> {
    page: &'a str,
    shell: &'a str,
    role: &'a str,
    count: u32,
}

#[derive(Default)]
struct Pending {
    counts: BTreeMap<Control, u32>,
    timer: Option<Timeout>,
    /// The leave listeners are on: they come with the first press, so a screen
    /// nobody presses (a stage display) holds none.
    leave_listening: bool,
}

thread_local! {
    static PENDING: RefCell<Pending> = RefCell::default();
}

fn batch_json(counts: &BTreeMap<Control, u32>) -> Option<String> {
    let rows: Vec<Row<'_>> = counts
        .iter()
        .map(|(control, count)| Row {
            page: &control.page,
            shell: &control.shell,
            role: &control.role,
            count: *count,
        })
        .collect();
    serde_json::to_string(&rows).ok()
}

fn send_now() {
    let counts = PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending.timer = None;
        std::mem::take(&mut pending.counts)
    });
    if counts.is_empty() {
        return;
    }
    if let (Some(window), Some(batch)) = (web_sys::window(), batch_json(&counts)) {
        // `false` = the browser's queue is full: those counts are simply lost.
        let _ = window
            .navigator()
            .send_beacon_with_opt_str(URL, Some(&batch));
    }
}

fn count(control: Control) {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        *pending.counts.entry(control).or_default() += 1;
        if !std::mem::replace(&mut pending.leave_listening, true) {
            install_leave_listeners();
        }
        if pending.timer.is_none() {
            pending.timer = Some(Timeout::new(SEND_AFTER_MS, send_now));
        }
    });
}

fn pressed_control(ev: &web_sys::Event) -> Option<Control> {
    let target = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
    let role = target
        .closest("[data-role]")
        .ok()??
        .get_attribute("data-role")?;
    let document = target.owner_document()?;
    let page = document.location()?.pathname().ok()?;
    let shell = document.body()?.get_attribute("data-shell");
    (role.len() <= MAX_NAME).then_some(Control {
        page,
        shell: shell_name(shell.as_deref()).to_string(),
        role,
    })
}

/// Pages without the phone shell are desktop pages.
fn shell_name(shell: Option<&str>) -> &'static str {
    match shell {
        Some("phone") => "phone",
        _ => "desktop",
    }
}

/// Once, at the app's start. Before the page's own handlers (capture), so a
/// control that removes itself is still counted.
pub fn install() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(|ev: web_sys::Event| {
        if let Some(control) = pressed_control(&ev) {
            count(control);
        }
    });
    let _ = document.add_event_listener_with_callback_and_bool(
        "click",
        on_click.as_ref().unchecked_ref(),
        true,
    );
    on_click.forget();
}

/// A phone leaves without closing anything: a hidden page may never run again.
fn install_leave_listeners() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let on_leave = Closure::<dyn FnMut()>::new(|| {
        let hidden = web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| document.hidden());
        if hidden {
            send_now();
        }
    });
    let _ = document
        .add_event_listener_with_callback("visibilitychange", on_leave.as_ref().unchecked_ref());
    on_leave.forget();
    let on_pagehide = Closure::<dyn FnMut()>::new(send_now);
    let _ =
        window.add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref());
    on_pagehide.forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_names_the_control_and_its_count_only() {
        let mut counts = BTreeMap::new();
        counts.insert(
            Control {
                page: "/ui/study".to_string(),
                shell: "phone".to_string(),
                role: "study-fav".to_string(),
            },
            3,
        );
        assert_eq!(
            batch_json(&counts).as_deref(),
            Some(r#"[{"page":"/ui/study","shell":"phone","role":"study-fav","count":3}]"#)
        );
    }

    #[test]
    fn only_the_phone_shell_is_a_phone() {
        assert_eq!(shell_name(Some("phone")), "phone");
        assert_eq!(shell_name(Some("desktop")), "desktop");
        assert_eq!(shell_name(None), "desktop");
    }
}
