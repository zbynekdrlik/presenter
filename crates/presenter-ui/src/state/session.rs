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
