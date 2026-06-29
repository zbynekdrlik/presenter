use web_sys::{Document, HtmlElement, Window};

/// Get the browser window object.
pub fn window() -> Window {
    web_sys::window().expect("no global window")
}

/// Get the document object.
pub fn document() -> Document {
    window().document().expect("no document")
}

/// Get the document body.
pub fn document_body() -> Option<HtmlElement> {
    document().body()
}

/// Get the current pathname from the URL.
pub fn current_pathname() -> String {
    window()
        .location()
        .pathname()
        .unwrap_or_else(|_| "/".to_string())
}

/// True when the current URL carries `?<name>=1` (or `=true`).
///
/// Used to detect the operator-header stage preview (`/stage?preview=1`, #460):
/// a preview stage page connects to `/live/ws` for live events but tags its
/// socket so the server excludes it from the stage-monitor count.
pub fn url_flag_enabled(name: &str) -> bool {
    window()
        .location()
        .search()
        .ok()
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get(name))
        .map(|value| value == "1" || value == "true")
        .unwrap_or(false)
}
