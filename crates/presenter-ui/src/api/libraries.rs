use super::{get_json, ApiError};
use presenter_core::{Library, LibrarySummary, PresentationSummary};

/// Fetch all library summaries.
pub async fn list_libraries() -> Result<Vec<LibrarySummary>, ApiError> {
    get_json("/libraries").await
}

/// Fetch a single library with its presentations.
pub async fn get_library(id: &str) -> Result<Library, ApiError> {
    get_json(&format!("/libraries/{id}")).await
}

/// Fetch presentations in a library.
pub async fn list_presentations(library_id: &str) -> Result<Vec<PresentationSummary>, ApiError> {
    get_json(&format!("/libraries/{library_id}/presentations")).await
}
