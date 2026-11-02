//! The sermon API (#851 spec §11) as the study page uses it. Every write
//! carries the revision the page read and answers the whole sermon; a refusal
//! keeps the server's Slovak text (`ApiError::Refused`).

use chrono::NaiveDate;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use serde::{Deserialize, Serialize};

use super::{
    delete_json_refusal, delete_refusal, finish_json, get_json, patch_json_refusal,
    post_json_refusal, start_keepalive, ApiError,
};

/// A save that never answers turns into a failure instead of holding the queue.
const SAVE_TIMEOUT_MS: u32 = 15_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SermonSummary {
    pub id: String,
    pub title: String,
    pub service_date: NaiveDate,
    pub author_email: Option<String>,
    pub item_count: usize,
    pub slide_count: usize,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SermonList {
    pub today: NaiveDate,
    pub default_service_date: NaiveDate,
    pub sermons: Vec<SermonSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SermonItem {
    pub id: String,
    pub translation: String,
    pub book_code: String,
    pub book: String,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
    pub source: Vec<SourceVerse>,
    pub edits: ItemEdits,
    pub source_changed: bool,
    pub slide_ids: Vec<String>,
    pub label: String,
    pub rendered_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SermonDetail {
    pub id: String,
    pub title: String,
    pub service_date: NaiveDate,
    pub default_translation: String,
    pub revision: i64,
    pub items: Vec<SermonItem>,
    pub slides: Vec<SermonSlide>,
    pub secondary_missing: bool,
}

/// A stored slide, as the wall would get it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SermonSlide {
    pub id: String,
    pub main_text: String,
    pub main_reference: String,
    pub secondary_text: String,
    pub secondary_reference: String,
}

/// A passage of one chapter, as an item is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonPassage {
    pub translation: String,
    pub book_code: String,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewSermon<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_translation: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewItem<'a> {
    expected_revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<usize>,
    translation: &'a str,
    book_code: &'a str,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemOrder<'a> {
    expected_revision: i64,
    item_ids: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Revision {
    expected_revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewEdits<'a> {
    expected_revision: i64,
    verses: &'a ItemEdits,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemPatch<'a> {
    expected_revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<&'a str>,
    refresh_source: bool,
}

pub async fn list_sermons() -> Result<SermonList, ApiError> {
    get_json("/bible/sermons").await
}

/// The sermons of the server's today (the console, the tablet).
pub async fn todays_sermons() -> Result<Vec<SermonDetail>, ApiError> {
    get_json("/bible/sermons/today").await
}

pub async fn get_sermon(id: &str) -> Result<SermonDetail, ApiError> {
    get_json(&format!("/bible/sermons/{id}")).await
}

/// `None` = the server's defaults (its clock, never the phone's). The default
/// translation is the one the preacher reads; the NLT can be no sermon's.
pub async fn create_sermon(
    title: Option<&str>,
    service_date: Option<NaiveDate>,
    default_translation: Option<&str>,
) -> Result<SermonDetail, ApiError> {
    post_json_refusal(
        "/bible/sermons",
        &NewSermon {
            title,
            service_date,
            default_translation,
        },
    )
    .await
}

pub async fn delete_sermon(id: &str) -> Result<(), ApiError> {
    delete_refusal(&format!("/bible/sermons/{id}")).await
}

pub async fn add_item(
    id: &str,
    revision: i64,
    position: Option<usize>,
    passage: &SermonPassage,
) -> Result<SermonDetail, ApiError> {
    post_json_refusal(
        &format!("/bible/sermons/{id}/items"),
        &NewItem {
            expected_revision: revision,
            position,
            translation: &passage.translation,
            book_code: &passage.book_code,
            chapter: passage.chapter,
            verse_start: passage.verse_start,
            verse_end: passage.verse_end,
        },
    )
    .await
}

pub async fn reorder_items(
    id: &str,
    revision: i64,
    item_ids: &[String],
) -> Result<SermonDetail, ApiError> {
    let started = start_keepalive(
        "POST",
        &format!("/bible/sermons/{id}/items/reorder"),
        &ItemOrder {
            expected_revision: revision,
            item_ids,
        },
        SAVE_TIMEOUT_MS,
    );
    finish_json(started).await
}

pub async fn regenerate(id: &str, revision: i64) -> Result<SermonDetail, ApiError> {
    post_json_refusal(
        &format!("/bible/sermons/{id}/regenerate"),
        &Revision {
            expected_revision: revision,
        },
    )
    .await
}

/// A `keepalive` fetch: the browser cancels a plain one when the tab closes, and
/// the last taps go out exactly then. The caller's task must run in the same
/// turn as the page's last event, which `spawn_local` does.
pub async fn put_edits(
    id: &str,
    item_id: &str,
    revision: i64,
    edits: &ItemEdits,
) -> Result<SermonDetail, ApiError> {
    let started = start_keepalive(
        "PUT",
        &format!("/bible/sermons/{id}/items/{item_id}/edits"),
        &NewEdits {
            expected_revision: revision,
            verses: edits,
        },
        SAVE_TIMEOUT_MS,
    );
    finish_json(started).await
}

pub async fn change_translation(
    id: &str,
    item_id: &str,
    revision: i64,
    translation: &str,
) -> Result<SermonDetail, ApiError> {
    patch_item(id, item_id, revision, Some(translation), false).await
}

pub async fn refresh_source(
    id: &str,
    item_id: &str,
    revision: i64,
) -> Result<SermonDetail, ApiError> {
    patch_item(id, item_id, revision, None, true).await
}

async fn patch_item(
    id: &str,
    item_id: &str,
    revision: i64,
    translation: Option<&str>,
    refresh_source: bool,
) -> Result<SermonDetail, ApiError> {
    patch_json_refusal(
        &format!("/bible/sermons/{id}/items/{item_id}"),
        &ItemPatch {
            expected_revision: revision,
            translation,
            refresh_source,
        },
    )
    .await
}

pub async fn delete_item(id: &str, item_id: &str, revision: i64) -> Result<SermonDetail, ApiError> {
    delete_json_refusal(&format!(
        "/bible/sermons/{id}/items/{item_id}?expectedRevision={revision}"
    ))
    .await
}

/// `GET /bible/study/identity`: the Access e-mail, `None` on the LAN.
pub async fn identity() -> Result<Option<String>, ApiError> {
    #[derive(Deserialize)]
    struct Identity {
        email: Option<String>,
    }
    let answer: Identity = get_json("/bible/study/identity").await?;
    Ok(answer.email)
}

/// Did the write lose to another device?
pub fn is_conflict(err: &ApiError) -> bool {
    matches!(err, ApiError::Refused(409, _) | ApiError::Status(409, _))
}

/// The server read this edit and will never take it: no retry can store it.
pub fn is_refused_edit(err: &ApiError) -> bool {
    matches!(err, ApiError::Refused(422, _) | ApiError::Status(422, _))
}

pub fn is_not_found(err: &ApiError) -> bool {
    matches!(err, ApiError::Refused(404, _) | ApiError::Status(404, _))
}
