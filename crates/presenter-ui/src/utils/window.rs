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

/// Read a query-string parameter's raw string value from the current URL.
///
/// Returns `None` when the parameter is absent (or the URL/search cannot be
/// parsed). Used by the stream output page's preview mode
/// (`?preview=1&scene=<id>&overlays=<id,id>`, #709) to read the forced base
/// scene and overlay ids. `url_flag_enabled` above is the boolean-only sibling.
pub fn url_param(name: &str) -> Option<String> {
    window()
        .location()
        .search()
        .ok()
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get(name))
}

/// Set ONE query parameter on the current URL, keeping every other one (and the
/// `#hash`), without a new history entry (`history.replaceState`). The stream
/// editor mirrors both `?output=` and `?tab=` this way (#829), so neither
/// drops the other. `value` must already be URL-safe (a slug / a fixed id).
pub fn replace_url_param(name: &str, value: &str) {
    let win = window();
    let location = win.location();
    let search = location.search().unwrap_or_default();
    let hash = location.hash().unwrap_or_default();
    let new_url = format!(
        "{}{}{hash}",
        current_pathname(),
        query_with_param(&search, name, value)
    );
    if let Ok(history) = win.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&new_url));
    }
}

/// `search` (a `location.search`, with or without the leading `?`) with
/// `name=value` set: an existing `name` pair is replaced IN PLACE (later
/// duplicates dropped), otherwise it is appended; every other pair is kept
/// verbatim. Always returns a `?`-prefixed query.
pub fn query_with_param(search: &str, name: &str, value: &str) -> String {
    let mut replaced = false;
    let mut pairs: Vec<String> = Vec::new();
    for pair in search.trim_start_matches('?').split('&') {
        if pair.is_empty() {
            continue;
        }
        if pair.split('=').next() == Some(name) {
            if !replaced {
                pairs.push(format!("{name}={value}"));
                replaced = true;
            }
        } else {
            pairs.push(pair.to_string());
        }
    }
    if !replaced {
        pairs.push(format!("{name}={value}"));
    }
    format!("?{}", pairs.join("&"))
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

#[cfg(test)]
mod tests {
    use super::query_with_param;

    #[test]
    fn sets_a_param_on_an_empty_query() {
        assert_eq!(query_with_param("", "tab", "fonts"), "?tab=fonts");
        assert_eq!(query_with_param("?", "tab", "fonts"), "?tab=fonts");
    }

    #[test]
    fn keeps_the_other_params_when_adding_one() {
        assert_eq!(
            query_with_param("?output=timer", "tab", "nameplates"),
            "?output=timer&tab=nameplates"
        );
    }

    #[test]
    fn replaces_an_existing_param_in_place() {
        assert_eq!(
            query_with_param("?output=stream&tab=scenes&x=1", "output", "timer"),
            "?output=timer&tab=scenes&x=1"
        );
    }

    #[test]
    fn drops_duplicate_pairs_and_matches_whole_names_only() {
        assert_eq!(
            query_with_param("tab=a&tabs=keep&tab=b&flag", "tab", "c"),
            "?tab=c&tabs=keep&flag"
        );
    }
}
