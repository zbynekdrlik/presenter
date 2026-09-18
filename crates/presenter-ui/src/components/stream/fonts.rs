//! Uploaded-web-font wiring for the stream OUTPUT and EDITOR pages (#778).
//!
//! Two tiny helpers, deliberately kept OUT of the page files' message-handling
//! so the parallel element/opacity lanes are untouched:
//!
//! - [`ensure_fonts_css_link`] injects/refreshes a `<link rel="stylesheet"
//!   href="/stream/fonts.css?v=N">` in `<head>` at runtime (not in
//!   `index.html`, so operator/stage pages never pay for it). The generated
//!   stylesheet is served `no-cache` + ETag, so re-injecting with a bumped `v`
//!   (after an upload / on a config bump) always revalidates.
//! - [`spawn_font_gate`] preloads every uploaded face via `document.fonts.load`
//!   and flips a `ready` signal once they settle (bounded at 2 s), so the output
//!   page can keep text hidden until the real fonts are ready — no fallback-font
//!   flash on a scene take. `font-display: block` in the generated CSS is the
//!   belt-and-suspenders native guard.

use leptos::prelude::*;
use presenter_core::StreamFont;

/// Marker attribute so the same `<link>` is reused (href updated) across bumps.
const LINK_ROLE: &str = "stream-fonts-css";

/// Inject the generated font stylesheet link, or update its `?v=` if present.
pub fn ensure_fonts_css_link(version: u64) {
    let doc = crate::utils::window::document();
    let href = format!("/stream/fonts.css?v={version}");

    if let Ok(Some(existing)) = doc.query_selector(&format!("link[data-role='{LINK_ROLE}']")) {
        let _ = existing.set_attribute("href", &href);
        return;
    }

    let Ok(link) = doc.create_element("link") else {
        return;
    };
    let _ = link.set_attribute("rel", "stylesheet");
    let _ = link.set_attribute("data-role", LINK_ROLE);
    let _ = link.set_attribute("href", &href);
    if let Ok(Some(head)) = doc.query_selector("head") {
        let _ = head.append_child(&link);
    }
}

/// Build the CSS font spec (`<weight> 1em "<family>"`) the browser's
/// `FontFaceSet.load` accepts. Quotes/backslashes are stripped (upload already
/// rejects them, this is defense in depth).
fn face_spec(font: &StreamFont) -> String {
    let family = font.family.replace('"', "").replace('\\', "");
    format!("{} 1em \"{}\"", font.weight, family)
}

/// Fetch the uploaded faces, preload each one, then flip `ready` true. A 2 s
/// timeout is armed as a safety net so a slow/broken face never leaves text
/// hidden forever. A failure to list the fonts reveals immediately (no gate).
pub fn spawn_font_gate(ready: RwSignal<bool>) {
    leptos::task::spawn_local(async move {
        let faces = match crate::api::get_json::<Vec<StreamFont>>("/stream/api/fonts").await {
            Ok(list) => list,
            Err(e) => {
                leptos::logging::log!("stream fonts: list failed, revealing text: {e}");
                ready.set(true);
                return;
            }
        };
        let font_set = crate::utils::window::document().fonts();
        for font in &faces {
            // `load` returns a Promise that resolves when the face is ready (or
            // rejects if unavailable — ignored, the timeout still reveals).
            let promise = font_set.load(&face_spec(font));
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        }
        leptos::logging::log!("stream fonts: {} face(s) preloaded", faces.len());
        ready.set(true);
    });

    // Safety net: reveal after 2 s no matter what (idempotent signal set).
    gloo_timers::callback::Timeout::new(2000, move || ready.set(true)).forget();
}
