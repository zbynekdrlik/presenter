//! PLAYBACK element HTTP surface (#843) — the Drive mirror's playlist, the
//! „Obnoviť" trigger and the cached files.
//!
//! Routes (mounted with one `.merge(routes())` in `router.rs::build_router`):
//! - `GET  /stream/api/playback?url=<link>` → `PlaybackPlaylist` (422 for a
//!   link that is not a Drive folder);
//! - `POST /stream/api/playback/refresh` `{url?}` (or a bare POST) → 202, a
//!   sync pass is queued;
//! - `GET  /stream/playback/{folder_id}/{file_id}` → the cached file, with a
//!   single `Range: bytes=…` honoured (206). `tower-http`'s `fs` feature is not
//!   enabled here, and a moov-at-end MP4 needs ranges to play in Chromium/OBS.
//!
//! Files are id-addressed: both path segments must be Drive ids (`[-\w]+`)
//! and the file must be listed in the folder's manifest, which supplies the
//! extension and the `Content-Type` — client text never reaches a path join.
//! `playback` is a reserved output slug, so `/stream/playback/…` never
//! shadows an output page.

use std::io::SeekFrom;
use std::path::Path;

use axum::{
    body::Body,
    extract::{Path as UrlPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::Stream;
use presenter_core::{
    is_valid_drive_file_id, is_valid_drive_folder_id, parse_drive_folder_url, DriveFolder,
    PlaybackPlaylist,
};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tracing::instrument;

use super::AppError;
use crate::state::AppState;

/// Read size of one streamed body chunk.
const CHUNK_BYTES: usize = 64 * 1024;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/stream/api/playback", get(get_playlist))
        .route("/stream/api/playback/refresh", post(refresh))
        .route("/stream/playback/{folder_id}/{file_id}", get(serve_file))
}

#[derive(Debug, Deserialize)]
pub(super) struct PlaylistQuery {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RefreshRequest {
    #[serde(default)]
    url: Option<String>,
}

/// A pasted link → its folder, or a 422 naming why it is not one.
fn folder_of(url: &str) -> Result<DriveFolder, AppError> {
    parse_drive_folder_url(url).map_err(|err| {
        AppError::unprocessable(format!("neplatný odkaz na priečinok Google Drive: {err}"))
    })
}

/// `GET /stream/api/playback?url=<link>` — the folder's cached playlist.
#[instrument(skip_all)]
async fn get_playlist(
    State(state): State<AppState>,
    Query(query): Query<PlaylistQuery>,
) -> Result<Json<PlaybackPlaylist>, AppError> {
    let folder = folder_of(&query.url)?;
    Ok(Json(state.playback_playlist(&folder).await))
}

/// `POST /stream/api/playback/refresh` — queue a full sync pass now (plus the
/// given link's folder, even if no saved element names it yet).
#[instrument(skip_all)]
async fn refresh(
    State(state): State<AppState>,
    request: Option<Json<RefreshRequest>>,
) -> Result<StatusCode, AppError> {
    // A bare POST (no Content-Type, no body) refreshes every referenced folder.
    let request = request.map(|Json(request)| request).unwrap_or_default();
    let extra = match request.url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => Some(folder_of(url)?),
        _ => None,
    };
    state.request_playback_refresh(extra).await;
    Ok(StatusCode::ACCEPTED)
}

/// `GET /stream/playback/{folder_id}/{file_id}` — a cached file (Range aware).
#[instrument(skip_all)]
async fn serve_file(
    State(state): State<AppState>,
    UrlPath((folder_id, file_id)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !is_valid_drive_folder_id(&folder_id) || !is_valid_drive_file_id(&file_id) {
        return Err(AppError::not_found("playback file not found"));
    }
    let (path, mime) = state
        .playback_file(&folder_id, &file_id)
        .await
        .ok_or_else(|| AppError::not_found("playback file not found"))?;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    file_response(&path, &mime, range).await
}

/// The response for `path`: 200 (no / ignored range), 206 (one range) or 416.
async fn file_response(path: &Path, mime: &str, range: Option<&str>) -> Result<Response, AppError> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::not_found("playback file not found"));
        }
        Err(err) => {
            return Err(AppError::internal(format!(
                "playback file unreadable: {err}"
            )))
        }
    };
    let len = file
        .metadata()
        .await
        .map_err(|err| AppError::internal(format!("playback file unreadable: {err}")))?
        .len();
    let (status, start, count) = match parse_range(range, len) {
        ByteRange::Full => (StatusCode::OK, 0, len),
        ByteRange::Partial { start, end } => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
        ByteRange::Unsatisfiable => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            insert(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes */{len}"),
            );
            insert(&mut response, header::ACCEPT_RANGES, "bytes");
            return Ok(response);
        }
    };
    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|err| AppError::internal(format!("playback file seek failed: {err}")))?;
    }
    let mut response = (status, Body::from_stream(file_chunks(file, count))).into_response();
    insert(&mut response, header::CONTENT_TYPE, mime);
    insert(&mut response, header::CONTENT_LENGTH, &count.to_string());
    insert(&mut response, header::ACCEPT_RANGES, "bytes");
    // The playlist URL carries `?v=<size+date token>`, so a replaced file gets
    // a new URL; an hour of caching saves re-reads on every loop.
    insert(&mut response, header::CACHE_CONTROL, "public, max-age=3600");
    insert(&mut response, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if status == StatusCode::PARTIAL_CONTENT {
        let end = start + count - 1;
        insert(
            &mut response,
            header::CONTENT_RANGE,
            &format!("bytes {start}-{end}/{len}"),
        );
    }
    Ok(response)
}

fn insert(response: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// `count` bytes of `file` from its current position, as a body stream.
fn file_chunks(
    mut file: tokio::fs::File,
    count: u64,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    async_stream::try_stream! {
        let mut remaining = count;
        let mut buffer = vec![0u8; CHUNK_BYTES];
        while remaining > 0 {
            let want = usize::try_from(remaining).unwrap_or(CHUNK_BYTES).min(CHUNK_BYTES);
            let read = file.read(&mut buffer[..want]).await?;
            if read == 0 {
                break;
            }
            remaining -= read as u64;
            yield Bytes::copy_from_slice(&buffer[..read]);
        }
    }
}

/// A parsed `Range` header for a file of a known length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ByteRange {
    /// No range, or one this handler ignores (multi-range, another unit, a
    /// malformed or reversed spec — RFC 9110 lets a server ignore those).
    Full,
    /// One satisfiable range, `end` inclusive and clamped to the file.
    Partial { start: u64, end: u64 },
    /// A range that starts past the end (or an empty suffix) → 416.
    Unsatisfiable,
}

/// Parse a single `bytes=a-b` / `bytes=a-` / `bytes=-n` range.
pub(super) fn parse_range(header: Option<&str>, len: u64) -> ByteRange {
    let Some(spec) = header
        .map(str::trim)
        .and_then(|h| h.strip_prefix("bytes="))
        .map(str::trim)
    else {
        return ByteRange::Full;
    };
    if spec.contains(',') {
        return ByteRange::Full;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return ByteRange::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // Suffix range: the last `n` bytes.
        let Ok(suffix) = last.parse::<u64>() else {
            return ByteRange::Full;
        };
        if suffix == 0 || len == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Partial {
            start: len.saturating_sub(suffix),
            end: len - 1,
        };
    }
    let Ok(start) = first.parse::<u64>() else {
        return ByteRange::Full;
    };
    let end = if last.is_empty() {
        u64::MAX
    } else {
        match last.parse::<u64>() {
            Ok(end) if end >= start => end,
            _ => return ByteRange::Full,
        }
    };
    if start >= len {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Partial {
        start,
        end: end.min(len - 1),
    }
}

#[cfg(test)]
mod tests;
