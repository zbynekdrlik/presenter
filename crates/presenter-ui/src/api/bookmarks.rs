//! The study page's bookmarks (#915, #951): the server keeps the list, so the
//! phone and the PC show the same one; it answers newest first, each row with
//! the start of its verse's text. A refusal carries the server's Slovak sentence
//! (`ApiError::Refused`).

use serde::{Deserialize, Serialize};

use super::{get_json_detail, post_json_refusal, put_refusal, ApiError};

const PATH: &str = "/bible/study/bookmarks";

/// One stored bookmark. `book` = the translation's own name of the book; `text`
/// = the start of the verse (the list only).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: i64,
    pub name: String,
    pub translation: String,
    pub book_code: String,
    pub book: String,
    pub chapter: u16,
    pub verse: u16,
    #[serde(default)]
    pub text: Option<String>,
}

/// A verse of a translation, as a write names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkPlace {
    pub translation: String,
    pub book_code: String,
    pub chapter: u16,
    pub verse: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateBody<'a> {
    name: &'a str,
    #[serde(flatten)]
    place: &'a BookmarkPlace,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
}

#[derive(Serialize)]
struct Ids<'a> {
    ids: &'a [i64],
}

/// A deleted bookmark and where it stood (top-first), for Späť.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Deleted {
    pub bookmark: Bookmark,
    pub index: usize,
}

pub async fn list() -> Result<Vec<Bookmark>, ApiError> {
    get_json_detail(PATH).await
}

/// A new bookmark on top of the list, or at `index` (top-first).
pub async fn create(
    name: &str,
    place: &BookmarkPlace,
    index: Option<usize>,
) -> Result<Bookmark, ApiError> {
    post_json_refusal(PATH, &CreateBody { name, place, index }).await
}

/// The whole list in a new order, top-first (a stale set is a 409 refusal).
pub async fn reorder(ids: &[i64]) -> Result<(), ApiError> {
    put_refusal(&format!("{PATH}/order"), &Ids { ids }).await
}

/// The chosen ones at once; answered with their old indices, ascending.
pub async fn delete_many(ids: &[i64]) -> Result<Vec<Deleted>, ApiError> {
    post_json_refusal(&format!("{PATH}/delete"), &Ids { ids }).await
}
