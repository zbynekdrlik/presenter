use chrono::{DateTime, Utc};
use presenter_core::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSongSnapshot,
    AbleSetTitleMismatch, PresentationId,
};
use std::collections::{HashMap, VecDeque};
use tracing::{info, warn};

use super::ableset_ack::AckMap;
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
    /// Bumped on every invalidation (#639 TOCTOU fix). A refresh captures
    /// this BEFORE its async DB fetch and re-checks it, under the same write
    /// lock as the install, before writing the fetched data back. A mismatch
    /// means a concurrent invalidate raced the fetch — see
    /// `install_if_current`.
    pub(crate) generation: u64,
    /// Per-number AbleSet<->Presenter title disagreements from the most
    /// recent successful rebuild (#601). Cleared alongside `entries` on
    /// invalidation — see `install_if_current` and the invalidate methods.
    pub(crate) mismatches: Vec<AbleSetTitleMismatch>,
    /// `prefix -> full presentation name` from the most recent successful
    /// rebuild (#655 F5c/F8) — lets an ack/unack recompute the mismatch
    /// report WITHOUT a full DB rescan, and lets `refresh_ableset_cache`
    /// avoid re-fetching it. Cleared alongside `entries` on invalidation.
    pub(crate) presenter_titles: HashMap<String, String>,
    /// Mirror of the persisted AbleSet mismatch-acknowledgement store (#655
    /// F8) — updated ONLY by `acknowledge_ableset_mismatch` /
    /// `unacknowledge_ableset_mismatch` / the F9b prune, never reloaded from
    /// the DB inside a normal `refresh_ableset_cache` rebuild. This is what
    /// narrows the #639 CAS window down to the one remaining unavoidable DB
    /// fetch (the library summaries). Survives `invalidate`/
    /// `invalidate_entries` — an ack is keyed on song NUMBER, not on which
    /// library is currently tracked.
    pub(crate) acks: AckMap,
}

impl AbleSetLibraryCache {
    pub(crate) fn invalidate(&mut self) {
        self.entries.clear();
        self.library_name = None;
        self.song_prefix_length = 0;
        self.last_updated = None;
        self.last_error = None;
        self.mismatches.clear();
        self.presenter_titles.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Clear only the resolved entries, keeping library/prefix identity
    /// (#575) — used after a local mutation that can change which
    /// presentations exist without touching AbleSet settings. Bumps
    /// `generation` (#639) so a refresh already past its async DB fetch
    /// detects this race and discards its stale result instead of
    /// resurrecting what this call just cleared.
    pub(crate) fn invalidate_entries(&mut self) {
        self.entries.clear();
        self.mismatches.clear();
        self.presenter_titles.clear();
        self.generation = self.generation.wrapping_add(1);
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

    /// Read-only view of the `prefix -> title` map from the last successful
    /// rebuild (#655 F5c).
    pub(crate) fn presenter_titles(&self) -> &HashMap<String, String> {
        &self.presenter_titles
    }

    /// Read-only view of the cached acknowledgement-store mirror (#655 F8).
    pub(crate) fn acks(&self) -> &AckMap {
        &self.acks
    }

    /// Current generation counter (#639) — captured by a refresh BEFORE its
    /// async DB fetch, so its eventual install can detect a concurrent
    /// invalidate.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Install a freshly-fetched refresh, but only if `expected_generation`
    /// still matches the current generation (#639 compare-and-swap). A
    /// mismatch means a concurrent invalidate (a settings change or a local
    /// mutation) landed between the caller's DB read and this install — the
    /// fetched data may already be stale relative to that invalidation, so
    /// it is discarded rather than overwriting the cache. The next
    /// `ensure_ableset_cache` call sees `entries` still empty (the
    /// invalidate's own effect, left untouched) and retries cleanly.
    /// Returns whether the install happened.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install_if_current(
        &mut self,
        expected_generation: u64,
        library_name: String,
        prefix_len: u8,
        entries: HashMap<String, PresentationId>,
        error: Option<String>,
        mismatches: Vec<AbleSetTitleMismatch>,
        presenter_titles: HashMap<String, String>,
    ) -> bool {
        if self.generation != expected_generation {
            return false;
        }
        self.library_name = Some(library_name);
        self.song_prefix_length = prefix_len;
        self.entries = entries;
        self.last_updated = Some(Utc::now());
        self.last_error = error;
        self.mismatches = mismatches;
        self.presenter_titles = presenter_titles;
        true
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
    /// only the resolved `prefix -> id` map; `ensure_ableset_cache` lazily
    /// rebuilds it from the DB on the next `resolve_ableset_presentation`
    /// call. Call this after ANY mutation that changes which presentations
    /// exist, their names, or which library they belong to. The AbleSet
    /// settings themselves (which library / prefix length to track) are
    /// untouched by these mutations, so a full struct reset is unnecessary —
    /// only the resolved entries can go stale.
    pub(crate) async fn invalidate_ableset_cache(&self) {
        self.caches.ableset.write().await.invalidate_entries();
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
        let (mismatches, mismatch_count) = cap_mismatches_for_status(cache.mismatches());
        snapshot.mismatches = mismatches;
        snapshot.mismatch_count = mismatch_count;
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
    /// the #601 title-mismatch report, and install both atomically under a
    /// #639 compare-and-swap guard against a concurrent invalidate.
    pub(super) async fn refresh_ableset_cache(
        &self,
        settings: &AbleSetStatusSnapshot,
    ) -> anyhow::Result<()> {
        let expected_generation = self.caches.ableset.read().await.generation();
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
        // #655 F8: read the cached ack mirror instead of the DB — the mirror
        // is kept current by acknowledge/unacknowledge/prune, and NOT
        // reloading it here removes a DB await that used to sit inside the
        // #639 CAS window (between the generation capture above and the
        // install below), narrowing that window to just the one remaining
        // unavoidable fetch (`list_library_summaries`).
        let acks = self.caches.ableset.read().await.acks().clone();
        let mismatches = super::ableset_mismatch::compute_ableset_mismatches(
            &presenter_titles,
            &ableset_songs,
            settings.song_prefix_length,
            &acks,
        );

        // #655 F9b: prune acks no longer present on EITHER side — only when
        // both sides are non-empty (a real comparison), so an empty/disabled
        // tick never wrongly prunes acks for numbers that simply weren't
        // compared this round.
        if !presenter_titles.is_empty() && !ableset_songs.is_empty() {
            self.prune_stale_ableset_acks(&presenter_titles, &ableset_songs)
                .await;
        }

        let entries_found = entries.len();
        let mut cache = self.caches.ableset.write().await;
        let installed = cache.install_if_current(
            expected_generation,
            settings.library_name.clone(),
            settings.song_prefix_length,
            entries,
            error,
            mismatches,
            presenter_titles,
        );
        log_ableset_rebuild_result(
            installed,
            &settings.library_name,
            settings.song_prefix_length,
            entries_found,
            cache.mismatches(),
        );
        Ok(())
    }
}

/// Log the outcome of a `refresh_ableset_cache` install — extracted to keep
/// that function under the repo's per-function line cap. `installed = false`
/// is the #639 race-lost path (a concurrent invalidate won); `true` is the
/// normal path, which also emits the #601 per-mismatch WARN and #623's
/// previously-missing cache-rebuild INFO.
fn log_ableset_rebuild_result(
    installed: bool,
    library_name: &str,
    prefix_length: u8,
    entries_found: usize,
    mismatches: &[AbleSetTitleMismatch],
) {
    if !installed {
        // #639: the cache is already whatever the racing invalidate left it
        // as (entries/mismatches cleared) — discarding here, instead of
        // overwriting, is what stops the invalidate from being silently
        // lost. The next `ensure_ableset_cache` call sees `entries.is_empty()`
        // and retries without racing.
        warn!(
            library_name = library_name,
            "AbleSet cache rebuild lost a race with a concurrent invalidate (#639) — \
             discarding stale fetch; the next resolution attempt will retry cleanly"
        );
        return;
    }
    // #623: the rebuild itself was previously silent end-to-end.
    info!(
        library_name = library_name,
        prefix_length = prefix_length,
        entries_found = entries_found,
        mismatch_count = mismatches.len(),
        "AbleSet cache rebuild complete"
    );
    for mismatch in mismatches {
        warn!(
            number = %mismatch.number,
            ableset_title = %mismatch.ableset_title,
            presenter_title = %mismatch.presenter_title,
            "AbleSet/Presenter numbering disagreement — verify before the service (#601)"
        );
    }
}

/// Build the resolved `prefix -> presentation` map and the parallel
/// `prefix -> title` map (the latter feeds the #601 mismatch report) from a
/// resolved library's presentations. Extracted from `refresh_ableset_cache`
/// to keep that function under the repo's per-function line cap. Returns
/// `error` set when the library has no presentation with a valid prefix.
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

/// Cap the mismatch list surfaced on the 5s-polled `GET
/// /integrations/ableset/status` endpoint at 25 entries (#655 F15) — an
/// unbounded list scales with the size of the pre-service checklist gap,
/// which can briefly be large right after a library switch. Returns the
/// capped slice (the FIRST 25, stable order) plus the TRUE total count so
/// the operator can tell "showing 25 of N".
fn cap_mismatches_for_status(
    mismatches: &[AbleSetTitleMismatch],
) -> (Vec<AbleSetTitleMismatch>, usize) {
    const MAX_STATUS_MISMATCHES: usize = 25;
    let total = mismatches.len();
    let capped = mismatches
        .iter()
        .take(MAX_STATUS_MISMATCHES)
        .cloned()
        .collect();
    (capped, total)
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    // #655 F15 — RED (this commit): `cap_mismatches_for_status` does not
    // exist yet, so this fails to compile — the established pattern in this
    // codebase for a new API being introduced (see the #601 strip_song_prefix
    // precedent in presenter-core). GREEN adds the function and wires it
    // into `ableset_status_snapshot`.
    #[test]
    fn cap_mismatches_for_status_caps_at_25_and_reports_total_count() {
        let mismatches: Vec<AbleSetTitleMismatch> = (0..30)
            .map(|i| AbleSetTitleMismatch {
                number: format!("{i:03}"),
                ableset_title: format!("AbleSet {i}"),
                presenter_title: format!("Presenter {i}"),
            })
            .collect();

        let (capped, total) = cap_mismatches_for_status(&mismatches);

        assert_eq!(
            capped.len(),
            25,
            "a 5s-polled status endpoint must cap the inline mismatch list at 25"
        );
        assert_eq!(
            total, 30,
            "the total count must reflect ALL mismatches, not just the capped slice"
        );
        assert_eq!(
            capped[0].number, "000",
            "the capped slice must be the FIRST 25, not an arbitrary subset"
        );
    }

    #[test]
    fn cap_mismatches_for_status_does_not_truncate_under_the_cap() {
        let mismatches = vec![AbleSetTitleMismatch {
            number: "001".to_string(),
            ableset_title: "A".to_string(),
            presenter_title: "B".to_string(),
        }];
        let (capped, total) = cap_mismatches_for_status(&mismatches);
        assert_eq!(capped.len(), 1);
        assert_eq!(total, 1);
    }

    // #639 — RED (this commit): `generation`/`install_if_current` prove the
    // compare-and-swap that closes the TOCTOU window described in the issue
    // ("An invalidate request that lands in the window between the read and
    // the install is silently lost — the installed (stale) cache overwrites
    // the invalidation"). GREEN wires them into `refresh_ableset_cache`.

    #[test]
    fn install_if_current_rejects_a_stale_generation() {
        let mut cache = AbleSetLibraryCache::default();
        let captured_generation = cache.generation();

        // Simulate a concurrent invalidate landing WHILE a refresh's async DB
        // fetch was in flight — exactly the #639 race window.
        cache.invalidate_entries();

        let installed = cache.install_if_current(
            captured_generation,
            "Hymnal".to_string(),
            3,
            HashMap::from([("001".to_string(), PresentationId::new())]),
            None,
            Vec::new(),
            HashMap::new(),
        );

        assert!(
            !installed,
            "a refresh whose generation went stale during its fetch must be rejected, \
             not silently overwrite what the concurrent invalidate just cleared"
        );
        assert!(
            cache.entries.is_empty(),
            "the rejected install must leave the cache exactly as the invalidate left it"
        );
    }

    #[test]
    fn install_if_current_succeeds_when_generation_is_unchanged() {
        let mut cache = AbleSetLibraryCache::default();
        let captured_generation = cache.generation();
        let id = PresentationId::new();

        let installed = cache.install_if_current(
            captured_generation,
            "Hymnal".to_string(),
            3,
            HashMap::from([("001".to_string(), id)]),
            None,
            Vec::new(),
            HashMap::new(),
        );

        assert!(
            installed,
            "a refresh with no concurrent invalidate must install normally"
        );
        assert_eq!(cache.entries.get("001"), Some(&id));
    }

    #[test]
    fn invalidate_entries_bumps_generation() {
        let mut cache = AbleSetLibraryCache::default();
        let before = cache.generation();
        cache.invalidate_entries();
        assert_ne!(
            before,
            cache.generation(),
            "invalidate_entries must bump the generation counter so an in-flight \
             refresh detects the race (#639)"
        );
    }

    // #655 F5a — RED (this commit): `recently_failed_rebuild` does not exist
    // yet. GREEN adds it and consults it from `ensure_ableset_cache`'s
    // `entries.is_empty()` retry escape hatch.

    #[test]
    fn recently_failed_rebuild_is_false_with_no_prior_attempt() {
        let cache = AbleSetLibraryCache::default();
        assert!(!cache.recently_failed_rebuild());
    }

    #[test]
    fn recently_failed_rebuild_is_true_immediately_after_a_failed_attempt() {
        let mut cache = AbleSetLibraryCache::default();
        cache.last_failed_rebuild_attempt = Some(Utc::now());
        assert!(cache.recently_failed_rebuild());
    }

    #[test]
    fn recently_failed_rebuild_expires_after_the_cooldown() {
        let mut cache = AbleSetLibraryCache::default();
        cache.last_failed_rebuild_attempt = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(
            !cache.recently_failed_rebuild(),
            "a failed attempt older than the cooldown must no longer suppress a retry"
        );
    }

    // #655 F5b — RED (this commit): `summarize_mismatches` does not exist
    // yet. GREEN adds it and wires it into `refresh_ableset_cache`'s
    // logging, replacing the old per-mismatch WARN loop.

    #[test]
    fn summarize_mismatches_splits_gaps_from_title_disagreements() {
        let mismatches = vec![
            AbleSetTitleMismatch {
                number: "001".to_string(),
                ableset_title: "A".to_string(),
                presenter_title: String::new(), // gap
            },
            AbleSetTitleMismatch {
                number: "002".to_string(),
                ableset_title: String::new(), // gap
                presenter_title: "B".to_string(),
            },
            AbleSetTitleMismatch {
                number: "003".to_string(),
                ableset_title: "C".to_string(),
                presenter_title: "D".to_string(), // genuine disagreement
            },
        ];
        let summary = summarize_mismatches(&mismatches);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.gaps, 2);
        assert_eq!(summary.title_disagreements, 1);
        assert_eq!(summary.sample.len(), 3);
    }

    #[test]
    fn summarize_mismatches_bounds_the_sample_to_five() {
        let mismatches: Vec<AbleSetTitleMismatch> = (0..10)
            .map(|i| AbleSetTitleMismatch {
                number: format!("{i:03}"),
                ableset_title: "A".to_string(),
                presenter_title: "B".to_string(),
            })
            .collect();
        let summary = summarize_mismatches(&mismatches);
        assert_eq!(summary.total, 10);
        assert_eq!(summary.sample.len(), 5, "the log sample must cap at 5");
    }
}
