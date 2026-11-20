//! The study page's bookmarks (#915, #951): the server keeps the list, so the
//! phone and the PC show the same one; it answers newest first, each row with
//! the start of its verse's text. A refusal carries the server's Slovak sentence
//! (`ApiError::Refused`).
//!
//! #952: the bookmarks sit in FOLDERS. Every bookmark call names one (`None` =
//! the default folder "Kde som skončil", sent as no `folder` at all); the
//! reader's own folders are `/bible/study/bookmark-folders`.

use serde::{Deserialize, Serialize};

use super::{get_json_detail, post_json_refusal, put_json_refusal, put_refusal, ApiError};

const PATH: &str = "/bible/study/bookmarks";
const FOLDERS: &str = "/bible/study/bookmark-folders";

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

/// One of the reader's own folders; `count` = its bookmarks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub count: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    folder: Option<i64>,
}

#[derive(Serialize)]
struct Ids<'a> {
    ids: &'a [i64],
    #[serde(skip_serializing_if = "Option::is_none")]
    folder: Option<i64>,
}

#[derive(Serialize)]
struct Name<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct Id {
    id: i64,
}

/// A deleted bookmark and where it stood (top-first), for Späť.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Deleted {
    pub bookmark: Bookmark,
    pub index: usize,
}

/// The list's path: `?folder=<id>` for one of the reader's folders.
pub fn list_path(folder: Option<i64>) -> String {
    match folder {
        Some(id) => format!("{PATH}?folder={id}"),
        None => PATH.to_string(),
    }
}

pub async fn list(folder: Option<i64>) -> Result<Vec<Bookmark>, ApiError> {
    get_json_detail(&list_path(folder)).await
}

/// A new bookmark on top of the folder's list, or at `index` (top-first).
pub async fn create(
    folder: Option<i64>,
    name: &str,
    place: &BookmarkPlace,
    index: Option<usize>,
) -> Result<Bookmark, ApiError> {
    let body = CreateBody {
        name,
        place,
        index,
        folder,
    };
    post_json_refusal(PATH, &body).await
}

/// The folder's whole list in a new order, top-first (a stale set is a 409
/// refusal).
pub async fn reorder(folder: Option<i64>, ids: &[i64]) -> Result<(), ApiError> {
    put_refusal(&format!("{PATH}/order"), &Ids { ids, folder }).await
}

/// The chosen ones at once; answered with their old indices, ascending.
pub async fn delete_many(folder: Option<i64>, ids: &[i64]) -> Result<Vec<Deleted>, ApiError> {
    post_json_refusal(&format!("{PATH}/delete"), &Ids { ids, folder }).await
}

/// The reader's own folders, in their order.
pub async fn folders() -> Result<Vec<Folder>, ApiError> {
    get_json_detail(FOLDERS).await
}

/// A new folder at the end of the list.
pub async fn create_folder(name: &str) -> Result<Folder, ApiError> {
    post_json_refusal(FOLDERS, &Name { name }).await
}

pub async fn rename_folder(id: i64, name: &str) -> Result<Folder, ApiError> {
    put_json_refusal(&format!("{FOLDERS}/{id}"), &Name { name }).await
}

/// The folder and every bookmark in it; answered with the folder as it was.
pub async fn delete_folder(id: i64) -> Result<Folder, ApiError> {
    post_json_refusal(&format!("{FOLDERS}/delete"), &Id { id }).await
}
