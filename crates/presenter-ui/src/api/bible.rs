use super::{get_json, post_json, ApiError};
use presenter_core::{BibleBroadcast, BiblePreferences, BibleTranslation};
use serde::{Deserialize, Serialize};

/// Fetch available Bible translations.
pub async fn list_translations() -> Result<Vec<BibleTranslation>, ApiError> {
    get_json("/bible/translations").await
}

/// Fetch Bible preferences.
pub async fn get_preferences() -> Result<BiblePreferences, ApiError> {
    get_json("/bible/preferences").await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub translation: String,
}

#[derive(Deserialize)]
pub struct SearchResult {
    pub results: Vec<BibleSearchHit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSearchHit {
    pub reference: String,
    pub text: String,
}

/// Search Bible text.
pub async fn search(query: &str, translation: &str) -> Result<SearchResult, ApiError> {
    post_json(
        "/bible/search",
        &SearchRequest {
            query: query.to_string(),
            translation: translation.to_string(),
        },
    )
    .await
}

/// Get the current Bible broadcast state.
pub async fn get_broadcast() -> Result<Option<BibleBroadcast>, ApiError> {
    get_json("/bible/broadcast").await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRequest {
    pub reference: String,
    pub translation: String,
}

/// Broadcast a Bible passage to stage displays.
pub async fn broadcast(reference: &str, translation: &str) -> Result<BibleBroadcast, ApiError> {
    post_json(
        "/bible/broadcast",
        &BroadcastRequest {
            reference: reference.to_string(),
            translation: translation.to_string(),
        },
    )
    .await
}

/// Clear the current Bible broadcast.
pub async fn clear_broadcast() -> Result<(), ApiError> {
    super::delete("/bible/broadcast").await
}
