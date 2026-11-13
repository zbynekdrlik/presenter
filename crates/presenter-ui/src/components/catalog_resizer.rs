//! The drag handles of the Piesne left column (#944).
//!
//! A desktop sizes Knižnice and Playlisty on their own: the handle between
//! them sets the library list's height, the handle under Playlisty the
//! playlist list's. A phone keeps its one box for both (`catalogTopHeight`),
//! sized by the handle under Playlisty as before. Every size is remembered
//! per browser (localStorage).

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::state::operator::{
    clamp_catalog_top_height, clamp_list_height, OperatorState, LIBRARIES_HEIGHT_KEY,
    PLAYLISTS_HEIGHT_KEY,
};
use crate::state::AppContext;

/// What one drag changes.
#[derive(Clone, Copy)]
struct ResizeTarget {
    size: RwSignal<f64>,
    storage_key: &'static str,
    /// The list whose RENDERED height a drag starts from (a short list is
    /// smaller than its stored maximum, and the first pixel must move it).
    /// `None` = start from the stored value.
    measured: Option<&'static str>,
    clamp: fn(f64) -> f64,
}

/// The height of the first element matching `selector`, if it is shown.
fn rendered_height(selector: &str) -> Option<f64> {
    let el = crate::utils::window::document()
        .query_selector(selector)
        .ok()??;
    let height = el.get_bounding_client_rect().height();
    (height > 0.0).then_some(height)
}

/// The handle between Knižnice and Playlisty (desktop only).
#[component]
pub fn LibraryResizer() -> impl IntoView {
    let op = use_ctx!(OperatorState);
    let desktop = ResizeTarget {
        size: op.catalog_libraries_height,
        storage_key: LIBRARIES_HEIGHT_KEY,
        measured: Some(r#"[data-view-panel="worship"] [data-role="library-list"]"#),
        clamp: clamp_list_height,
    };
    view! { <SectionResizer role="library-resizer" desktop=desktop phone=None /> }
}

/// The handle under Playlisty: the playlists on a desktop, the whole
/// libraries + playlists box on a phone.
#[component]
pub fn CatalogResizer() -> impl IntoView {
    let op = use_ctx!(OperatorState);
    let desktop = ResizeTarget {
        size: op.catalog_playlists_height,
        storage_key: PLAYLISTS_HEIGHT_KEY,
        measured: Some(r#"[data-view-panel="worship"] [data-role="playlist-list"]"#),
        clamp: clamp_list_height,
    };
    let phone = ResizeTarget {
        size: op.catalog_top_height,
        storage_key: "catalogTopHeight",
        measured: None,
        clamp: clamp_catalog_top_height,
    };
    view! { <SectionResizer role="catalog-resizer" desktop=desktop phone=Some(phone) /> }
}

/// One drag handle: sizes `desktop`, or `phone` (when given) in the phone shell.
#[component]
fn SectionResizer(
    role: &'static str,
    desktop: ResizeTarget,
    phone: Option<ResizeTarget>,
) -> impl IntoView {
    let phone_shell = use_ctx!(AppContext).phone_shell;

    let on_pointerdown = move |ev: web_sys::PointerEvent| {
        let target = match phone {
            Some(phone) if phone_shell.get_untracked() => phone,
            _ => desktop,
        };
        let start_y = ev.client_y() as f64;
        let start_height = target
            .measured
            .and_then(rendered_height)
            .unwrap_or_else(|| target.size.get_untracked());
        let pointer_id = ev.pointer_id();

        let handle = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        if let Some(el) = &handle {
            let _ = el.set_pointer_capture(pointer_id);
        }

        // The move handler is handed to JS (dropped by its GC).
        let on_move: Closure<dyn Fn(web_sys::PointerEvent)> =
            Closure::new(move |ev: web_sys::PointerEvent| {
                let dy = ev.client_y() as f64 - start_y;
                target.size.set((target.clamp)(start_height + dy));
            });
        let on_move_fn = on_move.into_js_value();
        let on_move_fn_for_cleanup = on_move_fn.clone();
        let handle_for_cleanup = handle.clone();

        // The drag ends when the handle loses the pointer: after pointerup,
        // and also when the browser cancels the gesture (a touch screen),
        // which fires no pointerup. Store the size, remove the move listener.
        let on_up = Closure::once_into_js(move |_ev: web_sys::PointerEvent| {
            let height = target.size.get_untracked();
            crate::state::session::set_persistent(target.storage_key, &height.to_string());
            if let Some(el) = handle_for_cleanup {
                let _ = el.remove_event_listener_with_callback(
                    "pointermove",
                    on_move_fn_for_cleanup.unchecked_ref(),
                );
            }
        });

        if let Some(el) = &handle {
            let _ = el.add_event_listener_with_callback("pointermove", on_move_fn.unchecked_ref());
            let opts = web_sys::AddEventListenerOptions::new();
            opts.set_once(true);
            let _ = el.add_event_listener_with_callback_and_add_event_listener_options(
                "lostpointercapture",
                on_up.unchecked_ref(),
                &opts,
            );
        }
    };

    view! {
        <div
            class="operator__catalog-resizer"
            data-role=role
            aria-hidden="true"
            on:pointerdown=on_pointerdown
        ></div>
    }
}
