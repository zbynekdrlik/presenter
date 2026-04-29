use super::{delete, delete_json, get_json, patch_no_content, post_form_data, post_json, ApiError};
use presenter_core::{LibraryId, LibrarySummary, Presentation, Slide};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationDetail {
    pub library_id: LibraryId,
    pub library_name: String,
    pub presentation: Presentation,
}

pub async fn get_presentation(id: &str) -> Result<PresentationDetail, ApiError> {
    get_json(&format!("/presentations/{id}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePresentationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slides: Option<Vec<SlideInput>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideInput {
    pub main: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePresentationResponse {
    pub library_id: String,
    pub presentation: Presentation,
    pub library_summary: Option<LibrarySummary>,
}

pub async fn create_blank(
    library_id: &str,
    name: &str,
) -> Result<CreatePresentationResponse, ApiError> {
    post_json(
        &format!("/libraries/{library_id}/presentations"),
        &CreatePresentationRequest {
            name: Some(name.to_string()),
            slides: None,
        },
    )
    .await
}

pub async fn create_from_paste(
    library_id: &str,
    name: &str,
    slides: Vec<SlideInput>,
) -> Result<CreatePresentationResponse, ApiError> {
    post_json(
        &format!("/libraries/{library_id}/presentations"),
        &CreatePresentationRequest {
            name: Some(name.to_string()),
            slides: Some(slides),
        },
    )
    .await
}

pub async fn import_pro(
    library_id: &str,
    form_data: &web_sys::FormData,
) -> Result<CreatePresentationResponse, ApiError> {
    post_form_data(
        &format!("/libraries/{library_id}/presentations/import"),
        form_data,
    )
    .await
}

#[derive(Serialize)]
struct RenameRequest {
    name: String,
}

pub async fn rename(id: &str, name: &str) -> Result<(), ApiError> {
    patch_no_content(
        &format!("/presentations/{id}"),
        &RenameRequest {
            name: name.to_string(),
        },
    )
    .await
}

pub async fn delete_presentation(id: &str) -> Result<(), ApiError> {
    delete(&format!("/presentations/{id}")).await
}

#[derive(Serialize)]
struct InsertSlideRequest {
    position: Option<u32>,
}

pub async fn insert_slide(pres_id: &str, position: Option<u32>) -> Result<Vec<Slide>, ApiError> {
    post_json(
        &format!("/presentations/{pres_id}/slides"),
        &InsertSlideRequest { position },
    )
    .await
}

pub async fn delete_slide(pres_id: &str, slide_id: &str) -> Result<Vec<Slide>, ApiError> {
    delete_json(&format!("/presentations/{pres_id}/slides/{slide_id}")).await
}

pub async fn duplicate_slide(pres_id: &str, slide_id: &str) -> Result<Vec<Slide>, ApiError> {
    post_json(
        &format!("/presentations/{pres_id}/slides/{slide_id}/duplicate"),
        &serde_json::json!({}),
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSlideRequest {
    main: String,
    translation: String,
    stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

pub async fn update_slide(
    pres_id: &str,
    slide_id: &str,
    main: &str,
    translation: &str,
    stage: &str,
) -> Result<Slide, ApiError> {
    super::patch_json(
        &format!("/presentations/{pres_id}/slides/{slide_id}"),
        &UpdateSlideRequest {
            main: main.to_string(),
            translation: translation.to_string(),
            stage: stage.to_string(),
            group: None,
        },
    )
    .await
}

pub async fn update_slide_with_group(
    pres_id: &str,
    slide_id: &str,
    main: &str,
    translation: &str,
    stage: &str,
    group: Option<String>,
) -> Result<Slide, ApiError> {
    super::patch_json(
        &format!("/presentations/{pres_id}/slides/{slide_id}"),
        &UpdateSlideRequest {
            main: main.to_string(),
            translation: translation.to_string(),
            stage: stage.to_string(),
            group,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReorderSlidesRequest {
    slide_ids: Vec<String>,
}

pub async fn reorder_slides(pres_id: &str, slide_ids: Vec<String>) -> Result<Vec<Slide>, ApiError> {
    post_json(
        &format!("/presentations/{pres_id}/slides/reorder"),
        &ReorderSlidesRequest { slide_ids },
    )
    .await
}

pub async fn fetch_group_colors() -> Result<HashMap<String, String>, ApiError> {
    get_json("/group-colors").await
}
