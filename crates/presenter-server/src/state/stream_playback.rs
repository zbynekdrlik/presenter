//! `AppState` glue of the PLAYBACK Drive mirror (#843): which folders the
//! stream elements reference, the background sync worker, and the reads the
//! `/stream/api/playback` + `/stream/playback/*` routes serve.
//!
//! The worker is spawned from `spawn_background_tasks`, so a validate-mode
//! (schema probe, #771) boot never starts it. It owns all writes into
//! `<stream-assets>/playback/`; the routes only read.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use presenter_core::{
    is_valid_drive_folder_id, parse_drive_folder_url, DriveFolder, PlaybackPlaylist,
    StreamElementProps,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{interval, MissedTickBehavior};

use super::AppState;
#[cfg(test)]
use crate::drive::DriveClient;
use crate::live::LiveEvent;
use crate::playback::manifest::{
    read_manifest, to_playlist, write_manifest, Manifest, ManifestItem,
};
use crate::playback::sync::sync_folder;
#[cfg(test)]
use crate::playback::PlaybackSync;
use crate::playback::{
    should_log_failure, should_prune_unreferenced, should_sync, Budget, SyncTrigger, PLAYBACK_DIR,
    SYNC_INTERVAL, TRIGGER_DEBOUNCE,
};

impl AppState {
    /// `<stream-assets>/playback` — follows `set_stream_assets_dir` in tests.
    pub(crate) fn playback_dir(&self) -> PathBuf {
        self.stream_assets_dir.join(PLAYBACK_DIR)
    }

    /// Point the playback sync at a mock Drive (test isolation only).
    #[cfg(test)]
    pub(crate) fn set_playback_drive_client(&mut self, client: DriveClient) {
        self.playback = PlaybackSync::with_client(client);
    }

    /// Spawn the sync worker: a pass at startup, then every 5 min, after a
    /// `StreamConfigChanged` and on a refresh nudge (bursts debounced).
    ///
    /// A tiny forwarder task reads the live hub and only bumps the sync's
    /// config-change counter (a `watch`), so the hub receiver is always drained
    /// in microseconds and never lags — even while the worker spends minutes in
    /// a download pass. (Reading the hub in the worker itself made every long
    /// pass lag and start another pass at once.)
    pub(super) fn spawn_playback_sync_task(&self) {
        let forwarder = self.playback.clone();
        let mut events = self.live_hub.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    // A lag here means the runtime was starved; a lost event
                    // may have been a config change.
                    Ok(LiveEvent::StreamConfigChanged { .. }) | Err(RecvError::Lagged(_)) => {
                        forwarder.note_config_change();
                    }
                    Ok(_) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let app = self.clone();
        let mut config_changes = self.playback.config_changes();
        tokio::spawn(async move {
            let mut ticker = interval(SYNC_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                let trigger = tokio::select! {
                    _ = ticker.tick() => SyncTrigger::Periodic,
                    () = app.playback.nudged() => SyncTrigger::Forced,
                    changed = config_changes.changed() => match changed {
                        Ok(()) => SyncTrigger::ConfigChanged,
                        Err(_) => break,
                    },
                };
                if trigger != SyncTrigger::Periodic {
                    tokio::time::sleep(TRIGGER_DEBOUNCE).await;
                    // The pass below covers the changes of the debounce window.
                    config_changes.mark_unchanged();
                }
                app.run_playback_pass(trigger).await;
            }
            tracing::warn!("playback: config-change channel closed — sync worker stopped");
        });
    }

    /// One sync pass: every referenced folder (plus folders a refresh asked
    /// for), one at a time; then the mirrors no element uses any more are
    /// pruned (after a grace period).
    pub(crate) async fn run_playback_pass(&self, trigger: SyncTrigger) {
        let _pass = self.playback.lock_pass().await;
        let started = Instant::now();
        // Marks set BEFORE this pass (a refresh); one set while it runs belongs
        // to the next pass, which the refresh's nudge queues.
        let marked_before = self.playback.pending_snapshot();
        let (mut folders, complete) = self.referenced_playback_folders().await;
        for extra in self.playback.take_extras() {
            if !folders.iter().any(|f| f.id == extra.id) {
                folders.push(extra);
            }
        }
        let root = self.playback_dir();
        let now = Utc::now();
        let mut due = Vec::new();
        for folder in &folders {
            let manifest = read_manifest(&root.join(&folder.id)).await;
            if should_sync(trigger, manifest.as_ref(), now) {
                due.push(folder.clone());
            }
        }
        self.playback.mark_pending(due.iter().map(|f| f.id.clone()));
        let mut budget = Budget {
            limit: self.playback.budget_limit(),
            used: mirror_bytes(&root).await,
        };
        if !due.is_empty() {
            tracing::info!(
                trigger = trigger.as_str(),
                folders = folders.len(),
                due = due.len(),
                mirror_mib = budget.used / (1024 * 1024),
                budget_mib = budget.limit / (1024 * 1024),
                "playback: sync pass started"
            );
        }
        let force = trigger == SyncTrigger::Forced;
        for folder in &due {
            self.sync_playback_folder(&root, folder, force, &mut budget)
                .await;
            self.playback.clear_pending(&folder.id);
        }
        if trigger == SyncTrigger::Forced {
            // A refresh mark this pass could not serve (its element was deleted
            // meanwhile) must not show "syncing" forever.
            for id in marked_before
                .iter()
                .filter(|id| !due.iter().any(|f| &f.id == *id))
            {
                self.playback.clear_pending(id);
            }
        }
        if complete {
            let keep: HashSet<String> = folders.iter().map(|f| f.id.clone()).collect();
            prune_unreferenced(&root, &keep).await;
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if due.is_empty() {
            tracing::debug!(
                trigger = trigger.as_str(),
                folders = folders.len(),
                elapsed_ms,
                "playback: nothing to sync"
            );
        } else {
            tracing::info!(
                trigger = trigger.as_str(),
                due = due.len(),
                elapsed_ms,
                "playback: sync pass finished"
            );
        }
    }

    /// Sync one folder and log the outcome (#484 gate on repeated failures).
    async fn sync_playback_folder(
        &self,
        root: &Path,
        folder: &DriveFolder,
        force: bool,
        budget: &mut Budget,
    ) {
        let dir = root.join(&folder.id);
        let started = Instant::now();
        match sync_folder(self.playback.client(), &dir, folder, force, budget).await {
            Ok((_, report)) => {
                let ended = self.playback.record_success(&folder.id);
                if ended > 0 {
                    tracing::info!(folder = %folder.id, failures = ended, "playback: folder sync recovered");
                }
                tracing::info!(
                    folder = %folder.id,
                    listed = report.listed,
                    items = report.items,
                    downloaded = report.downloaded,
                    kept = report.kept,
                    skipped = report.skipped,
                    failed = report.failed,
                    backed_off = report.backed_off,
                    pruned = report.pruned,
                    bytes = report.bytes,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "playback: folder synced"
                );
            }
            Err(err) => {
                let streak = self.playback.record_failure(&folder.id);
                if should_log_failure(streak) {
                    tracing::warn!(folder = %folder.id, streak, error = %err, "playback: folder sync failed — the last good playlist keeps playing");
                } else {
                    tracing::debug!(folder = %folder.id, streak, error = %err, "playback: folder sync failed again");
                }
                record_sync_error(&dir, &folder.id, &err.to_string()).await;
            }
        }
    }

    /// The Drive folders named by playback elements of every output (one per
    /// id; a `resourcekey` from any element naming it is kept). `complete` is
    /// `false` when an output's def could not be read — the pass then prunes
    /// nothing, so a DB hiccup never deletes a mirror.
    async fn referenced_playback_folders(&self) -> (Vec<DriveFolder>, bool) {
        let outputs = match self.repository.list_stream_outputs().await {
            Ok(outputs) => outputs,
            Err(err) => {
                tracing::warn!(error = %err, "playback: cannot list stream outputs");
                return (Vec::new(), false);
            }
        };
        let mut complete = true;
        let mut folders: Vec<DriveFolder> = Vec::new();
        for output in outputs {
            let def = match self.repository.load_output_def(&output.slug).await {
                Ok(def) => def,
                Err(err) => {
                    tracing::warn!(output = %output.slug, error = %err, "playback: cannot read an output def");
                    complete = false;
                    continue;
                }
            };
            for element in def.scenes.iter().flat_map(|s| &s.elements) {
                if let StreamElementProps::Playback { source_url, .. } = &element.props {
                    add_folder(&mut folders, source_url);
                }
            }
        }
        (folders, complete)
    }

    /// The playlist of `folder` as cached right now (files missing on disk are
    /// left out, so the output never requests a 404).
    pub(crate) async fn playback_playlist(&self, folder: &DriveFolder) -> PlaybackPlaylist {
        let dir = self.playback_dir().join(&folder.id);
        let manifest = read_manifest(&dir).await;
        let items = match manifest.as_ref() {
            Some(manifest) => present_items(&dir, &manifest.items).await,
            None => Vec::new(),
        };
        let syncing = self.playback.is_pending(&folder.id);
        to_playlist(&folder.id, manifest.as_ref(), &items, syncing)
    }

    /// The cached file + its MIME type for `/stream/playback/{folder}/{file}`.
    /// Both ids must already be validated by the caller.
    pub(crate) async fn playback_file(
        &self,
        folder_id: &str,
        file_id: &str,
    ) -> Option<(PathBuf, String)> {
        let dir = self.playback_dir().join(folder_id);
        let manifest = read_manifest(&dir).await?;
        if let Some(item) = manifest.items.iter().find(|item| item.file_id == file_id) {
            return Some((dir.join(item.file_name()), item.mime.clone()));
        }
        // A file that just left the folder is still served for its grace
        // period: an output's playlist can be up to 60 s old.
        let retired = manifest
            .retired
            .iter()
            .find(|retired| retired.file_id == file_id)?;
        Some((dir.join(retired.file_name()), retired.mime.clone()))
    }

    /// „Obnoviť": queue a full pass now. `extra` (the editor's link) is synced
    /// even before a saved element names it. The folders show `syncing` at
    /// once.
    pub(crate) async fn request_playback_refresh(&self, extra: Option<DriveFolder>) {
        let mut ids = Vec::new();
        if let Some(folder) = extra {
            ids.push(folder.id.clone());
            self.playback.add_extra(folder);
        }
        let (folders, _) = self.referenced_playback_folders().await;
        ids.extend(folders.into_iter().map(|f| f.id));
        tracing::info!(folders = ids.len(), "playback: refresh requested");
        self.playback.mark_pending(ids);
        self.playback.nudge();
    }
}

/// Add the folder a link names (skipping empty / unparseable links; the first
/// `resourcekey` seen for an id wins).
fn add_folder(folders: &mut Vec<DriveFolder>, source_url: &str) {
    if source_url.trim().is_empty() {
        return;
    }
    let folder = match parse_drive_folder_url(source_url) {
        Ok(folder) => folder,
        Err(err) => {
            tracing::debug!(error = %err, "playback: element link is not a Drive folder");
            return;
        }
    };
    match folders.iter_mut().find(|f| f.id == folder.id) {
        Some(existing) => {
            if existing.resource_key.is_none() {
                existing.resource_key = folder.resource_key;
            }
        }
        None => folders.push(folder),
    }
}

/// Cached bytes of the whole mirror (every folder's manifest sizes) — the
/// starting point of a pass's byte budget.
async fn mirror_bytes(root: &Path) -> u64 {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return 0;
    };
    let mut total: u64 = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if !entry.file_type().await.is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if let Some(manifest) = read_manifest(&entry.path()).await {
            let playing = manifest
                .items
                .iter()
                .filter_map(|item| item.size)
                .fold(0, u64::saturating_add);
            let retired = manifest
                .retired
                .iter()
                .filter_map(|r| r.size)
                .fold(0, u64::saturating_add);
            total = total.saturating_add(playing).saturating_add(retired);
        }
    }
    total
}

/// The manifest items whose cached file exists. A missing file is logged at
/// DEBUG only: every playlist GET (each output every 60 s, the editor every
/// 2 s) passes here, and the next sync sees the file missing and downloads it
/// again with an INFO line (#484 log-flood rule).
async fn present_items(dir: &Path, items: &[ManifestItem]) -> Vec<ManifestItem> {
    let mut present = Vec::with_capacity(items.len());
    for item in items {
        if tokio::fs::metadata(dir.join(item.file_name()))
            .await
            .is_ok()
        {
            present.push(item.clone());
        } else {
            tracing::debug!(dir = %dir.display(), file = %item.file_name(), "playback: cached file missing — left out of the playlist");
        }
    }
    present
}

/// Record a failed sync in the folder's manifest; the items stay.
async fn record_sync_error(dir: &Path, folder_id: &str, message: &str) {
    let mut manifest = read_manifest(dir).await.unwrap_or_else(|| Manifest {
        folder_id: folder_id.to_string(),
        ..Manifest::default()
    });
    manifest.last_error = Some(message.to_string());
    manifest.error_at = Some(Utc::now());
    if let Err(err) = write_manifest(dir, &manifest).await {
        tracing::warn!(dir = %dir.display(), error = %err, "playback: cannot record the sync error");
    }
}

/// Delete the mirror of every folder no element (and no recent refresh)
/// names, once its grace period is over.
async fn prune_unreferenced(root: &Path, keep: &HashSet<String>) {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            tracing::warn!(dir = %root.display(), error = %err, "playback: cannot list the mirror root");
            return;
        }
    };
    let now = Utc::now();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name) || !is_valid_drive_folder_id(&name) {
            continue;
        }
        let is_dir = entry.file_type().await.is_ok_and(|t| t.is_dir());
        if !is_dir {
            continue;
        }
        let manifest = read_manifest(&entry.path()).await;
        if !should_prune_unreferenced(manifest.as_ref(), now) {
            continue;
        }
        match tokio::fs::remove_dir_all(entry.path()).await {
            Ok(()) => {
                tracing::info!(folder = %name, "playback: removed the mirror of a folder no element uses");
            }
            Err(err) => {
                tracing::warn!(folder = %name, error = %err, "playback: cannot remove an unused mirror");
            }
        }
    }
}
