//! The on-disk record of one mirrored Drive folder (#843):
//! `<stream-assets>/playback/<folder_id>/manifest.json` next to the cached
//! files `<file_id>.<ext>`.
//!
//! The manifest is the playlist: the playable items in Drive's listing order,
//! each with the size + last-modified the download HEAD reported (the change
//! detector — the listing has neither), plus when the folder was last synced
//! and the last error. A failed sync keeps the items (the last good playlist
//! keeps playing) and only records the error.

use std::path::Path;

use chrono::{DateTime, Utc};
use presenter_core::{PlaybackItem, PlaybackItemKind, PlaybackPlaylist};
use serde::{Deserialize, Serialize};

use crate::drive::RemoteFile;

/// The manifest's file name inside a folder directory.
pub(crate) const MANIFEST_FILE: &str = "manifest.json";

/// One cached, playable file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestItem {
    pub(crate) file_id: String,
    pub(crate) name: String,
    /// The MIME type from the Drive listing (served as `Content-Type`).
    pub(crate) mime: String,
    pub(crate) kind: PlaybackItemKind,
    /// The cached file's extension (from [`playable`]).
    pub(crate) ext: String,
    pub(crate) size: Option<u64>,
    pub(crate) last_modified: Option<String>,
}

impl ManifestItem {
    /// The cached file's name: `<file_id>.<ext>`. Both parts are validated
    /// (`is_valid_drive_file_id` + the [`playable`] whitelist), so the name can
    /// never carry `/` or `..`.
    pub(crate) fn file_name(&self) -> String {
        format!("{}.{}", self.file_id, self.ext)
    }
}

/// The mirror state of one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub(crate) folder_id: String,
    #[serde(default)]
    pub(crate) items: Vec<ManifestItem>,
    /// The last SUCCESSFUL listing (even if some files failed to download).
    #[serde(default)]
    pub(crate) synced_at: Option<DateTime<Utc>>,
    /// The last error (a failed listing, or files that failed to download).
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) error_at: Option<DateTime<Utc>>,
}

/// The item kind + cache extension of a playable MIME type, `None` for anything
/// the output page cannot play. Images are the formats every Chromium (OBS CEF
/// included) renders — not `image/*` wholesale: `image/heic` (iPhone photos)
/// does not render, and an `image/svg+xml` served from our origin could run
/// script when opened directly. Videos are `mp4` (H.264 in OBS) and `webm`;
/// `video/quicktime` does not play in Chromium (#843 research).
pub(crate) fn playable(mime: &str) -> Option<(PlaybackItemKind, &'static str)> {
    let kind_ext = match mime {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => (PlaybackItemKind::Image, "jpg"),
        "image/png" => (PlaybackItemKind::Image, "png"),
        "image/webp" => (PlaybackItemKind::Image, "webp"),
        "image/gif" => (PlaybackItemKind::Image, "gif"),
        "image/avif" => (PlaybackItemKind::Image, "avif"),
        "image/bmp" => (PlaybackItemKind::Image, "bmp"),
        "video/mp4" => (PlaybackItemKind::Video, "mp4"),
        "video/webm" => (PlaybackItemKind::Video, "webm"),
        _ => return None,
    };
    Some(kind_ext)
}

/// `true` when a cached file must be (re)downloaded: it is missing on disk,
/// the manifest has no record of it, or the HEAD reports a different size or
/// last-modified. A field the HEAD did not report is not compared.
pub(crate) fn needs_download(
    old: Option<&ManifestItem>,
    remote: &RemoteFile,
    cached: bool,
) -> bool {
    let Some(old) = old.filter(|_| cached) else {
        return true;
    };
    let size_changed = remote.size.is_some() && remote.size != old.size;
    let modified_changed =
        remote.last_modified.is_some() && remote.last_modified != old.last_modified;
    size_changed || modified_changed
}

/// A short cache-busting token for an item's URL (`?v=`): changes whenever the
/// cached bytes change (size or last-modified), so the output page's browser
/// cache never serves an old version of a replaced file.
pub(crate) fn version_token(item: &ManifestItem) -> String {
    let key = format!(
        "{}|{}",
        item.size.unwrap_or_default(),
        item.last_modified.as_deref().unwrap_or_default()
    );
    let mut hash = crate::state::stream_assets::sha256_hex(key.as_bytes());
    hash.truncate(12);
    hash
}

/// The id-addressed URL the output page loads an item from.
pub(crate) fn item_url(folder_id: &str, item: &ManifestItem) -> String {
    format!(
        "/stream/playback/{folder_id}/{}?v={}",
        item.file_id,
        version_token(item)
    )
}

/// The API playlist of a folder (`items` = the given, already filtered list).
pub(crate) fn to_playlist(
    folder_id: &str,
    manifest: Option<&Manifest>,
    items: &[ManifestItem],
    syncing: bool,
) -> PlaybackPlaylist {
    PlaybackPlaylist {
        folder_id: folder_id.to_string(),
        items: items
            .iter()
            .map(|item| PlaybackItem {
                file_id: item.file_id.clone(),
                kind: item.kind,
                name: item.name.clone(),
                url: item_url(folder_id, item),
            })
            .collect(),
        synced_at: manifest.and_then(|m| m.synced_at),
        error: manifest.and_then(|m| m.last_error.clone()),
        syncing,
    }
}

/// Read a folder's manifest. `None` when there is none yet; a corrupt manifest
/// is logged and treated as none (the next sync rewrites it).
pub(crate) async fn read_manifest(dir: &Path) -> Option<Manifest> {
    let path = dir.join(MANIFEST_FILE);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "playback: manifest unreadable");
            return None;
        }
    };
    match serde_json::from_slice::<Manifest>(&bytes) {
        Ok(manifest) => Some(manifest),
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "playback: manifest corrupt — resyncing");
            None
        }
    }
}

/// Write a folder's manifest atomically (unique tmp file + rename), so a
/// reader never sees half a file.
pub(crate) async fn write_manifest(dir: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(".manifest-{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, dir.join(MANIFEST_FILE)).await
}
