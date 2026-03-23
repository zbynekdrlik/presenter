use super::{get_json, post_json, post_no_content, ApiError};
use presenter_core::{
    BibleBroadcast, BiblePreferences, BiblePreferencesDraft, BibleSlideOutput, BibleTranslation,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Translations
// ---------------------------------------------------------------------------

/// Fetch available Bible translations.
pub async fn list_translations() -> Result<Vec<BibleTranslation>, ApiError> {
    get_json("/bible/translations").await
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// Fetch Bible preferences.
pub async fn get_preferences() -> Result<BiblePreferences, ApiError> {
    get_json("/bible/preferences").await
}

/// Update Bible preferences.
pub async fn update_preferences(draft: &BiblePreferencesDraft) -> Result<(), ApiError> {
    super::put_no_content("/bible/preferences", draft).await
}

// ---------------------------------------------------------------------------
// Books
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleBookDto {
    pub book: String,
    pub code: String,
    pub number: u16,
    pub chapters: Vec<BibleChapterDto>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleChapterDto {
    pub number: u16,
    pub verse_count: u16,
}

/// List books for a given translation.
pub async fn list_books(translation: &str) -> Result<Vec<BibleBookDto>, ApiError> {
    get_json(&format!(
        "/bible/books?translation={}",
        urlencoding::encode(translation)
    ))
    .await
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSearchHit {
    pub reference: BibleSearchReference,
    pub translation: BibleTranslation,
    pub text: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSearchReference {
    pub book: String,
    #[serde(default)]
    pub book_code: Option<String>,
    #[serde(default)]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
}

impl BibleSearchReference {
    pub fn to_human_readable(&self) -> String {
        if self.verse_start == self.verse_end {
            format!("{} {}:{}", self.book, self.chapter, self.verse_start)
        } else {
            format!(
                "{} {}:{}-{}",
                self.book, self.chapter, self.verse_start, self.verse_end
            )
        }
    }
}

/// Search Bible text. Server endpoint: GET /bible/search?query=&translation=&limit=
pub async fn search(
    query: &str,
    translation: &str,
    limit: Option<u32>,
) -> Result<Vec<BibleSearchHit>, ApiError> {
    let limit_param = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
    get_json(&format!(
        "/bible/search?query={}&translation={}{}",
        urlencoding::encode(query),
        urlencoding::encode(translation),
        limit_param,
    ))
    .await
}

// ---------------------------------------------------------------------------
// Active broadcast
// ---------------------------------------------------------------------------

/// Get the current Bible broadcast state. Server endpoint: GET /bible/active
pub async fn get_broadcast() -> Result<Option<BibleBroadcast>, ApiError> {
    get_json("/bible/active").await
}

/// Clear the current Bible broadcast. Server endpoint: POST /bible/clear
pub async fn clear_broadcast() -> Result<(), ApiError> {
    post_no_content("/bible/clear", &serde_json::json!({})).await
}

// ---------------------------------------------------------------------------
// Trigger (structured reference)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRequest {
    pub translation: String,
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verse_start: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_end: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_reference_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation_reference_label: Option<String>,
}

/// Trigger a Bible broadcast (structured reference). Server: POST /bible/trigger
pub async fn trigger(req: &TriggerRequest) -> Result<BibleBroadcast, ApiError> {
    post_json("/bible/trigger", req).await
}

// ---------------------------------------------------------------------------
// Trigger slide (single-source-of-truth)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSlideRequest {
    pub main_text: String,
    pub main_reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapter: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_end: Option<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSlideResponse {
    pub success: bool,
    pub output: BibleSlideOutput,
}

/// Trigger a slide via single-source-of-truth. Server: POST /bible/trigger-slide
pub async fn trigger_slide(req: &TriggerSlideRequest) -> Result<TriggerSlideResponse, ApiError> {
    post_json("/bible/trigger-slide", req).await
}

// ---------------------------------------------------------------------------
// Resolve (generate slides from reference)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    pub main_translation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_translation: Option<String>,
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    pub chapter: u16,
    pub verse_start: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_end: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResponse {
    pub main_translation: BibleTranslation,
    pub secondary_translation: Option<BibleTranslation>,
    pub slides: Vec<BibleSlideDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideDto {
    pub id: String,
    pub order: u32,
    pub main: String,
    pub translation: String,
    pub stage: String,
    pub group: Option<String>,
    pub metadata: Option<BibleSlideMetadataDto>,
    pub main_reference: Option<String>,
    pub translation_reference: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideMetadataDto {
    pub bible: Option<BibleSlideMetaBible>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideMetaBible {
    pub translation_code: Option<String>,
    pub book: Option<String>,
    pub book_code: Option<String>,
    pub book_number: Option<u16>,
    pub chapter: Option<u16>,
    /// Flat verse range (used by trigger-slide API responses)
    pub verse_start: Option<u16>,
    pub verse_end: Option<u16>,
    /// Structured verse ranges (used by resolve/presentation detail responses)
    #[serde(default)]
    pub verses: Vec<BibleSlideVerseRefDto>,
    pub main_reference_label: Option<String>,
    pub translation_reference_label: Option<String>,
}

impl BibleSlideMetaBible {
    /// Get the effective verse start, preferring `verses` array over flat fields.
    pub fn effective_verse_start(&self) -> Option<u16> {
        self.verses.first().map(|v| v.start).or(self.verse_start)
    }

    /// Get the effective verse end, preferring `verses` array over flat fields.
    pub fn effective_verse_end(&self) -> Option<u16> {
        self.verses.last().map(|v| v.end).or(self.verse_end)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideVerseRefDto {
    pub start: u16,
    pub end: u16,
}

/// Generate slides from a Bible reference. Server: POST /bible/resolve
pub async fn resolve_slides(req: &ResolveRequest) -> Result<ResolveResponse, ApiError> {
    post_json("/bible/resolve", req).await
}

// ---------------------------------------------------------------------------
// Presentations
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentationSummary {
    pub id: String,
    pub name: String,
    pub slide_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentationDetail {
    pub id: String,
    pub name: String,
    pub slides: Vec<BibleSlideDto>,
}

/// List Bible presentations. Server: GET /bible/presentations
pub async fn list_presentations() -> Result<Vec<BiblePresentationSummary>, ApiError> {
    get_json("/bible/presentations").await
}

/// Create a new Bible presentation. Server: POST /bible/presentations
pub async fn create_presentation(name: &str) -> Result<BiblePresentationDetail, ApiError> {
    post_json("/bible/presentations", &serde_json::json!({ "name": name })).await
}

/// Get a Bible presentation by ID. Server: GET /bible/presentations/{id}
pub async fn get_presentation(id: &str) -> Result<BiblePresentationDetail, ApiError> {
    get_json(&format!("/bible/presentations/{id}")).await
}

/// Rename a Bible presentation. Server: PATCH /bible/presentations/{id}
pub async fn rename_presentation(id: &str, name: &str) -> Result<(), ApiError> {
    super::patch_no_content(
        &format!("/bible/presentations/{id}"),
        &serde_json::json!({ "name": name }),
    )
    .await
}

/// Delete a Bible presentation. Server: DELETE /bible/presentations/{id}
pub async fn delete_presentation(id: &str) -> Result<(), ApiError> {
    super::delete(&format!("/bible/presentations/{id}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendSlidesRequest {
    pub slides: Vec<AppendSlideInput>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendSlideInput {
    pub main: String,
    pub translation: String,
    pub stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BibleSlideMetadataDto>,
}

/// Append slides to a Bible presentation. Server: POST /bible/presentations/{id}/append
pub async fn append_presentation_slides(
    id: &str,
    slides: &[AppendSlideInput],
) -> Result<BiblePresentationDetail, ApiError> {
    post_json(
        &format!("/bible/presentations/{id}/append"),
        &AppendSlidesRequest {
            slides: slides.to_vec(),
        },
    )
    .await
}

/// Delete a slide from a Bible presentation. Uses the generic presentations endpoint.
/// Server: DELETE /presentations/{id}/slides/{slide_id}
pub async fn delete_presentation_slide(
    presentation_id: &str,
    slide_id: &str,
) -> Result<(), ApiError> {
    // The generic endpoint returns Vec<Slide> but we just need success
    let _slides: Vec<serde_json::Value> = super::delete_json(&format!(
        "/presentations/{presentation_id}/slides/{slide_id}"
    ))
    .await?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReorderSlidesRequest {
    slide_ids: Vec<String>,
}

/// Reorder slides in a Bible presentation. Uses the generic presentations endpoint.
/// Server: POST /presentations/{id}/slides/reorder
pub async fn reorder_presentation_slides(
    presentation_id: &str,
    slide_ids: Vec<String>,
) -> Result<(), ApiError> {
    let _slides: Vec<serde_json::Value> = super::post_json(
        &format!("/presentations/{presentation_id}/slides/reorder"),
        &ReorderSlidesRequest { slide_ids },
    )
    .await?;
    Ok(())
}
