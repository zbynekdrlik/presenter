use super::{delete, get_json, patch_no_content, post_json, post_no_content, ApiError};
use presenter_core::{Library, LibrarySummary, PresentationSummary};
use serde::{Deserialize, Serialize};

pub async fn list_libraries() -> Result<Vec<LibrarySummary>, ApiError> {
    get_json("/libraries/summary").await
}

/// Get a specific library by ID from the summary endpoint.
/// Note: There's no dedicated GET /libraries/{id} endpoint, so we fetch all and filter.
pub async fn get_library(id: &str) -> Result<LibrarySummary, ApiError> {
    let libraries: Vec<LibrarySummary> = get_json("/libraries/summary").await?;
    libraries
        .into_iter()
        .find(|lib| lib.id.to_string() == id)
        .ok_or_else(|| ApiError::NotFound("Library not found".to_string()))
}

/// Get presentations for a library by fetching from the summary endpoint.
pub async fn list_presentations(library_id: &str) -> Result<Vec<PresentationSummary>, ApiError> {
    let library = get_library(library_id).await?;
    Ok(library.presentations)
}

#[derive(Serialize)]
struct CreateLibraryRequest {
    name: String,
}

pub async fn create_library(name: &str) -> Result<Library, ApiError> {
    post_json(
        "/libraries",
        &CreateLibraryRequest {
            name: name.to_string(),
        },
    )
    .await
}

#[derive(Serialize)]
struct RenameLibraryRequest {
    name: String,
}

pub async fn rename_library(id: &str, name: &str) -> Result<(), ApiError> {
    patch_no_content(
        &format!("/libraries/{id}"),
        &RenameLibraryRequest {
            name: name.to_string(),
        },
    )
    .await
}

pub async fn delete_library(id: &str) -> Result<(), ApiError> {
    delete(&format!("/libraries/{id}")).await
}

#[derive(Serialize)]
struct SetFavoriteRequest {
    favorite: bool,
}

pub async fn set_favorite(id: &str, favorite: bool) -> Result<(), ApiError> {
    post_no_content(
        &format!("/libraries/{id}/favorite"),
        &SetFavoriteRequest { favorite },
    )
    .await
}

#[derive(Deserialize)]
pub struct FavoriteLibraryIdsResponse {
    pub ids: Vec<String>,
}

pub async fn get_favorites() -> Result<Vec<String>, ApiError> {
    let resp: FavoriteLibraryIdsResponse = get_json("/libraries/favorites").await?;
    Ok(resp.ids)
}
