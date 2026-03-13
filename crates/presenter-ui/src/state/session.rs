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
