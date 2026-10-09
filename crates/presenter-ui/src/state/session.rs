//! Browser storage of UI state: per tab (`sessionStorage`) and per browser
//! (`localStorage`). Every access is NON-THROWING — with storage unavailable
//! (private mode, blocked site data) a read is `None` and a write is dropped,
//! where gloo-storage used to throw and kill the handler (#832). Values stay
//! stored as JSON strings (`"\"5\""`), the format gloo-storage wrote, so values
//! saved by earlier builds (and the E2E specs' seeded `sessionStorage`) still
//! read back.

const PREFIX: &str = "presenter:";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok()?
}

/// The stored form of `value`: a JSON string.
fn encode_stored(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// The value of a stored JSON string; `None` for anything else.
fn decode_stored(raw: &str) -> Option<String> {
    serde_json::from_str(raw).ok()
}

fn read(storage: Option<web_sys::Storage>, key: &str) -> Option<String> {
    let raw = storage?.get_item(&format!("{PREFIX}{key}")).ok()??;
    decode_stored(&raw)
}

fn write(storage: Option<web_sys::Storage>, key: &str, value: &str) {
    if let Some(storage) = storage {
        let _ = storage.set_item(&format!("{PREFIX}{key}"), &encode_stored(value));
    }
}

/// Get a value from session storage (per-tab state).
pub fn get(key: &str) -> Option<String> {
    read(session_storage(), key)
}

/// Set a value in session storage (per-tab state).
pub fn set(key: &str, value: &str) {
    write(session_storage(), key, value);
}

/// Remove a value from session storage.
pub fn remove(key: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(&format!("{PREFIX}{key}"));
    }
}

/// Get a value from local storage (persistent across sessions).
/// Use for settings like lineLimit, catalogTopHeight and slide columns.
pub fn get_persistent(key: &str) -> Option<String> {
    read(local_storage(), key)
}

/// Set a value in local storage (persistent across sessions).
/// Use for settings like lineLimit, catalogTopHeight and slide columns.
pub fn set_persistent(key: &str, value: &str) {
    write(local_storage(), key, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_stay_json_strings_like_gloo_storage_wrote_them() {
        // Values saved by earlier builds (gloo-storage) are JSON strings, and
        // E2E specs seed `sessionStorage` the same way — keep the format.
        assert_eq!(encode_stored("5"), "\"5\"");
        assert_eq!(encode_stored("Ján \"1\""), "\"Ján \\\"1\\\"\"");
        assert_eq!(decode_stored("\"5\""), Some("5".to_string()));
        assert_eq!(
            decode_stored(&encode_stored("Ján \"1\"")),
            Some("Ján \"1\"".to_string())
        );
    }

    #[test]
    fn a_value_that_is_not_a_json_string_reads_as_absent() {
        assert_eq!(decode_stored("5"), None);
        assert_eq!(decode_stored(""), None);
        assert_eq!(decode_stored("{"), None);
    }
}
