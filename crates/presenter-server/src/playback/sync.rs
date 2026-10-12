//! Mirror ONE Drive folder into its cache directory (#843).
//!
//! 1. List the folder (a failed listing aborts — nothing is touched, the last
//!    good playlist keeps playing).
//! 2. Keep images and `video/mp4|webm` in listing order; skip subfolders and
//!    other types.
//! 3. For each kept file, HEAD it and (re)download only when it is missing or
//!    its size / last-modified changed. A download streams into a temp file
//!    and is renamed into place, so a reader never sees a partial file.
//! 4. Write the manifest, then delete the files that left the folder — after a
//!    5-minute grace period in which they are still served (`retired`), since
//!    output pages refetch their playlist only every 60 s.
//!
//! A file that fails (HEAD or download) keeps its previously cached copy when
//! there is one; otherwise it is left out of the playlist. Every failure is
//! recorded in the manifest (`FileFailure`): the file is not tried again
//! before its exponential backoff ends (5 min … 6 h; "Obnoviť" retries at
//! once), repeated failures log on the #484 power-of-two gate, and the first
//! failure's reason becomes the manifest's `last_error`. A download that would
//! push the whole mirror past its byte budget is refused before it starts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, Utc};
use presenter_core::{DriveFolder, PlaybackItemKind};

use super::manifest::{
    failure_summary, needs_download, playable, read_manifest, retire, write_manifest, FileFailure,
    Manifest, ManifestItem, RetiredFile, MANIFEST_FILE,
};
use super::{retry_backoff, should_log_failure, Budget};
use crate::drive::{DriveClient, DriveEntry, DriveError, RemoteFile};

/// Prefix of an in-flight download's temp file inside a folder directory.
const DOWNLOAD_TMP_PREFIX: &str = ".download-";
const MIB: u64 = 1024 * 1024;

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
    /// Failing files not tried this pass (their retry backoff runs).
    pub(crate) backed_off: usize,
    pub(crate) pruned: usize,
    pub(crate) bytes: u64,
}

/// Sync `folder` into `dir`; returns the written manifest and the report.
/// `force` ("Obnoviť") retries failing files before their backoff ends;
/// `budget` is the mirror-wide byte budget, updated as files come and go.
pub(crate) async fn sync_folder(
    client: &DriveClient,
    dir: &Path,
    folder: &DriveFolder,
    force: bool,
    budget: &mut Budget,
) -> Result<(Manifest, SyncReport), DriveError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| DriveError::Io {
            detail: err.to_string(),
        })?;
    sweep_download_tmp(dir).await;
    let old = read_manifest(dir).await.unwrap_or_default();
    let entries = client.list_folder(folder).await?;
    // The files that left the folder are retired and pruned a few minutes
    // later: their bytes already count as free for this folder's downloads
    // (the next pass counts retirees again while they are still on disk).
    budget.used = budget.used.saturating_sub(bytes_leaving(&old, &entries));

    let mut sync = FolderSync {
        client,
        dir,
        old,
        force,
        now: Utc::now(),
        budget,
        failures: Vec::new(),
        report: SyncReport {
            listed: entries.len(),
            ..SyncReport::default()
        },
    };
    let mut items = Vec::new();
    for entry in &entries {
        if let Some(item) = sync.entry(entry).await {
            items.push(item);
        }
    }
    let FolderSync {
        failures,
        mut report,
        now,
        old,
        ..
    } = sync;
    report.items = items.len();
    let retired = retire(&old, &items, now);
    let manifest = Manifest {
        folder_id: folder.id.clone(),
        items,
        synced_at: Some(now),
        last_error: failure_summary(&failures),
        error_at: (!failures.is_empty()).then_some(now),
        failures,
        retired,
    };
    write_manifest(dir, &manifest)
        .await
        .map_err(|err| DriveError::Io {
            detail: err.to_string(),
        })?;
    report.pruned = prune_files(dir, &manifest).await;
    Ok((manifest, report))
}

/// Cached bytes of the files no longer in the listing.
fn bytes_leaving(old: &Manifest, entries: &[DriveEntry]) -> u64 {
    old.items
        .iter()
        .filter(|item| !entries.iter().any(|entry| entry.id == item.file_id))
        .filter_map(|item| item.size)
        .fold(0, u64::saturating_add)
}

/// One folder sync in progress.
struct FolderSync<'a> {
    client: &'a DriveClient,
    dir: &'a Path,
    old: Manifest,
    force: bool,
    now: DateTime<Utc>,
    budget: &'a mut Budget,
    failures: Vec<FileFailure>,
    report: SyncReport,
}

/// What fetching one file did.
enum Fetched {
    /// The cached copy is current.
    Unchanged,
    Downloaded(ManifestItem),
}

impl FolderSync<'_> {
    /// One listing entry → its manifest item (`None` when skipped / failed).
    async fn entry(&mut self, entry: &DriveEntry) -> Option<ManifestItem> {
        let mime = entry.mime.as_deref().unwrap_or_default();
        let playable_kind = if entry.is_folder {
            None
        } else {
            playable(mime)
        };
        let Some((kind, ext)) = playable_kind else {
            tracing::debug!(file_id = %entry.id, name = %entry.name, mime, folder = entry.is_folder, "playback: entry skipped");
            self.report.skipped += 1;
            return None;
        };
        let path = self.dir.join(format!("{}.{ext}", entry.id));
        let cached = tokio::fs::metadata(&path).await.is_ok();
        // A file back within its grace period still has its cached copy.
        let previous = self
            .old
            .items
            .iter()
            .find(|item| item.file_id == entry.id && item.ext == ext)
            .cloned()
            .or_else(|| {
                self.old
                    .retired
                    .iter()
                    .find(|retired| retired.file_id == entry.id && retired.ext == ext)
                    .map(|retired| ManifestItem {
                        file_id: retired.file_id.clone(),
                        name: entry.name.clone(),
                        mime: retired.mime.clone(),
                        kind,
                        ext: retired.ext.clone(),
                        size: retired.size,
                        last_modified: retired.last_modified.clone(),
                    })
            });
        let prior = self
            .old
            .failures
            .iter()
            .find(|failure| failure.file_id == entry.id)
            .cloned();
        if let Some(failure) = prior
            .clone()
            .filter(|f| !self.force && f.retry_at > self.now)
        {
            tracing::debug!(file_id = %entry.id, streak = failure.streak, retry_at = %failure.retry_at, "playback: file waits for its retry");
            self.report.backed_off += 1;
            self.failures.push(failure);
            return keep(previous.as_ref(), cached, entry, &mut self.report);
        }
        let target = Target {
            entry,
            path: &path,
            mime,
            kind,
            ext,
        };
        match self.fetch(&target, previous.as_ref(), cached).await {
            Ok(fetched) => {
                if let Some(failure) = prior {
                    tracing::info!(file_id = %entry.id, name = %entry.name, failures = failure.streak, "playback: file fetch recovered");
                }
                match fetched {
                    Fetched::Downloaded(item) => Some(item),
                    Fetched::Unchanged => keep(previous.as_ref(), cached, entry, &mut self.report),
                }
            }
            Err(err) => {
                self.record_failure(entry, prior.as_ref(), &err);
                keep(previous.as_ref(), cached, entry, &mut self.report).or_else(|| {
                    self.report.failed += 1;
                    None
                })
            }
        }
    }

    /// HEAD the file and download it when it changed (within the budget).
    async fn fetch(
        &mut self,
        target: &Target<'_>,
        previous: Option<&ManifestItem>,
        cached: bool,
    ) -> Result<Fetched, DriveError> {
        let entry = target.entry;
        let remote = self
            .client
            .head_file(&entry.id, entry.resource_key.as_deref())
            .await?;
        if !needs_download(previous, &remote, cached) {
            return Ok(Fetched::Unchanged);
        }
        // Replacing a cached file frees its bytes.
        let replaced = previous
            .filter(|_| cached)
            .and_then(|item| item.size)
            .unwrap_or_default();
        let allowance = self.budget.remaining().saturating_add(replaced);
        let over_budget = DriveError::BudgetExceeded {
            limit_mib: self.budget.limit / MIB,
        };
        if remote.size.is_some_and(|size| size > allowance) {
            return Err(over_budget);
        }
        let written = match download(self.client, self.dir, entry, target.path, allowance).await {
            // The budget, not the per-file cap, stopped a size-less download.
            Err(DriveError::TooLarge { .. }) if allowance < self.client.max_file_bytes() => {
                return Err(over_budget);
            }
            other => other?,
        };
        let size = written.size.unwrap_or_default();
        self.budget.used = self
            .budget
            .used
            .saturating_sub(replaced)
            .saturating_add(size);
        self.report.downloaded += 1;
        self.report.bytes = self.report.bytes.saturating_add(size);
        Ok(Fetched::Downloaded(ManifestItem {
            file_id: entry.id.clone(),
            name: entry.name.clone(),
            mime: target.mime.to_string(),
            kind: target.kind,
            ext: target.ext.to_string(),
            size: written.size,
            // The HEAD's date is the one the next HEAD compares with.
            last_modified: remote.last_modified.or(written.last_modified),
        }))
    }

    /// Record a failed fetch: a longer backoff each time, a WARN only on the
    /// #484 power-of-two milestones (DEBUG otherwise).
    fn record_failure(
        &mut self,
        entry: &DriveEntry,
        prior: Option<&FileFailure>,
        err: &DriveError,
    ) {
        let streak = prior.map_or(0, |f| f.streak).saturating_add(1);
        let backoff = retry_backoff(streak);
        if should_log_failure(streak) {
            tracing::warn!(file_id = %entry.id, name = %entry.name, streak, retry_in_min = backoff.num_minutes(), error = %err, "playback: file fetch failed");
        } else {
            tracing::debug!(file_id = %entry.id, name = %entry.name, streak, error = %err, "playback: file fetch failed again");
        }
        self.failures.push(FileFailure {
            file_id: entry.id.clone(),
            name: entry.name.clone(),
            streak,
            retry_at: self.now.checked_add_signed(backoff).unwrap_or(self.now),
            reason: err.to_string(),
        });
    }
}

/// The file being fetched and how it is stored.
struct Target<'a> {
    entry: &'a DriveEntry,
    path: &'a Path,
    mime: &'a str,
    kind: PlaybackItemKind,
    ext: &'static str,
}

/// Keep the previously cached copy (renamed if the name changed), if any.
fn keep(
    previous: Option<&ManifestItem>,
    cached: bool,
    entry: &DriveEntry,
    report: &mut SyncReport,
) -> Option<ManifestItem> {
    let previous = previous.filter(|_| cached)?;
    report.kept += 1;
    Some(ManifestItem {
        name: entry.name.clone(),
        ..previous.clone()
    })
}

/// Download one file (at most `max_bytes`) into a temp file, then rename it
/// to `path`.
async fn download(
    client: &DriveClient,
    dir: &Path,
    entry: &DriveEntry,
    path: &Path,
    max_bytes: u64,
) -> Result<RemoteFile, DriveError> {
    let started = Instant::now();
    let tmp: PathBuf = dir.join(format!("{DOWNLOAD_TMP_PREFIX}{}.tmp", uuid::Uuid::new_v4()));
    let result = client
        .download_file(&entry.id, entry.resource_key.as_deref(), &tmp, max_bytes)
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

/// Delete every file in `dir` that the manifest neither plays nor still serves
/// as retired (the files that left the folder more than the grace period ago,
/// or changed type). Returns how many were deleted.
async fn prune_files(dir: &Path, manifest: &Manifest) -> usize {
    let keep: HashSet<String> = manifest
        .items
        .iter()
        .map(ManifestItem::file_name)
        .chain(manifest.retired.iter().map(RetiredFile::file_name))
        .collect();
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
