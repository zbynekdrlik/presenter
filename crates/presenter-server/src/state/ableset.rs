use chrono::{DateTime, Utc};
use presenter_core::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSongSnapshot,
    AbleSetTitleMismatch, PresentationId,
};
use std::collections::{HashMap, VecDeque};
use tracing::{info, warn};

use super::AppState;
use crate::ableset::AbleSetStatusSnapshot;

/// Maximum number of recent resolution attempts retained in the ring buffer
/// (#600). Enough to cover a typical worship set (15-20 songs) while bounding
/// memory on a long-running instance.
const MAX_RECENT_ATTEMPTS: usize = 20;

/// A single AbleSet song-resolution attempt, retained for diagnostic purposes
/// (#600). Surfaced read-only via `/integrations/ableset/status` so operators
/// can answer "it didn't work 30 min ago" from data instead of memory.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbleSetResolutionAttempt {
    /// ISO-8601 timestamp of the resolution attempt.
    pub(crate) timestamp: DateTime<Utc>,
    /// The incoming prefix (trimmed) that was looked up.
    pub(crate) input: String,
    /// Whether the prefix resolved to a known presentation (`true`) or was a
    /// cache miss (`false`).
    pub(crate) found: bool,
}

#[derive(Default)]
pub(crate) struct AbleSetLibraryCache {
    pub(crate) library_name: Option<String>,
    pub(crate) song_prefix_length: u8,
    pub(crate) entries: HashMap<String, PresentationId>,
    pub(crate) last_updated: Option<DateTime<Utc>>,
    pub(crate) last_error: Option<String>,
    pub(crate) recent_attempts: VecDeque<AbleSetResolutionAttempt>,
    /// Per-number AbleSet<->Presenter title disagreements from the most
    /// recent successful rebuild (#601). Cleared alongside `entries` on
    /// invalidation.
    pub(crate) mismatches: Vec<AbleSetTitleMismatch>,
}

impl AbleSetLibraryCache {
    pub(crate) fn invalidate(&mut self) {
        self.entries.clear();
        self.library_name = None;
        self.song_prefix_length = 0;
        self.last_updated = None;
        self.last_error = None;
        self.mismatches.clear();
    }

    pub(crate) fn matches(&self, library_name: &str, prefix_len: u8) -> bool {
        if let Some(current) = &self.library_name {
            current.eq_ignore_ascii_case(library_name) && self.song_prefix_length == prefix_len
        } else {
            false
        }
    }

    /// Number of resolved entries in the cache (#600 status surface).
    pub(crate) fn cache_size(&self) -> usize {
        self.entries.len()
    }

    /// When the cache was last rebuilt (#600 status surface).
    pub(crate) fn last_updated(&self) -> Option<DateTime<Utc>> {
        self.last_updated
    }

    /// Last error from a cache rebuild, if any (#600 status surface).
    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Read-only view of the recent resolution-attempt ring buffer (#600).
    pub(crate) fn recent_attempts(&self) -> &VecDeque<AbleSetResolutionAttempt> {
        &self.recent_attempts
    }

    /// Read-only view of the current title-mismatch report (#601).
    pub(crate) fn mismatches(&self) -> &[AbleSetTitleMismatch] {
        &self.mismatches
    }

    /// Record a resolution attempt in the ring buffer, evicting the oldest
    /// entry when the cap is reached (FIFO).
    fn record_attempt(&mut self, input: &str, found: bool) {
        if self.recent_attempts.len() >= MAX_RECENT_ATTEMPTS {
            self.recent_attempts.pop_front();
        }
        self.recent_attempts.push_back(AbleSetResolutionAttempt {
            timestamp: Utc::now(),
            input: input.to_string(),
            found,
        });
    }
}

impl AppState {
    pub async fn ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
        self.repository.get_ableset_settings().await
    }

    /// Invalidate the resolved AbleSet song-name cache (#575). Cheap — clears
    /// only the resolved `prefix -> id` map (and the #601 mismatch report,
    /// which is derived from the same data); `ensure_ableset_cache` lazily
    /// rebuilds both from the DB on the next `resolve_ableset_presentation`
    /// call. Call this after ANY mutation that changes which presentations
    /// exist, their names, or which library they belong to. The AbleSet
    /// settings themselves (which library / prefix length to track) are
    /// untouched by these mutations, so a full struct reset is unnecessary —
    /// only the resolved entries can go stale.
    pub(crate) async fn invalidate_ableset_cache(&self) {
        let mut cache = self.caches.ableset.write().await;
        cache.entries.clear();
        cache.mismatches.clear();
    }

    pub async fn update_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AbleSetSettings> {
        let settings = self
            .repository
            .upsert_ableset_settings(&draft, source, actor)
            .await?;
        self.ableset_bridge.apply_settings(settings.clone()).await?;
        {
            let mut cache = self.caches.ableset.write().await;
            cache.invalidate();
            cache.library_name = None;
            cache.song_prefix_length = settings.song_prefix_length;
        }
        Ok(settings)
    }

    /// Build the status snapshot, enriched with library-cache state (#600)
    /// and the title-mismatch report (#601). The bridge contributes the live
    /// AbleSet connection/tracking fields; the cache layer contributes
    /// `cache_size`, `cache_last_updated`, `cache_last_error`, the recent
    /// resolution attempts, and `mismatches`. This merged snapshot is what
    /// `GET /integrations/ableset/status` returns.
    pub async fn ableset_status_snapshot(&self) -> AbleSetStatusSnapshot {
        let mut snapshot = self.ableset_bridge.status_snapshot().await;
        let cache = self.caches.ableset.read().await;
        snapshot.cache_size = Some(cache.cache_size());
        snapshot.cache_last_updated = cache.last_updated();
        snapshot.cache_last_error = cache.last_error().map(str::to_owned);
        snapshot.recent_attempts = cache
            .recent_attempts()
            .iter()
            .map(|a| presenter_core::AbleSetResolutionAttempt {
                timestamp: a.timestamp,
                input: a.input.clone(),
                found: a.found,
            })
            .collect();
        snapshot.mismatches = cache.mismatches().to_vec();
        snapshot
    }

    pub async fn set_ableset_follow(&self, enabled: bool) -> AbleSetStatusSnapshot {
        self.ableset_bridge.set_follow_enabled(enabled).await
    }

    pub async fn current_ableset_song(&self) -> Option<AbleSetSongSnapshot> {
        self.ableset_bridge.song_snapshot().await
    }

    pub async fn resolve_ableset_presentation(
        &self,
        prefix: &str,
    ) -> anyhow::Result<Option<PresentationId>> {
        let key = prefix.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let settings = self.ableset_bridge.status_snapshot().await;
        if !settings.enabled {
            // #623: #600 named this exact early return as the one that
            // "fails EVERY song with no log" — an operator who forgot to
            // (re-)enable the integration otherwise sees only silent
            // no-ops on every incoming song change.
            warn!(
                prefix = key,
                "AbleSet resolution skipped — AbleSet integration is disabled"
            );
            return Ok(None);
        }
        self.ensure_ableset_cache(&settings).await?;
        let lookup = key.to_ascii_lowercase();

        let result = {
            let cache = self.caches.ableset.read().await;
            cache.entries.get(&lookup).copied()
        };

        // Record the attempt in the ring buffer and log misses at WARN (#600).
        // The buffer read-lock is released before acquiring the write-lock to
        // avoid holding both halves of the RwLock simultaneously.
        {
            let mut cache = self.caches.ableset.write().await;
            cache.record_attempt(key, result.is_some());
            if result.is_none() {
                warn!(
                    prefix = key,
                    cache_size = cache.cache_size(),
                    library_name = cache.library_name.as_deref().unwrap_or("?"),
                    last_updated = ?cache.last_updated(),
                    last_error = cache.last_error(),
                    "AbleSet prefix resolution miss — prefix not in cache"
                );
            }
        }

        Ok(result)
    }

    async fn ensure_ableset_cache(&self, settings: &AbleSetStatusSnapshot) -> anyhow::Result<()> {
        let needs_refresh = {
            let cache = self.caches.ableset.read().await;
            !cache.matches(&settings.library_name, settings.song_prefix_length)
                || cache.entries.is_empty()
        };
        if needs_refresh {
            self.refresh_ableset_cache(settings).await?;
        }
        Ok(())
    }

    /// Rebuild the resolved `prefix -> presentation` cache from the DB, plus
    /// the #601 title-mismatch report against the live AbleSet setlist.
    pub(super) async fn refresh_ableset_cache(
        &self,
        settings: &AbleSetStatusSnapshot,
    ) -> anyhow::Result<()> {
        let summaries = self.repository.list_library_summaries(None).await?;
        let target = summaries
            .into_iter()
            .find(|summary| summary.name.eq_ignore_ascii_case(&settings.library_name));

        let (entries, presenter_titles, error) = match target {
            Some(summary) => {
                build_ableset_entries(summary.presentations, settings.song_prefix_length)
            }
            None => {
                // #623: previously silent — only recorded into `last_error`,
                // never logged, even though a missing library means EVERY
                // resolution will miss forever until someone notices.
                warn!(
                    library_name = %settings.library_name,
                    "AbleSet cache rebuild found no library with this name — \
                     resolution will keep missing every song"
                );
                (
                    HashMap::new(),
                    HashMap::new(),
                    Some("library not found".to_string()),
                )
            }
        };

        let ableset_songs = self.ableset_bridge.setlist_song_titles().await;
        let acks = match self.load_ableset_mismatch_acks().await {
            Ok(acks) => acks,
            Err(err) => {
                warn!(
                    ?err,
                    "failed to load AbleSet mismatch acknowledgements — treating as none (#601)"
                );
                super::ableset_mismatch::AckMap::new()
            }
        };
        let mismatches = super::ableset_mismatch::compute_ableset_mismatches(
            &presenter_titles,
            &ableset_songs,
            settings.song_prefix_length,
            &acks,
        );
        for mismatch in &mismatches {
            warn!(
                number = %mismatch.number,
                ableset_title = %mismatch.ableset_title,
                presenter_title = %mismatch.presenter_title,
                "AbleSet/Presenter numbering disagreement — verify before the service (#601)"
            );
        }

        // #623: the rebuild itself was previously silent end-to-end.
        info!(
            library_name = %settings.library_name,
            prefix_length = settings.song_prefix_length,
            entries_found = entries.len(),
            mismatch_count = mismatches.len(),
            "AbleSet cache rebuild complete"
        );

        let mut cache = self.caches.ableset.write().await;
        cache.library_name = Some(settings.library_name.clone());
        cache.song_prefix_length = settings.song_prefix_length;
        cache.entries = entries;
        cache.last_updated = Some(Utc::now());
        cache.last_error = error;
        cache.mismatches = mismatches;
        Ok(())
    }
}

/// Build the resolved `prefix -> presentation` map and the parallel
/// `prefix -> title` map (the latter feeds the #601 mismatch report) from a
/// resolved library's presentations. Returns `error` set when the library
/// has no presentation with a valid prefix.
fn build_ableset_entries(
    presentations: Vec<presenter_core::PresentationSummary>,
    prefix_length: u8,
) -> (
    HashMap<String, PresentationId>,
    HashMap<String, String>,
    Option<String>,
) {
    let mut entries = HashMap::new();
    let mut presenter_titles = HashMap::new();
    for presentation in presentations {
        if let Some(prefix) = extract_song_prefix(&presentation.name, prefix_length) {
            let key = prefix.to_ascii_lowercase();
            presenter_titles.insert(key.clone(), presentation.name.clone());
            entries.insert(key, presentation.id);
        }
    }
    let error = entries
        .is_empty()
        .then_some("no presentations with valid prefix".to_string());
    (entries, presenter_titles, error)
}
