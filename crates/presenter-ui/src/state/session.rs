use gloo_storage::{LocalStorage, SessionStorage, Storage};

const PREFIX: &str = "presenter:";

/// Get a value from session storage (per-tab state).
pub fn get(key: &str) -> Option<String> {
    SessionStorage::get(format!("{PREFIX}{key}")).ok()
}

/// Set a value in session storage (per-tab state).
pub fn set(key: &str, value: &str) {
    let _ = SessionStorage::set(format!("{PREFIX}{key}"), value.to_string());
}

/// Remove a value from session storage.
pub fn remove(key: &str) {
    SessionStorage::delete(format!("{PREFIX}{key}"));
}

/// Read a raw value from local storage WITHOUT throwing when storage is
/// unavailable (private mode, blocked site data) — gloo's `LocalStorage`
/// throws there. Stored unencoded, so never mix a key with `get_persistent`.
pub fn try_get_local(key: &str) -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok()??;
    storage.get_item(&format!("{PREFIX}{key}")).ok()?
}

/// Store a raw value in local storage; `false` when storage is unavailable.
pub fn try_set_local(key: &str, value: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .is_some_and(|storage| storage.set_item(&format!("{PREFIX}{key}"), value).is_ok())
}

/// Get a value from local storage (persistent across sessions).
/// Use for settings like lineLimit and catalogTopHeight.
pub fn get_persistent(key: &str) -> Option<String> {
    LocalStorage::get(format!("{PREFIX}{key}")).ok()
}

/// Set a value in local storage (persistent across sessions).
/// Use for settings like lineLimit and catalogTopHeight.
pub fn set_persistent(key: &str, value: &str) {
    let _ = LocalStorage::set(format!("{PREFIX}{key}"), value.to_string());
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
