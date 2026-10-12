//! Keyless Google Drive client for the stream PLAYBACK element (#843).
//!
//! The operator shares a Drive folder as "anyone with the link" and pastes the
//! link; nothing else is set up (no Cloud project, no API key — ROZHODNUTÉ on
//! #843). Two public, keyless endpoints are used, both live-tested on #843:
//!
//! - **Listing**: `GET {list}/embeddedfolderview?id=<FID>[&resourcekey=<RK>]`,
//!   server-rendered HTML parsed by [`listing::parse_listing`]. 401 = the folder
//!   is not public or the resource key is missing, 404 = unknown id.
//! - **Download**: `GET|HEAD {download}/download?id=<ID>&export=download&confirm=t
//!   [&resourcekey=<RK>]` → 200 with `content-length`, `last-modified`,
//!   `accept-ranges`. `confirm=t` skips the virus-scan page of big files; the old
//!   `uc?export=download` URL is never used (its redirect drops `confirm=t`). A
//!   `text/html` answer is an error page and is never cached.
//!
//! This module is the ONLY place that talks to Drive, so an API-key path
//! (Drive API v3) can replace it if Google changes the keyless endpoints.
//! Base URLs come from `PRESENTER_DRIVE_URL` / `PRESENTER_DRIVE_DOWNLOAD_URL`
//! (tests point them at a mock; the E2E default is a dead loopback).

mod listing;
#[cfg(test)]
mod tests;

pub(crate) use listing::{is_listing_page, parse_listing, DriveEntry};

use std::path::Path;
use std::time::Duration;

use presenter_core::DriveFolder;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LAST_MODIFIED};
use tokio::io::AsyncWriteExt;

/// Drive's web host (the folder listing).
const DEFAULT_LIST_URL: &str = "https://drive.google.com";
/// Drive's download host.
const DEFAULT_DOWNLOAD_URL: &str = "https://drive.usercontent.google.com";
/// Bound on a listing or HEAD request, connect to last byte.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Bound on opening a connection (downloads have no total timeout).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Any request that receives nothing for this long — no response headers, or
/// no body byte — is abandoned (reqwest's `read_timeout` covers both the wait
/// for the headers and each body read), so a peer that never answers cannot
/// wedge the single sync worker.
const CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest file the mirror stores (2 GiB) — a guard for the venue PC's disk.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Where and how the client talks to Drive.
pub(crate) struct DriveConfig {
    pub(crate) list_base_url: String,
    pub(crate) download_base_url: String,
    pub(crate) request_timeout: Duration,
    pub(crate) chunk_idle_timeout: Duration,
    pub(crate) max_file_bytes: u64,
}

impl DriveConfig {
    /// `PRESENTER_DRIVE_URL` (default `https://drive.google.com`) and
    /// `PRESENTER_DRIVE_DOWNLOAD_URL` (default `https://drive.usercontent.google.com`).
    fn from_env() -> Self {
        Self {
            list_base_url: env_or("PRESENTER_DRIVE_URL", DEFAULT_LIST_URL),
            download_base_url: env_or("PRESENTER_DRIVE_DOWNLOAD_URL", DEFAULT_DOWNLOAD_URL),
            request_timeout: REQUEST_TIMEOUT,
            chunk_idle_timeout: CHUNK_IDLE_TIMEOUT,
            max_file_bytes: MAX_FILE_BYTES,
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// A failed Drive request. The text is Slovak: it reaches the operator as the
/// editor's playback status line (`PlaybackPlaylist.error`).
#[derive(Debug, thiserror::Error)]
pub(crate) enum DriveError {
    /// 401/403: not shared as "anyone with the link", or the link lacks the
    /// folder's `resourcekey`.
    #[error("priečinok nie je verejný — zdieľaj ho ako „ktokoľvek s odkazom“ (HTTP {status})")]
    NotPublic { status: u16 },
    #[error("priečinok na Google Drive neexistuje (HTTP 404)")]
    FolderNotFound,
    #[error("súbor na Google Drive neexistuje (HTTP 404)")]
    FileNotFound,
    #[error("Google Drive vrátil HTTP {status}")]
    Status { status: u16 },
    /// A 200 page that is not a folder listing (a sign-in page, a format
    /// change) — never treated as "the folder is empty", which would prune
    /// the whole mirror.
    #[error("Google Drive vrátil neočakávanú stránku namiesto zoznamu súborov")]
    UnexpectedListing,
    /// A `text/html` answer where a file was expected (an error or virus-scan
    /// page).
    #[error("Google Drive vrátil chybovú stránku namiesto súboru")]
    HtmlPage,
    #[error("Google Drive neodpovedal do {secs} s")]
    Timeout { secs: u64 },
    #[error("Google Drive nedostupný: {detail}")]
    Network { detail: String },
    #[error("súbor je väčší ako {max_mib} MiB")]
    TooLarge { max_mib: u64 },
    /// The mirror-wide byte budget would be exceeded (all folders together).
    #[error("úložisko prehrávania je plné (limit {limit_mib} MiB) — zmenši priečinok alebo zvýš PRESENTER_PLAYBACK_MAX_GB")]
    BudgetExceeded { limit_mib: u64 },
    #[error("sťahovanie skončilo predčasne ({got} z {expected} B)")]
    Truncated { got: u64, expected: u64 },
    #[error("zápis na disk zlyhal: {detail}")]
    Io { detail: String },
}

impl DriveError {
    fn io(err: &std::io::Error) -> Self {
        Self::Io {
            detail: err.to_string(),
        }
    }
}

/// What a HEAD (or a finished download) tells about a remote file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RemoteFile {
    /// `content-length` (for a download: the bytes actually written).
    pub(crate) size: Option<u64>,
    /// The raw `last-modified` header.
    pub(crate) last_modified: Option<String>,
}

/// One shared client per `AppState` (held by the playback sync).
pub(crate) struct DriveClient {
    http: reqwest::Client,
    config: DriveConfig,
}

impl DriveClient {
    pub(crate) fn from_env() -> Self {
        let config = DriveConfig::from_env();
        tracing::info!(
            list_url = %config.list_base_url,
            download_url = %config.download_base_url,
            "playback: Google Drive client configured (keyless)"
        );
        Self::new(config)
    }

    pub(crate) fn new(config: DriveConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(config.chunk_idle_timeout)
            .user_agent(concat!("presenter/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "playback: custom HTTP client failed, using the default");
                reqwest::Client::new()
            });
        Self { http, config }
    }

    /// A client for a mock Drive (listing and download on the same base).
    #[cfg(test)]
    pub(crate) fn for_test(base_url: &str) -> Self {
        Self::new(DriveConfig {
            list_base_url: base_url.to_string(),
            download_base_url: base_url.to_string(),
            request_timeout: Duration::from_secs(5),
            chunk_idle_timeout: Duration::from_secs(5),
            max_file_bytes: MAX_FILE_BYTES,
        })
    }

    /// Same as [`Self::for_test`] with a custom file-size cap.
    #[cfg(test)]
    pub(crate) fn for_test_with_max_bytes(base_url: &str, max_file_bytes: u64) -> Self {
        let mut client = Self::for_test(base_url);
        client.config.max_file_bytes = max_file_bytes;
        client
    }

    /// Same as [`Self::for_test`] with a custom idle (read) timeout.
    #[cfg(test)]
    pub(crate) fn for_test_with_idle_timeout(base_url: &str, idle: Duration) -> Self {
        Self::new(DriveConfig {
            list_base_url: base_url.to_string(),
            download_base_url: base_url.to_string(),
            request_timeout: Duration::from_secs(30),
            chunk_idle_timeout: idle,
            max_file_bytes: MAX_FILE_BYTES,
        })
    }

    /// The largest single file this client downloads.
    pub(crate) fn max_file_bytes(&self) -> u64 {
        self.config.max_file_bytes
    }

    /// The folder's entries, in Drive's listing order.
    pub(crate) async fn list_folder(
        &self,
        folder: &DriveFolder,
    ) -> Result<Vec<DriveEntry>, DriveError> {
        let url = format!(
            "{}/embeddedfolderview",
            self.config.list_base_url.trim_end_matches('/')
        );
        let mut query = vec![("id", folder.id.as_str())];
        if let Some(key) = folder.resource_key.as_deref() {
            query.push(("resourcekey", key));
        }
        let bound = self.config.request_timeout;
        let response = self
            .http
            .get(url)
            .query(&query)
            .timeout(bound)
            .send()
            .await
            .map_err(|err| self.transport_error(err, bound))?;
        check_status(response.status().as_u16(), DriveError::FolderNotFound)?;
        let page = response
            .text()
            .await
            .map_err(|err| self.transport_error(err, bound))?;
        if !is_listing_page(&page) {
            return Err(DriveError::UnexpectedListing);
        }
        Ok(parse_listing(&page))
    }

    /// Size + last-modified of a file, without downloading it.
    pub(crate) async fn head_file(
        &self,
        file_id: &str,
        resource_key: Option<&str>,
    ) -> Result<RemoteFile, DriveError> {
        let bound = self.config.request_timeout;
        let response = self
            .http
            .head(self.download_url())
            .query(&download_query(file_id, resource_key))
            .timeout(bound)
            .send()
            .await
            .map_err(|err| self.transport_error(err, bound))?;
        check_status(response.status().as_u16(), DriveError::FileNotFound)?;
        let headers = response.headers();
        if is_html(headers) {
            return Err(DriveError::HtmlPage);
        }
        Ok(remote_file(headers))
    }

    /// Stream a file into `dest` (created/truncated), refusing more than
    /// `max_bytes` (further capped by the client's own per-file limit) — before
    /// the first byte when the `content-length` already says so, otherwise the
    /// moment the body passes it. On error `dest` may hold a partial file — the
    /// caller removes it. Returns the written size and the download's
    /// `last-modified`. There is no total timeout (big videos); the client's
    /// read timeout ends a download that stalls, before or after the headers.
    pub(crate) async fn download_file(
        &self,
        file_id: &str,
        resource_key: Option<&str>,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<RemoteFile, DriveError> {
        let idle = self.config.chunk_idle_timeout;
        let mut response = self
            .http
            .get(self.download_url())
            .query(&download_query(file_id, resource_key))
            .send()
            .await
            .map_err(|err| self.transport_error(err, idle))?;
        check_status(response.status().as_u16(), DriveError::FileNotFound)?;
        if is_html(response.headers()) {
            return Err(DriveError::HtmlPage);
        }
        let declared = remote_file(response.headers());
        let max = max_bytes.min(self.config.max_file_bytes);
        if declared.size.is_some_and(|size| size > max) {
            return Err(too_large(max));
        }
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|err| DriveError::io(&err))?;
        let mut written: u64 = 0;
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|err| self.transport_error(err, idle))?;
            let Some(chunk) = chunk else { break };
            written = written.saturating_add(chunk.len() as u64);
            if written > max {
                return Err(too_large(max));
            }
            file.write_all(&chunk)
                .await
                .map_err(|err| DriveError::io(&err))?;
        }
        file.flush().await.map_err(|err| DriveError::io(&err))?;
        file.sync_all().await.map_err(|err| DriveError::io(&err))?;
        if let Some(expected) = declared.size {
            if written != expected {
                return Err(DriveError::Truncated {
                    got: written,
                    expected,
                });
            }
        }
        Ok(RemoteFile {
            size: Some(written),
            last_modified: declared.last_modified,
        })
    }

    fn download_url(&self) -> String {
        format!(
            "{}/download",
            self.config.download_base_url.trim_end_matches('/')
        )
    }

    /// A transport failure; a timeout names the bound that actually fired:
    /// the connect timeout for a connect failure, else the operation's `bound`
    /// (the listing/HEAD total, or the download's idle timeout).
    fn transport_error(&self, err: reqwest::Error, bound: Duration) -> DriveError {
        if err.is_timeout() {
            let fired = if err.is_connect() {
                CONNECT_TIMEOUT
            } else {
                bound
            };
            DriveError::Timeout {
                secs: fired.as_secs(),
            }
        } else {
            DriveError::Network {
                detail: describe(err),
            }
        }
    }
}

fn too_large(max_bytes: u64) -> DriveError {
    DriveError::TooLarge {
        max_mib: max_bytes / (1024 * 1024),
    }
}

/// The download endpoint's query for one file.
fn download_query<'a>(file_id: &'a str, resource_key: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut query = vec![("id", file_id), ("export", "download"), ("confirm", "t")];
    if let Some(key) = resource_key {
        query.push(("resourcekey", key));
    }
    query
}

/// Map a non-success status: 401/403 → not public, 404 → `not_found`.
fn check_status(status: u16, not_found: DriveError) -> Result<(), DriveError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(DriveError::NotPublic { status }),
        404 => Err(not_found),
        status => Err(DriveError::Status { status }),
    }
}

/// `true` when the answer is an HTML page (Drive's error / virus-scan page).
fn is_html(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().to_ascii_lowercase().starts_with("text/html"))
}

/// Size + last-modified from the headers (read directly: for a HEAD the body
/// size hint is 0, so `Response::content_length` would lie).
fn remote_file(headers: &reqwest::header::HeaderMap) -> RemoteFile {
    let header = |name: reqwest::header::HeaderName| {
        headers
            .get(name)
            .and_then(|value: &reqwest::header::HeaderValue| value.to_str().ok())
            .map(|value| value.trim().to_string())
    };
    RemoteFile {
        size: header(CONTENT_LENGTH).and_then(|value| value.parse::<u64>().ok()),
        last_modified: header(LAST_MODIFIED).filter(|value| !value.is_empty()),
    }
}

/// `err` without its URL, plus its causes.
fn describe(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut text = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = std::error::Error::source(cause);
    }
    text
}
