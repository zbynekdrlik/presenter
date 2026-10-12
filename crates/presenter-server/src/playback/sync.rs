//! Mirror ONE Drive folder into its cache directory (#843).
//!
//! 1. List the folder (a failed listing aborts — nothing is touched, the last
//!    good playlist keeps playing).
//! 2. Keep images and `video/mp4|webm` in listing order; skip subfolders and
//!    other types.
//! 3. For each kept file, HEAD it and (re)download only when it is missing or
//!    its size / last-modified changed. A download streams into a temp file
//!    and is renamed into place, so a reader never sees a partial file.
//! 4. Write the manifest, then delete the files that left the folder.
//!
//! A file that fails (HEAD or download) keeps its previously cached copy when
//! there is one; otherwise it is left out of the playlist and counted in the
//! manifest's error, and the next sync tries again.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use presenter_core::DriveFolder;

use super::manifest::{
    needs_download, playable, read_manifest, write_manifest, Manifest, ManifestItem, MANIFEST_FILE,
};
use crate::drive::{DriveClient, DriveEntry, DriveError, RemoteFile};

/// Prefix of an in-flight download's temp file inside a folder directory.
const DOWNLOAD_TMP_PREFIX: &str = ".download-";

/// What one folder sync did — logged as one line per folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SyncReport {
    /// Entries in the listing (files + subfolders).
    pub(crate) listed: usize,
    /// Playlist items after the sync.
    pub(crate) items: usize,
    pub(crate) downloaded: usize,
    /// Cached files that were unchanged (or kept after a failed refresh).
    pub(crate) kept: usize,
    /// Subfolders and files of a type the output page cannot play.
    pub(crate) skipped: usize,
    /// Files that could not be fetched and had no cached copy.
    pub(crate) failed: usize,
    pub(crate) pruned: usize,
    pub(crate) bytes: u64,
}

/// Sync `folder` into `dir`; returns the written manifest and the report.
pub(crate) async fn sync_folder(
    client: &DriveClient,
    dir: &Path,
    folder: &DriveFolder,
) -> Result<(Manifest, SyncReport), DriveError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| DriveError::Io {
            detail: err.to_string(),
        })?;
    sweep_download_tmp(dir).await;
    let old = read_manifest(dir).await.unwrap_or_default();
    let entries = client.list_folder(folder).await?;

    let mut report = SyncReport {
        listed: entries.len(),
        ..SyncReport::default()
    };
    let mut items = Vec::new();
    for entry in &entries {
        if let Some(item) = sync_entry(client, dir, entry, &old, &mut report).await {
            items.push(item);
        }
    }
    report.items = items.len();

    let failed = report.failed;
    let now = Utc::now();
    let manifest = Manifest {
        folder_id: folder.id.clone(),
        items,
        synced_at: Some(now),
        last_error: (failed > 0).then(|| format!("{failed} súbor(ov) sa nepodarilo stiahnuť")),
        error_at: (failed > 0).then_some(now),
    };
    write_manifest(dir, &manifest)
        .await
        .map_err(|err| DriveError::Io {
            detail: err.to_string(),
        })?;
    report.pruned = prune_files(dir, &manifest).await;
    Ok((manifest, report))
}

/// One listing entry → its manifest item (or `None` when skipped / failed).
async fn sync_entry(
    client: &DriveClient,
    dir: &Path,
    entry: &DriveEntry,
    old: &Manifest,
    report: &mut SyncReport,
) -> Option<ManifestItem> {
    let mime = entry.mime.as_deref().unwrap_or_default();
    let playable_kind = if entry.is_folder {
        None
    } else {
        playable(mime)
    };
    let Some((kind, ext)) = playable_kind else {
        tracing::debug!(file_id = %entry.id, name = %entry.name, mime, folder = entry.is_folder, "playback: entry skipped");
        report.skipped += 1;
        return None;
    };
    let path = dir.join(format!("{}.{ext}", entry.id));
    let cached = tokio::fs::metadata(&path).await.is_ok();
    let previous = old
        .items
        .iter()
        .find(|item| item.file_id == entry.id && item.ext == ext);

    let remote = match client
        .head_file(&entry.id, entry.resource_key.as_deref())
        .await
    {
        Ok(remote) => remote,
        Err(err) => {
            tracing::warn!(file_id = %entry.id, name = %entry.name, error = %err, cached, "playback: file check failed");
            return keep_or_fail(previous, cached, entry, report);
        }
    };
    if !needs_download(previous, &remote, cached) {
        return keep_or_fail(previous, cached, entry, report);
    }
    match download(client, dir, entry, &path).await {
        Ok(written) => {
            report.downloaded += 1;
            report.bytes = report
                .bytes
                .saturating_add(written.size.unwrap_or_default());
            Some(ManifestItem {
                file_id: entry.id.clone(),
                name: entry.name.clone(),
                mime: mime.to_string(),
                kind,
                ext: ext.to_string(),
                size: written.size,
                // The HEAD's date is the one the next HEAD compares with.
                last_modified: remote.last_modified.or(written.last_modified),
            })
        }
        Err(err) => {
            tracing::warn!(file_id = %entry.id, name = %entry.name, error = %err, cached, "playback: download failed");
            keep_or_fail(previous, cached, entry, report)
        }
    }
}

/// Keep the previously cached copy (renamed if the name changed), or count the
/// file as failed when there is none.
fn keep_or_fail(
    previous: Option<&ManifestItem>,
    cached: bool,
    entry: &DriveEntry,
    report: &mut SyncReport,
) -> Option<ManifestItem> {
    match previous.filter(|_| cached) {
        Some(previous) => {
            report.kept += 1;
            Some(ManifestItem {
                name: entry.name.clone(),
                ..previous.clone()
            })
        }
        None => {
            report.failed += 1;
            None
        }
    }
}

/// Download one file into a temp file, then rename it to `path`.
async fn download(
    client: &DriveClient,
    dir: &Path,
    entry: &DriveEntry,
    path: &Path,
) -> Result<RemoteFile, DriveError> {
    let started = Instant::now();
    let tmp: PathBuf = dir.join(format!("{DOWNLOAD_TMP_PREFIX}{}.tmp", uuid::Uuid::new_v4()));
    let result = client
        .download_file(&entry.id, entry.resource_key.as_deref(), &tmp)
        .await;
    let written = match result {
        Ok(written) => written,
        Err(err) => {
            remove_quietly(&tmp).await;
            return Err(err);
        }
    };
    if let Err(err) = tokio::fs::rename(&tmp, path).await {
        remove_quietly(&tmp).await;
        return Err(DriveError::Io {
            detail: err.to_string(),
        });
    }
    tracing::info!(
        file_id = %entry.id,
        name = %entry.name,
        bytes = written.size.unwrap_or_default(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "playback: file downloaded"
    );
    Ok(written)
}

/// Delete every file in `dir` that the manifest does not list (the files that
/// left the folder, or changed type). Returns how many were deleted.
async fn prune_files(dir: &Path, manifest: &Manifest) -> usize {
    let keep: HashSet<String> = manifest.items.iter().map(ManifestItem::file_name).collect();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(dir = %dir.display(), error = %err, "playback: cannot list the cache for pruning");
            return 0;
        }
    };
    let mut pruned = 0;
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(dir = %dir.display(), error = %err, "playback: cache listing failed");
                break;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == MANIFEST_FILE || name.starts_with('.') || keep.contains(&name) {
            continue;
        }
        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => {
                tracing::info!(dir = %dir.display(), file = %name, "playback: removed a file that left the folder");
                pruned += 1;
            }
            Err(err) => {
                tracing::warn!(dir = %dir.display(), file = %name, error = %err, "playback: prune failed");
            }
        }
    }
    pruned
}

/// Remove leftover download temp files (a crash mid-download). Only the sync
/// writes into a folder directory, and only one sync runs at a time.
async fn sweep_download_tmp(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(DOWNLOAD_TMP_PREFIX) || name.starts_with(".manifest-") {
            remove_quietly(&entry.path()).await;
        }
    }
}

async fn remove_quietly(path: &Path) {
    if let Err(err) = tokio::fs::remove_file(path).await {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), error = %err, "playback: temp file not removed");
        }
    }
}
