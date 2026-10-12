//! #841: the Bible CLEAR path for one Resolume host: a fade, never a trigger.
//!
//! Owner, 2026-10-11: a clear must not trigger any clip; it only fades the
//! opacity of the Bible clip that is showing. A clear therefore PUTs each Bible
//! lane clip's `video.opacity` parameter from its current value down to 0,
//! stepped over the transition duration of the clip's layer, the time that
//! layer used to crossfade the blank / `#bible-clear` clip in (SNV 0.5 s,
//! PP 0.7 s). Both A and B sides fade: which side is live is unknown after a
//! restart, and fading a disconnected clip shows nothing.
//!
//! Nothing is connected and no text is written, so a clear fills no lane and
//! flips nothing. The next verse lands on the other side, and the layer
//! crossfades it in from an invisible clip. Before a clip is connected,
//! [`HostDriver::restore_before_connect`] puts a faded opacity back.
//!
//! #848 (owner, 2026-10-12): the clip TAGGED `#bible-clear` (the sermon title
//! on SNV) is a cover: a clear fades it UP, a verse fades it DOWN. That fade
//! is deferred and best-effort, NOT part of this push (`bible_clear_cover.rs`).
//!
//! [`TrackedOpacities`] remembers the opacity this process gave every clip it
//! faded or connected:
//!
//! - A clip is tracked BEFORE its first fade PUT, so a fade that fails halfway
//!   is still restored. It stays tracked after the restore: a composition
//!   fetched while it was faded cached 0 (or a mid-fade value), and that cached
//!   number must never override a write made after that fetch.
//! - Every clip a verse connects is remembered with its clip id and layer fade.
//!   After a Resolume deck switch the layer keeps PLAYING that clip, while
//!   `/composition` lists only the new deck, so a clear fades it from what was
//!   remembered.
//! - Each composition fetch shows what Resolume really has, and nothing writes
//!   a Bible clip between a fetch and its sync, so the tracked level then
//!   takes the fetched value. An opacity changed in Arena counts from the
//!   next fetch on.

#[cfg(test)]
use super::bible_clear_cover::{cover_candidates, plan_cover_fade, CoverDirection};
use super::clip_map::ClipMapping;
use super::driver::{duration_ms, HostDriver, ACTION_TIMEOUT};
use super::mapping_refresh::StaleIdError;
use super::types::{ClipTarget, ResolvedEndpoint};
use anyhow::{anyhow, Context};
use futures_util::{stream::FuturesUnordered, StreamExt};
use reqwest::{Client, RequestBuilder, StatusCode};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::{debug, info, warn};

/// Upper bound of a fade. The host worker is serial, so a clear's fade holds
/// every later push of this host for its whole length.
pub(super) const MAX_CLEAR_FADE: Duration = Duration::from_secs(3);
/// Time between two fade steps (20 opacity updates per second). Every step
/// PUTs each clip on a fresh connection (#808 keeps no pool), so the rate
/// stays modest for Arena.
pub(super) const FADE_STEP: Duration = Duration::from_millis(50);
/// Opacities closer than this are the same value (Resolume may round).
pub(super) const SAME_OPACITY: f64 = 1e-6;

/// The opacity this process last gave one clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TrackedOpacity {
    /// The opacity a restore puts back.
    pub(super) base: f64,
    /// The last opacity Resolume accepted from us (or showed in a fetch);
    /// below `base` while faded.
    pub(super) level: f64,
}

impl TrackedOpacity {
    /// This clip once Resolume shows it at `shown` (a fetch, or the cover
    /// fade's read): a VISIBLE clip changed outside Presenter keeps that
    /// opacity as its base, so a dim made in Arena survives; a faded one is
    /// still restored to at least its old base, so 0 never becomes a base.
    pub(super) fn observe(self, shown: f64) -> Self {
        let visible = self.level >= self.base;
        let base = if visible && shown > 0.0 {
            shown
        } else {
            self.base.max(shown)
        };
        Self { base, level: shown }
    }
}

/// A clip a verse connected, as a clear needs it once a deck switch has taken
/// it out of the mapping.
#[derive(Debug, Clone, Copy)]
struct ConnectedClip {
    clip_id: i64,
    fade: Duration,
}

/// Every Bible clip this process has faded or connected, keyed by opacity
/// parameter id. Kept across mapping refetches (the ids survive them, and a
/// deck switch keeps the old deck's clips playing); reset with the host config.
#[derive(Debug, Default)]
pub(super) struct TrackedOpacities {
    clips: HashMap<i64, TrackedOpacity>,
    connected: HashMap<i64, ConnectedClip>,
    /// The mapping fetch (`last_mapping_refresh`) already synced, so a cached
    /// mapping is applied once, not on every push.
    synced_with: Option<Instant>,
}

/// What one [`TrackedOpacities::sync_with_fetch`] changed, for the log.
#[derive(Debug, Default, PartialEq)]
pub(super) struct FetchSync {
    /// Untracked clips the fetch showed at 0, now tracked as faded.
    pub(super) adopted: Vec<i64>,
    /// Tracked clips whose level moved to the fetched value.
    pub(super) corrected: Vec<i64>,
}

impl TrackedOpacities {
    pub(super) fn get(&self, param_id: i64) -> Option<TrackedOpacity> {
        self.clips.get(&param_id).copied()
    }

    /// Track `param_id` at `opacity`, replacing what was tracked.
    pub(super) fn set(&mut self, param_id: i64, opacity: TrackedOpacity) {
        self.clips.insert(param_id, opacity);
    }

    /// Track `param_id` at `opacity` unless it is tracked already. A fade
    /// tracks its clips BEFORE its first PUT, so a fade that fails halfway is
    /// still restored (or continued) from where it stopped.
    pub(super) fn track_if_absent(&mut self, param_id: i64, opacity: TrackedOpacity) {
        self.clips.entry(param_id).or_insert(opacity);
    }

    /// Resolume accepted `level` for a tracked clip.
    pub(super) fn record_level(&mut self, param_id: i64, level: f64) {
        if let Some(tracked) = self.clips.get_mut(&param_id) {
            tracked.level = level;
        }
    }

    /// Apply a fresh composition fetch, once per fetch:
    ///
    /// - an untracked Bible lane clip shown at opacity 0 is adopted as faded
    ///   from full: a clear left it invisible, possibly one of a previous
    ///   process, and it goes back to full opacity before its next connect;
    /// - a tracked clip takes the fetched value as its level (no Bible write
    ///   runs between a fetch and its sync, so a difference was made outside
    ///   Presenter, e.g. in Arena, or by a step that failed halfway), with the
    ///   base rules of [`TrackedOpacity::observe`].
    ///
    /// The `#bible-clear` covers are left out (#848): their deferred fade can
    /// write between a fetch and its sync, so a fetch may cache a value from
    /// the middle of that fade. The cover fade reads each cover's opacity in
    /// Arena before it plans instead (`bible_clear_cover.rs`).
    pub(super) fn sync_with_fetch(
        &mut self,
        mapping: &ClipMapping,
        fetched: Option<Instant>,
    ) -> FetchSync {
        let mut sync = FetchSync::default();
        if fetched.is_some() && self.synced_with == fetched {
            return sync;
        }
        self.synced_with = fetched;
        let mut seen = HashSet::new();
        for opacity in bible_lane_clips(mapping).filter_map(|target| target.opacity) {
            if !seen.insert(opacity.param_id) {
                continue;
            }
            let shown = opacity.base;
            match self.get(opacity.param_id) {
                None if shown <= 0.0 => {
                    let faded = TrackedOpacity {
                        base: 1.0,
                        level: 0.0,
                    };
                    self.clips.insert(opacity.param_id, faded);
                    sync.adopted.push(opacity.param_id);
                }
                Some(tracked) if (tracked.level - shown).abs() > SAME_OPACITY => {
                    self.clips.insert(opacity.param_id, tracked.observe(shown));
                    sync.corrected.push(opacity.param_id);
                }
                _ => {}
            }
        }
        sync
    }

    /// Stop tracking a clip Resolume no longer knows.
    pub(super) fn forget(&mut self, param_id: i64) {
        self.clips.remove(&param_id);
        self.connected.remove(&param_id);
    }
}

/// One clip of a fade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FadeTarget {
    pub(super) clip_id: i64,
    pub(super) param_id: i64,
    /// The opacity a restore puts back.
    pub(super) base: f64,
    /// The opacity the fade starts from.
    pub(super) from: f64,
    /// The opacity the fade ends at: 0 for a Bible clip and a cover a verse
    /// hides, the base for a `#bible-clear` cover a clear shows (#848).
    pub(super) to: f64,
    pub(super) duration: Duration,
    /// Listed by the current mapping. A clip outside it (a verse's clip or a
    /// cover from before a deck switch) that answers 404 is dropped; a mapped
    /// one is a stale mapping.
    pub(super) mapped: bool,
}

/// Every Bible lane clip of `mapping`, A and B sides.
pub(super) fn bible_lane_clips(mapping: &ClipMapping) -> impl Iterator<Item = &ClipTarget> {
    [
        &mapping.bible_a,
        &mapping.bible_b,
        &mapping.bible_reference_a,
        &mapping.bible_reference_b,
        &mapping.bible_translation_a,
        &mapping.bible_translation_b,
        &mapping.bible_translate_reference_a,
        &mapping.bible_translate_reference_b,
    ]
    .into_iter()
    .flatten()
}

/// The fade a clear runs: each still-visible Bible lane clip with an opacity
/// parameter, from where it is now (the tracked level, else the mapped value),
/// then each still-visible clip a verse connected that the mapping no longer
/// lists, all down to 0. A clip tagged twice fades once. The `#bible-clear`
/// covers are not part of it: their fade is deferred (#848,
/// `bible_clear_cover.rs`). Pure, so the selection is unit-tested without a
/// mock Resolume.
pub(super) fn plan_clear_fade(
    mapping: &ClipMapping,
    tracked: &TrackedOpacities,
) -> Vec<FadeTarget> {
    let mut seen = HashSet::new();
    let mut plan: Vec<FadeTarget> = bible_lane_clips(mapping)
        .filter_map(|target| {
            let opacity = target.opacity?;
            if !seen.insert(opacity.param_id) {
                return None;
            }
            let (base, from) = match tracked.get(opacity.param_id) {
                Some(clip) => (clip.base, clip.level),
                None => (opacity.base, opacity.base),
            };
            (from > 0.0).then_some(FadeTarget {
                clip_id: target.clip_id,
                param_id: opacity.param_id,
                base,
                from,
                to: 0.0,
                duration: target.fade,
                mapped: true,
            })
        })
        .collect();
    let mut unmapped: Vec<FadeTarget> = tracked
        .connected
        .iter()
        .filter(|(param_id, _)| !seen.contains(*param_id))
        .filter_map(|(&param_id, clip)| {
            let opacity = tracked.get(param_id)?;
            (opacity.level > 0.0).then_some(FadeTarget {
                clip_id: clip.clip_id,
                param_id,
                base: opacity.base,
                from: opacity.level,
                to: 0.0,
                duration: clip.fade,
                mapped: false,
            })
        })
        .collect();
    unmapped.sort_unstable_by_key(|target| target.param_id);
    plan.extend(unmapped);
    plan
}

/// #848: the hide a verse schedules, planned from the mapping and the tracked
/// opacities alone (the deferred fade also reads Arena first and adds the
/// covers it remembers): each `#bible-clear` cover that is not invisible,
/// down to 0, none that is also a Bible lane clip.
#[cfg(test)]
pub(super) fn plan_cover_hide(
    mapping: &ClipMapping,
    tracked: &TrackedOpacities,
) -> Vec<FadeTarget> {
    let covers = cover_candidates(mapping, &HashMap::new());
    plan_cover_fade(&covers, tracked, CoverDirection::Hide)
}

/// The opacity of a fading clip `elapsed` into its fade: linear from `from`
/// down to 0, reached exactly at `duration` (at once for a zero duration).
#[cfg(test)]
pub(super) fn fade_level(from: f64, elapsed: Duration, duration: Duration) -> f64 {
    fade_toward(from, 0.0, elapsed, duration)
}

/// The opacity of a fading clip `elapsed` into its fade: linear from `from`
/// to `to` (up or down), reached exactly at `duration` (at once for a zero
/// duration).
pub(super) fn fade_toward(from: f64, to: f64, elapsed: Duration, duration: Duration) -> f64 {
    if elapsed >= duration {
        return to;
    }
    let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
    from + (to - from) * progress
}

/// The longest fade of `fade` in milliseconds, for the logs.
fn longest_ms(fade: &[FadeTarget]) -> f64 {
    fade.iter()
        .map(|target| duration_ms(target.duration))
        .fold(0.0, f64::max)
}

impl HostDriver {
    /// Sync the tracked opacities with the cached mapping's fetch (once per
    /// fetch) and log what changed.
    pub(super) fn sync_opacities(&mut self, mapping: &ClipMapping) {
        let sync = self
            .opacities
            .sync_with_fetch(mapping, self.last_mapping_refresh);
        if !sync.adopted.is_empty() {
            info!(
                host = %self.config.host,
                adopted = ?sync.adopted,
                "resolume bible: Bible clips found at opacity 0 count as faded; the next verse on them shows at full opacity"
            );
        }
        if !sync.corrected.is_empty() {
            info!(
                host = %self.config.host,
                corrected = ?sync.corrected,
                "resolume bible: tracked opacities follow what the composition shows"
            );
        }
    }

    /// Fade every visible Bible clip of this host to opacity 0 without
    /// triggering a clip or writing text (see the module docs). The
    /// `#bible-clear` cover is shown afterwards, deferred
    /// (`HostDriver::schedule_cover_fade`).
    pub(super) async fn handle_bible_clear(&mut self, mapping: &ClipMapping) -> anyhow::Result<()> {
        self.warn_unfadeable(mapping);
        let fade = plan_clear_fade(mapping, &self.opacities);
        if fade.is_empty() {
            debug!(
                host = %self.config.host,
                "resolume bible clear: no visible Bible clip, nothing to fade"
            );
            return Ok(());
        }
        let fade_ms = longest_ms(&fade);
        let faded_clip_ids: Vec<i64> = fade.iter().map(|target| target.clip_id).collect();
        let unmapped_clip_ids: Vec<i64> = fade
            .iter()
            .filter(|target| !target.mapped)
            .map(|target| target.clip_id)
            .collect();
        let start = Instant::now();
        let mut steps = 0;
        let result = self.run_fade(&fade, &mut steps).await;
        info!(
            host = %self.config.host,
            ?faded_clip_ids,
            ?unmapped_clip_ids,
            fade_ms,
            steps,
            t_ms = duration_ms(start.elapsed()),
            ok = result.is_ok(),
            "resolume bible clear: faded the Bible clips' opacity to 0, no clip triggered"
        );
        result
    }

    /// A Bible clip without a `video.opacity` parameter cannot be faded, so its
    /// text stays on the wall after a clear; a `#bible-clear` clip without one
    /// is never shown or hidden.
    fn warn_unfadeable(&self, mapping: &ClipMapping) {
        let unfadeable: Vec<i64> = bible_lane_clips(mapping)
            .chain(&mapping.bible_clear)
            .filter(|target| target.opacity.is_none())
            .map(|target| target.clip_id)
            .collect();
        if !unfadeable.is_empty() {
            warn!(
                host = %self.config.host,
                ?unfadeable,
                "resolume bible clear: these Bible / #bible-clear clips have no video.opacity parameter and cannot be faded"
            );
        }
    }

    /// Step every target's opacity from where it is to its `to` over its
    /// duration. `steps` counts the steps that landed, and stays right when a
    /// later step fails.
    async fn run_fade(&mut self, fade: &[FadeTarget], steps: &mut usize) -> anyhow::Result<()> {
        // Tracked BEFORE the first PUT: a fade that fails halfway must still
        // be restored before these clips are connected again.
        for target in fade {
            let start = TrackedOpacity {
                base: target.base,
                level: target.from,
            };
            self.opacities.track_if_absent(target.param_id, start);
        }
        let longest = fade
            .iter()
            .map(|target| target.duration)
            .max()
            .unwrap_or_default();
        let endpoint = self.endpoint().await?;
        let start = Instant::now();
        loop {
            let elapsed = start.elapsed();
            let levels: Vec<(i64, f64)> = fade
                .iter()
                .filter(|target| {
                    self.opacities
                        .get(target.param_id)
                        .is_some_and(|tracked| (tracked.level - target.to).abs() > SAME_OPACITY)
                })
                .map(|target| {
                    let level = fade_toward(target.from, target.to, elapsed, target.duration);
                    (target.param_id, level)
                })
                .collect();
            if levels.is_empty() {
                return Ok(());
            }
            let puts = put_opacities(&self.client, &endpoint, &levels).await;
            // Record every PUT that landed, also in a failed step, so the
            // tracked levels match what Arena has.
            for (param_id, level) in levels {
                if puts.landed.contains(&param_id) {
                    self.opacities.record_level(param_id, level);
                }
            }
            let stale = self.drop_missing(fade, &puts.missing);
            if let Some(err) = puts.error {
                return Err(err);
            }
            *steps += 1;
            stale?;
            // Past the longest duration every level just sent was its `to`.
            if elapsed >= longest {
                return Ok(());
            }
            sleep(FADE_STEP).await;
        }
    }

    /// Handle the opacity parameters a fade step got a 404 for. A clip outside
    /// the mapping (a verse's clip from before a deck switch) is gone, so it is
    /// forgotten and the fade goes on without it. A mapped one means the
    /// mapping is stale (#808): refetch and retry.
    fn drop_missing(&mut self, fade: &[FadeTarget], missing: &[i64]) -> anyhow::Result<()> {
        let mut stale = Vec::new();
        for &param_id in missing {
            match fade.iter().find(|target| target.param_id == param_id) {
                Some(target) if !target.mapped => {
                    self.opacities.forget(param_id);
                    warn!(
                        host = %self.config.host,
                        clip_id = target.clip_id,
                        param_id,
                        "resolume bible clear: a clip from outside the mapping is gone (404), no longer fading it"
                    );
                }
                _ => stale.push(param_id),
            }
        }
        match stale.first() {
            Some(&param_id) => Err(StaleIdError::opacity_parameter(param_id)),
            None => Ok(()),
        }
    }

    /// Before `targets` are connected (their text is already written, and the
    /// A/B side being connected is not live): remember each clip, so a clear
    /// still fades it after a deck switch, and put back the base opacity of
    /// every clip a clear lowered. A clip stays tracked at its base, so a later
    /// fade starts from there.
    pub(super) async fn restore_before_connect(
        &mut self,
        targets: &[ClipTarget],
    ) -> anyhow::Result<()> {
        let mut seen = HashSet::new();
        let mut restores: Vec<(i64, f64)> = Vec::new();
        for target in targets {
            let Some(opacity) = target.opacity else {
                continue;
            };
            if !seen.insert(opacity.param_id) {
                continue;
            }
            let connected = ConnectedClip {
                clip_id: target.clip_id,
                fade: target.fade,
            };
            self.opacities.connected.insert(opacity.param_id, connected);
            let base = if opacity.base > 0.0 {
                opacity.base
            } else {
                1.0
            };
            let tracked = *self
                .opacities
                .clips
                .entry(opacity.param_id)
                .or_insert(TrackedOpacity {
                    base,
                    level: opacity.base,
                });
            if tracked.level < tracked.base {
                restores.push((opacity.param_id, tracked.base));
            }
        }
        if restores.is_empty() {
            return Ok(());
        }
        let endpoint = self.endpoint().await?;
        let puts = put_opacities(&self.client, &endpoint, &restores).await;
        for &(param_id, base) in &restores {
            if !puts.landed.contains(&param_id) {
                continue;
            }
            if let Some(tracked) = self.opacities.clips.get_mut(&param_id) {
                tracked.level = base;
            }
        }
        if let Some(err) = puts.error {
            return Err(err);
        }
        if let Some(&param_id) = puts.missing.first() {
            return Err(StaleIdError::opacity_parameter(param_id));
        }
        debug!(
            host = %self.config.host,
            restored = ?restores,
            "resolume bible: restored the opacity of faded clips before connecting them"
        );
        Ok(())
    }
}

/// What one batch of opacity PUTs did. Every request of the batch is awaited,
/// also after a failure (`batch.rs`).
#[derive(Debug, Default)]
pub(super) struct OpacityPuts {
    /// Accepted by Resolume.
    pub(super) landed: Vec<i64>,
    /// Answered 404; the caller decides what a missing clip means.
    pub(super) missing: Vec<i64>,
    /// The first other failure, in completion order.
    pub(super) error: Option<anyhow::Error>,
}

/// PUT each `(opacity param id, value)` concurrently and wait for all of them.
pub(super) async fn put_opacities(
    client: &Client,
    endpoint: &ResolvedEndpoint,
    levels: &[(i64, f64)],
) -> OpacityPuts {
    let futures = FuturesUnordered::new();
    for &(param_id, value) in levels {
        let url = format!("{}/parameter/by-id/{}", endpoint.base_url, param_id);
        let mut request = client.put(&url);
        if let Some(host) = &endpoint.host_header {
            request = request.header(reqwest::header::HOST, host.clone());
        }
        futures.push(async move { (param_id, put_opacity(request, param_id, value).await) });
    }
    let mut puts = OpacityPuts::default();
    let outcomes: Vec<(i64, anyhow::Result<bool>)> = futures.collect().await;
    for (param_id, outcome) in outcomes {
        match outcome {
            Ok(true) => puts.landed.push(param_id),
            Ok(false) => puts.missing.push(param_id),
            Err(err) => {
                if puts.error.is_none() {
                    puts.error = Some(err);
                }
            }
        }
    }
    puts
}

/// One opacity PUT: `Ok(true)` when Resolume accepted it, `Ok(false)` on 404.
async fn put_opacity(request: RequestBuilder, param_id: i64, value: f64) -> anyhow::Result<bool> {
    let response = request
        .json(&serde_json::json!({ "value": value }))
        .timeout(ACTION_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("failed to update opacity parameter {param_id}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(false);
    }
    if !response.status().is_success() {
        return Err(anyhow!(
            "opacity parameter update failed with status {}",
            response.status()
        ));
    }
    Ok(true)
}
