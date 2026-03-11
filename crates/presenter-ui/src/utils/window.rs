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
