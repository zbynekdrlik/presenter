//! The study page's named bookmarks (#915): the server keeps the list, so the
//! phone and the PC show the same one. A refusal carries the server's Slovak
//! sentence (`ApiError::Refused`).

use serde::{Deserialize, Serialize};

use super::{
    delete_json_refusal, get_json_detail, patch_json_refusal, post_json_refusal, ApiError,
};

const PATH: &str = "/bible/study/bookmarks";

/// One stored bookmark. `book` = the translation's own name of the book.
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
struct UpdateBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<&'a BookmarkPlace>,
}

/// What a delete answers: the bookmark and where it stood, for Späť.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Deleted {
    pub bookmark: Bookmark,
    pub index: usize,
}

pub async fn list() -> Result<Vec<Bookmark>, ApiError> {
    get_json_detail(PATH).await
}

/// A new bookmark at the end of the list, or at `index`.
pub async fn create(
    name: &str,
    place: &BookmarkPlace,
    index: Option<usize>,
) -> Result<Bookmark, ApiError> {
    post_json_refusal(PATH, &CreateBody { name, place, index }).await
}

pub async fn rename(id: i64, name: &str) -> Result<Bookmark, ApiError> {
    let body = UpdateBody {
        name: Some(name),
        place: None,
    };
    patch_json_refusal(&format!("{PATH}/{id}"), &body).await
}

/// "Presunúť sem": the bookmark to this place.
pub async fn move_to(id: i64, place: &BookmarkPlace) -> Result<Bookmark, ApiError> {
    let body = UpdateBody {
        name: None,
        place: Some(place),
    };
    patch_json_refusal(&format!("{PATH}/{id}"), &body).await
}

pub async fn delete(id: i64) -> Result<Deleted, ApiError> {
    delete_json_refusal(&format!("{PATH}/{id}")).await
}
