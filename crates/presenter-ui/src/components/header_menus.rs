//! The desktop header's three short menus (#875 H1, mockup v3): Stránky (the
//! sections that left the tab row, plus the study page), Obrazovky (the other
//! surfaces, new tab) and Pripojenia (the Resolume + AI chips). One menu is
//! open at a time; Escape, a press outside and picking an item close it. The
//! panels overlay the page, they never push it.
//!
//! Mounted only on a desktop (`header.rs`): the phone shell keeps its tab bar
//! and "Viac" sheet, and the chips must exist once, or their polls double
//! (`.claude/rules/phone-shell.md`). Panels are hidden, never unmounted, so the
//! chips keep polling and the Pripojenia button reads their state while closed.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::ai_status::AiStatusChip;
use crate::components::resolume_status::{HostLink, ResolumeStatusChips};
use crate::components::surface_nav::SurfaceNav;
use crate::state::view_nav::{set_view, VIEWS};
use crate::state::AppContext;
use crate::utils::dom_attr::bool_attr;

/// Sections with their own button in the header row.
pub const DIRECT_VIEWS: [&str; 2] = ["worship", "bible"];
/// The section behind the ⚙ button.
pub const SETTINGS_VIEW: &str = "settings";

/// The sections the Stránky menu lists: every view without its own button.
pub fn menu_views() -> impl Iterator<Item = (&'static str, &'static str)> {
    VIEWS
        .into_iter()
        .filter(|(id, _)| !DIRECT_VIEWS.contains(id) && *id != SETTINGS_VIEW)
}

/// What the Pripojenia button says without being opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Health {
    /// Every connection answered.
    Ok,
    /// Nothing is down, but something has not answered yet.
    Checking,
    /// These connections are down (Resolume host labels, "AI").
    Down(Vec<String>),
}

/// A Resolume host is down once the server reports `error` (the chip's yellow
/// and red); AI once its chip says `unavailable`. A failed last call or a
/// first poll still on its way is not an outage.
pub(crate) fn summarize(hosts: &[HostLink], ai_state: &str) -> Health {
    let mut down: Vec<String> = hosts
        .iter()
        .filter(|h| h.state == "error")
        .map(|h| h.label.clone())
        .collect();
    if ai_state == "unavailable" {
        down.push("AI".to_string());
    }
    if !down.is_empty() {
        Health::Down(down)
    } else if ai_state == "ok" && hosts.iter().all(|h| h.state == "connected") {
        Health::Ok
    } else {
        Health::Checking
    }
}

pub(crate) fn health_label(health: &Health) -> String {
    match health {
        Health::Down(names) => format!("Odpojené: {}", names.join(", ")),
        _ => "Pripojenia".to_string(),
    }
}

pub(crate) fn health_state(health: &Health) -> &'static str {
    match health {
        Health::Ok => "ok",
        Health::Checking => "checking",
        Health::Down(_) => "down",
    }
}

/// Close the open menu on Escape (focus back on its button) and on a press
/// outside every menu. Removed when the menus unmount (a window crossing to
/// the phone shell).
fn close_on_escape_and_outside(open: RwSignal<Option<&'static str>>) {
    let escape = window_event_listener_untyped("keydown", move |ev| {
        let Some(key) = ev.dyn_ref::<web_sys::KeyboardEvent>().map(|k| k.key()) else {
            return;
        };
        if key != "Escape" {
            return;
        }
        let Some(id) = open.try_get_untracked().flatten() else {
            return;
        };
        open.set(None);
        let selector = format!("[data-role=\"header-menu-toggle\"][data-menu=\"{id}\"]");
        if let Ok(Some(button)) = crate::utils::window::document().query_selector(&selector) {
            if let Ok(button) = button.dyn_into::<web_sys::HtmlElement>() {
                let _ = button.focus();
            }
        }
    });
    let outside = window_event_listener_untyped("pointerdown", move |ev| {
        if open.try_get_untracked().flatten().is_none() {
            return;
        }
        let inside = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|el| el.closest("[data-role=\"header-menu\"]").ok().flatten())
            .is_some();
        if !inside {
            open.set(None);
        }
    });
    on_cleanup(move || {
        escape.remove();
        outside.remove();
    });
}

/// A picked item (a link or a button) closes its menu; a status chip that is
/// only text does not.
fn close_on_item(ev: leptos::ev::MouseEvent, open: RwSignal<Option<&'static str>>) {
    let on_item = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest("a, button").ok().flatten())
        .is_some();
    if on_item {
        open.set(None);
    }
}

#[component]
fn HeaderMenu(
    id: &'static str,
    open: RwSignal<Option<&'static str>>,
    label: ViewFn,
    /// `data-state` of the button (Pripojenia: ok / checking / down).
    #[prop(optional, into)]
    state: Option<Signal<&'static str>>,
    /// The section shown is one of this menu's items.
    #[prop(optional, into)]
    active: Option<Signal<bool>>,
    #[prop(optional, into)] title: Option<Signal<String>>,
    children: Children,
) -> impl IntoView {
    let is_open = move || open.get() == Some(id);
    let toggle = move |_| {
        open.update(|current| *current = if *current == Some(id) { None } else { Some(id) });
    };
    let panel_id = format!("header-menu-{id}");
    view! {
        <div class="header-menu" data-role="header-menu" data-menu=id data-open=move || bool_attr(is_open())>
            <button
                type="button"
                class="header-menu__button"
                data-role="header-menu-toggle"
                data-menu=id
                data-state=move || state.map(|s| s.get()).unwrap_or("idle")
                data-active=move || bool_attr(active.is_some_and(|a| a.get()))
                aria-expanded=move || bool_attr(is_open())
                aria-controls=panel_id.clone()
                title=move || title.map(|t| t.get()).unwrap_or_default()
                on:click=toggle
            >
                {label.run()}
                <span class="header-menu__caret" aria-hidden="true">"\u{25be}"</span>
            </button>
            <div
                class="header-menu__panel"
                id=panel_id
                data-role="header-menu-panel"
                data-menu=id
                hidden=move || !is_open()
                on:click=move |ev| close_on_item(ev, open)
            >
                {children()}
            </div>
        </div>
    }
}

/// Stránky, Obrazovky and Pripojenia, in this order (after Piesne and Biblia).
#[component]
pub fn HeaderMenus(version: RwSignal<String>) -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let open = RwSignal::new(None::<&'static str>);
    close_on_escape_and_outside(open);

    let hosts = RwSignal::new(Vec::<HostLink>::new());
    let ai_state = RwSignal::new("checking");
    let health = Memo::new(move |_| hosts.with(|h| summarize(h, ai_state.get())));
    let health_text = Signal::derive(move || health.with(health_label));
    let pages_active = Signal::derive(move || {
        let view = ctx.view.get();
        menu_views().any(|(id, _)| id == view)
    });

    view! {
        <div class="header-menus" data-role="header-menus">
            <HeaderMenu
                id="pages"
                open=open
                active=pages_active
                label=ViewFn::from(|| view! { <span class="header-menu__label">"Stránky"</span> })
            >
                <a class="header-menu__item" data-role="header-menu-study" href="/ui/study">"Štúdium"</a>
                {menu_views().map(|(id, label)| view! {
                    <button
                        type="button"
                        class="header-menu__item"
                        data-role="view-toggle"
                        data-view=id
                        data-active=move || bool_attr(ctx.view.get() == id)
                        on:click=move |_| set_view(ctx.view, id)
                    >
                        {label}
                    </button>
                }).collect_view()}
            </HeaderMenu>
            <HeaderMenu
                id="screens"
                open=open
                label=ViewFn::from(|| view! { <span class="header-menu__label">"Obrazovky"</span> })
            >
                <SurfaceNav />
            </HeaderMenu>
            <HeaderMenu
                id="connections"
                open=open
                state=Signal::derive(move || health.with(health_state))
                title=health_text
                label=ViewFn::from(move || view! {
                    <span class="header-menu__dot" aria-hidden="true"></span>
                    <span class="header-menu__label" data-role="connections-label">{move || health_text.get()}</span>
                })
            >
                <div class="header-menu__status" data-role="connections-list">
                    <ResolumeStatusChips report=hosts />
                    <AiStatusChip report=ai_state />
                </div>
                <span class="header-menu__version operator__version-badge" data-role="header-version">
                    {move || version.get()}
                </span>
            </HeaderMenu>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(label: &str, state: &str) -> HostLink {
        HostLink {
            label: label.to_string(),
            state: state.to_string(),
        }
    }

    #[test]
    fn the_pages_menu_lists_every_section_without_its_own_button_in_order() {
        let ids: Vec<_> = menu_views().map(|(id, _)| id).collect();
        assert_eq!(ids, ["timers", "ai", "menovky"]);
    }

    #[test]
    fn everything_answered_is_ok() {
        let health = summarize(&[host("Bridge SNV", "connected")], "ok");
        assert_eq!(health, Health::Ok);
        assert_eq!(health_label(&health), "Pripojenia");
        assert_eq!(health_state(&health), "ok");
    }

    #[test]
    fn no_host_configured_and_ai_up_is_ok() {
        assert_eq!(summarize(&[], "ok"), Health::Ok);
    }

    #[test]
    fn a_host_in_error_names_itself_on_the_button() {
        let health = summarize(
            &[
                host("Bridge SNV", "connected"),
                host("CG Resolume", "error"),
            ],
            "ok",
        );
        assert_eq!(health, Health::Down(vec!["CG Resolume".to_string()]));
        assert_eq!(health_label(&health), "Odpojené: CG Resolume");
        assert_eq!(health_state(&health), "down");
    }

    #[test]
    fn ai_unavailable_is_down_and_listed_after_the_hosts() {
        let health = summarize(&[host("CG Resolume", "error")], "unavailable");
        assert_eq!(health_label(&health), "Odpojené: CG Resolume, AI");
    }

    #[test]
    fn a_first_poll_or_a_failed_last_call_is_not_an_outage() {
        assert_eq!(summarize(&[], "checking"), Health::Checking);
        assert_eq!(summarize(&[], "last_call_failed"), Health::Checking);
        assert_eq!(
            summarize(&[host("Bridge SNV", "connecting")], "ok"),
            Health::Checking
        );
    }
}
