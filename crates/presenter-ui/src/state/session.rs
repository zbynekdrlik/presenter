use gloo_storage::{SessionStorage, Storage};

const PREFIX: &str = "presenter:";

/// Get a value from session storage.
pub fn get(key: &str) -> Option<String> {
    SessionStorage::get(&format!("{PREFIX}{key}")).ok()
}

/// Set a value in session storage.
pub fn set(key: &str, value: &str) {
    let _ = SessionStorage::set(&format!("{PREFIX}{key}"), value.to_string());
}

/// Remove a value from session storage.
pub fn remove(key: &str) {
    SessionStorage::delete(&format!("{PREFIX}{key}"));
}
