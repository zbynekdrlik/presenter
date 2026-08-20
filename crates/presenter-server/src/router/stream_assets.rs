//! Stream-graphics asset HTTP surface (#708, epic #718 §5) — multipart upload
//! deduped by sha256, content-addressed serving with immutable cache headers,
//! and guarded delete (409 while an `image` element references the asset).
//!
//! The on-disk byte layer is [`crate::state::stream_assets::AssetStore`]; the
//! `stream_assets` metadata row is the #705 repository. This module wires the
//! two together behind the reserved `/stream/assets*` routes.
//!
//! Routes (mounted via one `.merge(routes())` in `router.rs::build_router`):
//! - `POST   /stream/assets`      multipart upload → `StreamAsset` JSON
//! - `GET    /stream/assets/{id}` serve bytes (immutable cache)
//! - `DELETE /stream/assets/{id}` delete row + file (409 while referenced)
//! - `GET    /stream/api/assets`  list metadata

use super::AppError;
use crate::state::stream_assets::{
    detect_image, ext_for_mime, image_dims, sha256_hex, MAX_UPLOAD_BYTES,
};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use presenter_core::stream::StreamAsset;
use presenter_persistence::NewStreamAsset;
use tracing::instrument;

/// Raw-body ceiling on the upload route — a DoS guard set generously above the
/// [`MAX_UPLOAD_BYTES`] business cap so a valid 20 MiB image (plus multipart
/// boundary overhead) reaches the handler and gets the precise `413` there,
/// while a wildly oversized body is still cut off by axum first.
const UPLOAD_BODY_LIMIT_BYTES: usize = MAX_UPLOAD_BYTES + 4 * 1024 * 1024;

/// The reserved `/stream/assets*` routes, merged into `build_router`.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/stream/assets",
            post(upload_asset).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT_BYTES)),
        )
        .route("/stream/assets/{id}", get(serve_asset).delete(delete_asset))
        .route("/stream/api/assets", get(list_assets))
}

/// `POST /stream/assets` — multipart image upload. Validates by MAGIC BYTES
/// (never the declared content-type), caps at 20 MiB (`413`), hashes, writes the
/// bytes content-addressed to disk (dedup), records/reuses the metadata row, and
/// returns the `StreamAsset`.
#[instrument(skip_all)]
async fn upload_asset(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<StreamAsset>, AppError> {
    let mut uploaded: Option<(Option<String>, bytes::Bytes)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e))?
    {
        if field.name() == Some("file") {
            let filename = field.file_name().map(str::to_string);
            let data = field.bytes().await.map_err(|e| AppError::bad_request(e))?;
            uploaded = Some((filename, data));
            break;
        }
    }

    let (filename, data) =
        uploaded.ok_or_else(|| AppError::bad_request_message("missing file field"))?;

    if data.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            anyhow::anyhow!(
                "uploaded image is {} bytes, over the {} MiB limit",
                data.len(),
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
        ));
    }

    let detected = detect_image(&data).ok_or_else(|| {
        AppError::unprocessable("unsupported image type (only PNG, JPEG and WebP are accepted)")
    })?;

    let sha256 = sha256_hex(&data);
    let (width, height) = match image_dims(&data) {
        Some((w, h)) => (i32::try_from(w).ok(), i32::try_from(h).ok()),
        None => (None, None),
    };

    // Write the bytes FIRST (idempotent on identical content), then the row —
    // so a metadata row always has its file. An orphan file from a failed row
    // insert is harmless and re-used by the next identical upload.
    state
        .asset_store()
        .store(&sha256, detected.ext, &data)
        .await
        .map_err(|e| AppError::internal(format!("failed to store asset bytes: {e}")))?;

    let original_filename = filename
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| format!("upload.{}", detected.ext));

    let asset = state
        .repository()
        .insert_or_get_stream_asset(NewStreamAsset {
            sha256,
            original_filename,
            mime: detected.mime.to_string(),
            size_bytes: data.len() as i64,
            width,
            height,
        })
        .await?;

    Ok(Json(asset))
}

/// `GET /stream/assets/{id}` — serve the stored image bytes with a
/// forever-immutable cache header. The URL is id-addressed, but the id→bytes
/// binding IS immutable: `stream_assets.id` is AUTOINCREMENT (never reused after
/// delete), there is no sha256-update path, and dedup keeps one row per content
/// — so a cached response can only ever go 200→404, never point at different
/// bytes. `404` for a missing row OR a missing file.
#[instrument(skip_all)]
async fn serve_asset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Missing row → RepositoryError::NotFound → 404 (central mapping).
    let asset = state.repository().get_stream_asset(id).await?;
    let ext = ext_for_mime(&asset.mime)
        .ok_or_else(|| AppError::not_found("asset has an unknown media type"))?;

    let bytes = state
        .asset_store()
        .read(&asset.sha256, ext)
        .await
        .map_err(|e| AppError::internal(format!("failed to read asset bytes: {e}")))?
        .ok_or_else(|| AppError::not_found("asset file not found"))?;

    let content_type = HeaderValue::from_str(&asset.mime)
        .unwrap_or_else(|_| HeaderValue::from_static("image/png"));
    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    // Defense-in-depth for a route that streams user-uploaded bytes: even though
    // the content-type is magic-byte-derived and restricted to image/*, tell the
    // browser never to MIME-sniff the body.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `DELETE /stream/assets/{id}` — delete the row (repository guard ⇒ `409` with
/// referencing scene names while any `image` element points at it) then the
/// file. `404` for a missing row.
#[instrument(skip_all)]
async fn delete_asset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    // Look up first so we can remove the file after the row is gone (404 here
    // if the row is already missing).
    let asset = state.repository().get_stream_asset(id).await?;
    // Guarded delete — ConflictDetail → 409 (central mapping) while referenced;
    // deletes the row when unreferenced.
    state.repository().delete_stream_asset(id).await?;

    // Row is gone; remove the file best-effort. A surviving orphan file is
    // harmless (re-uploadable, dedup re-uses it) and must not fail the request.
    if let Some(ext) = ext_for_mime(&asset.mime) {
        if let Err(e) = state.asset_store().remove(&asset.sha256, ext).await {
            tracing::warn!(
                asset_id = id,
                sha256 = %asset.sha256,
                error = %e,
                "asset row deleted but its file could not be removed"
            );
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /stream/api/assets` — all asset metadata, newest first.
#[instrument(skip_all)]
async fn list_assets(State(state): State<AppState>) -> Result<Json<Vec<StreamAsset>>, AppError> {
    Ok(Json(state.repository().list_stream_assets().await?))
}

#[cfg(test)]
mod tests;
