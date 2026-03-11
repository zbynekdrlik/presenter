use super::{get_json, ApiError};
use presenter_core::Presentation;

/// Fetch a single presentation with all slides.
pub async fn get_presentation(id: &str) -> Result<Presentation, ApiError> {
    get_json(&format!("/presentations/{id}")).await
}
