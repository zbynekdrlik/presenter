//! Stream-graphics font HTTP surface (#778, epic #718) — multipart upload
//! deduped by sha256, `ttf-parser` metadata extraction, content-addressed
//! serving with immutable cache headers, guarded delete (409 while the last
//! face of a family is referenced), and a generated `@font-face` stylesheet.
//!
//! The on-disk byte layer is [`crate::state::stream_fonts::FontStore`]; the
//! `stream_fonts` metadata row is the repository. Mirrors `router/stream_assets.rs`
//! conventions (body limit, 413, nosniff, immutable cache).
//!
//! Routes (mounted via one `.merge(routes())` in `router.rs::build_router`):
//! - `POST   /stream/fonts`      multipart upload (field `file`) → `StreamFont`
//! - `GET    /stream/api/fonts`  list metadata
//! - `GET    /stream/fonts/{id}` serve font bytes (immutable cache)
//! - `DELETE /stream/fonts/{id}` delete row + file (409 while referenced)
//! - `GET    /stream/fonts.css`  generated `@font-face` rules (no-cache + ETag)

use super::AppError;
use crate::state::stream_assets::sha256_hex;
use crate::state::stream_fonts::{detect_font, parse_font_metadata, MAX_FONT_BYTES};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use presenter_core::stream::StreamFont;
use presenter_persistence::NewStreamFont;
use tracing::instrument;

/// Raw-body ceiling on the upload route — a DoS guard generously above the
/// [`MAX_FONT_BYTES`] business cap so a valid font (plus multipart overhead)
/// reaches the handler and gets the precise `413` there.
const UPLOAD_BODY_LIMIT_BYTES: usize = MAX_FONT_BYTES + 2 * 1024 * 1024;

/// The reserved `/stream/fonts*` routes, merged into `build_router`.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/stream/fonts",
            post(upload_font).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT_BYTES)),
        )
        .route("/stream/fonts.css", get(fonts_css))
        .route("/stream/fonts/{id}", get(serve_font).delete(delete_font))
        .route("/stream/api/fonts", get(list_fonts))
}

/// Characters that would break out of the double-quoted CSS `font-family` value
/// or a `@font-face` block. A family name carrying any of these — or an ASCII
/// control char — is rejected at upload (`css_font_family` in the UI escapes
/// too, defense in depth).
fn family_name_is_safe(family: &str) -> bool {
    !family.is_empty()
        && !family
            .chars()
            .any(|c| matches!(c, '"' | '\\' | ';' | '{' | '}' | '<' | '>') || c.is_control())
}

/// `POST /stream/fonts` — multipart web-font upload (field `file`). Validates by
/// MAGIC BYTES (ttf/otf only; `.ttc`/`woff`/`woff2`/garbage → 422), caps at
/// [`MAX_FONT_BYTES`] (413), parses family/weight/italic, writes the bytes
/// content-addressed (dedup), records/reuses the metadata row, returns the
/// `StreamFont`.
#[instrument(skip_all)]
async fn upload_font(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<StreamFont>, AppError> {
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

    if data.len() > MAX_FONT_BYTES {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            anyhow::anyhow!(
                "uploaded font is {} bytes, over the {} MiB limit",
                data.len(),
                MAX_FONT_BYTES / (1024 * 1024)
            ),
        ));
    }

    let detected = detect_font(&data).ok_or_else(|| {
        AppError::unprocessable(
            "unsupported font type (only raw TTF and OTF are accepted; \
             .ttc / woff / woff2 are not)",
        )
    })?;

    let meta = parse_font_metadata(&data)
        .map_err(|e| AppError::unprocessable(format!("could not read font metadata: {e}")))?;

    if !family_name_is_safe(&meta.family) {
        return Err(AppError::unprocessable(format!(
            "font family name {:?} contains characters that are not allowed",
            meta.family
        )));
    }

    let sha256 = sha256_hex(&data);

    // Write the bytes FIRST (idempotent on identical content), then the row.
    state
        .font_store()
        .store(&sha256, detected.ext, &data)
        .await
        .map_err(|e| AppError::internal(format!("failed to store font bytes: {e}")))?;

    let original_filename = filename
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| format!("upload.{}", detected.ext));

    let font = state
        .repository()
        .insert_or_get_stream_font(NewStreamFont {
            sha256,
            original_filename,
            family: meta.family,
            weight: meta.weight,
            italic: meta.italic,
            format: detected.ext.to_string(),
            size_bytes: data.len() as i64,
        })
        .await?;

    Ok(Json(font))
}

/// `GET /stream/api/fonts` — all font metadata, newest first.
#[instrument(skip_all)]
async fn list_fonts(State(state): State<AppState>) -> Result<Json<Vec<StreamFont>>, AppError> {
    Ok(Json(state.repository().list_stream_fonts().await?))
}

/// `GET /stream/fonts/{id}` — serve the stored font bytes with a
/// forever-immutable cache header (the id→bytes binding is immutable: id is
/// AUTOINCREMENT, dedup keeps one row per content, no sha-update path).
#[instrument(skip_all)]
async fn serve_font(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let font = state.repository().get_stream_font(id).await?;
    let bytes = state
        .font_store()
        .read(&font.sha256, &font.format)
        .await
        .map_err(|e| AppError::internal(format!("failed to read font bytes: {e}")))?
        .ok_or_else(|| AppError::not_found("font file not found"))?;

    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime_for(&font.format)),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `DELETE /stream/fonts/{id}` — delete the row (repository guard ⇒ 409 with the
/// referencing scene names while it is the last face of an in-use family) then
/// the file. 404 for a missing row.
#[instrument(skip_all)]
async fn delete_font(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let font = state.repository().get_stream_font(id).await?;
    state.repository().delete_stream_font(id).await?;

    if let Err(e) = state.font_store().remove(&font.sha256, &font.format).await {
        tracing::warn!(
            font_id = id,
            sha256 = %font.sha256,
            error = %e,
            "font row deleted but its file could not be removed"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /stream/fonts.css` — the generated `@font-face` rules for every uploaded
/// face. `Cache-Control: no-cache` + an ETag over the face set so the OBS output
/// and editor pages always revalidate but transfer nothing when unchanged (a
/// fresh upload changes the set → a new ETag → a re-fetch).
#[instrument(skip_all)]
async fn fonts_css(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let fonts = state.repository().list_stream_fonts().await?;
    let css = build_fonts_css(&fonts);
    let etag = format!("\"{}\"", &sha256_hex(css.as_bytes())[..32]);

    // Conditional revalidation: 304 when the client already holds this face set.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm.split(',').any(|t| t.trim() == etag) {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            set_css_cache_headers(resp.headers_mut(), &etag);
            return Ok(resp);
        }
    }

    let mut resp = (StatusCode::OK, css).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    set_css_cache_headers(h, &etag);
    Ok(resp)
}

fn set_css_cache_headers(h: &mut HeaderMap, etag: &str) {
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(v) = HeaderValue::from_str(etag) {
        h.insert(header::ETAG, v);
    }
}

/// Build the `@font-face` stylesheet body from the face rows. One rule per face;
/// each `src` points at the id-addressed serve route with a `format()` hint.
fn build_fonts_css(fonts: &[StreamFont]) -> String {
    let mut css = String::from("/* generated stream web-font faces (#778) */\n");
    for f in fonts {
        // Family names are upload-validated to exclude `" \ ; { } < >` + control
        // chars; escape a quote/backslash anyway (defense in depth).
        let family = f.family.replace('\\', "\\\\").replace('"', "\\\"");
        let style = if f.italic { "italic" } else { "normal" };
        let format_hint = css_format_hint(&f.format);
        css.push_str(&format!(
            "@font-face {{\n  font-family: \"{family}\";\n  font-style: {style};\n  \
             font-weight: {weight};\n  font-display: block;\n  \
             src: url(\"/stream/fonts/{id}\") format(\"{format_hint}\");\n}}\n",
            weight = f.weight,
            id = f.id,
        ));
    }
    css
}

/// Served content-type for a stored font format.
fn mime_for(format: &str) -> &'static str {
    match format {
        "otf" => "font/otf",
        _ => "font/ttf",
    }
}

/// CSS `format()` hint for a stored font format.
fn css_format_hint(format: &str) -> &'static str {
    match format {
        "otf" => "opentype",
        _ => "truetype",
    }
}

#[cfg(test)]
mod tests;
