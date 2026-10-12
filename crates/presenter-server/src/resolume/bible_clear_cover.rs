//! #848: the `#bible-clear` COVER fade: deferred, best-effort, interruptible.
//!
//! Owner, 2026-10-12: "ked sa clearne v presentery vers alebo je clearnuty tak
//! ma sa zapnut opacita bible clear a ked sa zapne vers tak sa ma stiahnut
//! opacita bible clear aby bol vers vyditelny". A Bible clear fades the clip
//! tagged `#bible-clear` (the sermon title on SNV) UP to its opacity, a verse
//! fades it DOWN to 0. Nothing is triggered; only `video.opacity` is PUT.
//!
//! The cover fades over its own layer's transition (SNV layer 12: 2.3 s), and
//! the host worker is serial. Inside the push that froze the `#timer`
//! countdown and held the next verse for as long (#848 review), so the fade is
//! a step machine the worker runs between its commands:
//!
//! - A clear whose Bible fade succeeded schedules a SHOW; a connected verse
//!   schedules a HIDE after its connects and lane flips
//!   ([`HostDriver::schedule_cover_fade`]). A new schedule replaces the
//!   pending fade.
//! - `run_host_worker`'s biased `select!` calls
//!   [`HostDriver::step_cover_fade`] at [`HostDriver::cover_fade_due`], after
//!   its commands branch: every command that waits runs before the next
//!   50 ms step.
//! - The first step reads each cover's opacity in Arena
//!   (`GET /parameter/by-id/{id}`, a `RangeParameter` whose `value` is the
//!   opacity) and plans from it: a change made in Arena counts, a fade that a
//!   replacement cut short goes on from where it got, and a cover already at
//!   its target gets nothing. It PUTs nothing, except on a layer without a
//!   transition, where it sets the target. A failed read keeps the tracked
//!   value.
//! - Each cover fades over the share of its layer's transition that the
//!   distance left is of its base (`fade × |to − from| / base`), so a hide
//!   right after a short show is short too (#848 second review).
//! - It is best-effort. An error never fails a push and never touches the host
//!   status or the #484 backoff. A failed step jumps to ONE final step that
//!   PUTs each target `FADE_STEP` later; if that fails too, one WARN and the
//!   fade stops: at most 2 failed PUTs per cover, plus the first step's read.
//! - A cover answering 404 is left out of the fade and the others go on
//!   (#848 third review). A mapped one invalidates the mapping as `stale-id`,
//!   at most once per fetched mapping and once per id, so a cover that stays
//!   broken cannot pull the 16 MB composition on every clear and verse; its
//!   later 404s WARN through the #563h limiter. A remembered one is forgotten.
//! - Covers are remembered across a deck switch (like the clips a verse
//!   connected, #841), a small set per layer (`bible_clear_cover_memory.rs`):
//!   the old deck's cover still plays on its layer, so later fades include
//!   it.

use super::bible_clear::{
    fade_toward, put_opacities, FadeTarget, TrackedOpacities, TrackedOpacity, FADE_STEP,
    MAX_CLEAR_FADE, SAME_OPACITY,
};
// Re-exported: the planner tests import `cover_candidates` from here.
pub(super) use super::bible_clear_cover_memory::cover_candidates;
use super::bible_clear_cover_memory::{
    forget_remembered_cover, mapped_covers, remember_cover, RememberedCovers,
};
use super::clip_map::ClipMapping;
use super::driver::{
    duration_ms, FetchReason, HostDriver, ACTION_TIMEOUT, MISSING_CLIP_WARN_INTERVAL,
};
use super::types::ResolvedEndpoint;
use anyhow::{anyhow, Context};
use futures_util::future::join_all;
use reqwest::{header::HOST, Client, StatusCode};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, info, warn};

/// Which way a cover fade goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoverDirection {
    /// After a clear: up to the cover's base opacity.
    Show,
    /// After a verse: down to 0.
    Hide,
}

impl CoverDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Hide => "hide",
        }
    }
}

/// One `#bible-clear` clip as a cover fade needs it, also after a deck switch
/// took it out of the mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CoverClip {
    pub(super) clip_id: i64,
    pub(super) param_id: i64,
    /// The opacity its mapping showed.
    pub(super) mapped_base: f64,
    /// Its layer's transition.
    pub(super) fade: Duration,
    /// Its layer (1-based): the key it is remembered by.
    pub(super) layer: usize,
    /// Playing when its mapping was fetched.
    pub(super) connected: bool,
    /// Listed by the mapping the fade was scheduled with. An unlisted one
    /// answering 404 is forgotten; a listed one means a stale mapping.
    pub(super) mapped: bool,
}

/// A scheduled cover fade.
#[derive(Debug)]
pub(super) struct CoverFade {
    direction: CoverDirection,
    covers: Vec<CoverClip>,
    /// `last_mapping_refresh` of the mapping it was scheduled with.
    mapping_fetched: Option<Instant>,
    /// Planned by the first step, from the opacities Arena reported.
    targets: Vec<FadeTarget>,
    /// When the first step ran; `None` before.
    started: Option<Instant>,
    /// When the worker runs the next step.
    due: Instant,
    /// Steps whose PUTs went out without an error.
    steps: usize,
    /// The error of a step that failed: the next step is the final one, at
    /// each target. `None` while no step failed.
    failed_step: Option<String>,
    /// Clip ids of the remembered covers this fade found gone (404).
    forgotten: Vec<i64>,
    /// Clip ids of the mapped covers this fade left out (404: stale mapping).
    stale: Vec<i64>,
}

/// One host's cover fade state; reset with its config.
#[derive(Debug, Default)]
pub(super) struct CoverState {
    fade: Option<CoverFade>,
    /// A small set of covers per layer (`bible_clear_cover_memory.rs`).
    remembered: RememberedCovers,
    /// Every mapped cover id whose 404 refetched the mapping (until a config
    /// change): the refetched mapping lists it again, so it never refetches
    /// again, and two broken ids cannot take turns refetching on every fade.
    refetched_ids: HashSet<i64>,
    /// The mapping the last cover refetch was scheduled with.
    refetched_mapping: Option<Instant>,
    /// #563h: when a mapped cover's 404 that refetched nothing last WARNed,
    /// per opacity parameter id (the `cover-404:<id>` key).
    warned_404: HashMap<i64, Instant>,
}

/// How one step ended. A cover answering 404 does not end it: it is left out
/// of the fade and the others go on.
enum CoverStep {
    Continue,
    Done,
    Failed(anyhow::Error),
}

/// What a mapped cover's 404 does to the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoverRefetch {
    /// Invalidate it as `stale-id`; the next push or tick refetches it.
    Refetch,
    /// The fade was scheduled with another mapping than the cached one.
    MappingChanged,
    /// This id refetched the mapping already, and the new one lists it too.
    AlreadyForId,
    /// This mapping was refetched for another cover already.
    AlreadyForMapping,
    /// #808 paused stale-id refetches.
    Paused,
}

impl CoverRefetch {
    /// Why nothing is refetched, for the WARN. `cached`: a mapping is cached.
    fn reason(self, cached: bool) -> &'static str {
        match self {
            Self::Refetch => "the composition is refetched (stale-id)",
            Self::MappingChanged if cached => "a newer mapping is cached; the next fade uses it",
            Self::MappingChanged => "the mapping is dropped; the next push or tick refetches it",
            Self::AlreadyForId => "the composition was refetched for this id already",
            Self::AlreadyForMapping => "this mapping was refetched for another cover already",
            Self::Paused => "stale-id refetches are paused (#808)",
        }
    }
}

/// The worker's deadline for the next cover step: pending while none is
/// scheduled, so the `select!` branch never fires on its own.
pub(super) async fn cover_fade_deadline(due: Option<Instant>) {
    match due {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// The opacity a show brings a cover to when nothing tracked says otherwise:
/// its mapped value, or full opacity when it was mapped invisible.
fn default_cover_base(mapped_base: f64) -> f64 {
    if mapped_base > 0.0 {
        mapped_base
    } else {
        1.0
    }
}

/// Where a cover is now, and its base: the tracked values, else the mapped.
fn cover_now(cover: &CoverClip, tracked: &TrackedOpacities) -> TrackedOpacity {
    tracked.get(cover.param_id).unwrap_or(TrackedOpacity {
        base: default_cover_base(cover.mapped_base),
        level: cover.mapped_base,
    })
}

/// A cover's tracked opacity once Arena reported it at `shown`. A tracked
/// cover follows [`TrackedOpacity::observe`]: a shown cover dimmed in Arena
/// keeps that as its base, and 0 never becomes a base. An untracked one is
/// visible at `shown`, or hidden from its default base when `shown` is 0.
fn cover_read(tracked: Option<TrackedOpacity>, shown: f64, mapped_base: f64) -> TrackedOpacity {
    match tracked {
        Some(now) if (now.level - shown).abs() > SAME_OPACITY => now.observe(shown),
        Some(now) => now,
        None if shown > 0.0 => TrackedOpacity {
            base: shown,
            level: shown,
        },
        None => TrackedOpacity {
            base: default_cover_base(mapped_base),
            level: shown,
        },
    }
}

/// How long a cover takes from `from` to `to`: the share of its layer's
/// transition (capped by [`MAX_CLEAR_FADE`]) that the distance is of its
/// base (#848 second review: a fade that replaced one cut short ran the
/// whole transition, so quick verses stretched the hide).
fn cover_duration(fade: Duration, from: f64, to: f64, base: f64) -> Duration {
    let full = fade.min(MAX_CLEAR_FADE);
    let share = (to - from).abs() / base;
    if base <= 0.0 || !share.is_finite() || share >= 1.0 {
        return full;
    }
    Duration::try_from_secs_f64(full.as_secs_f64() * share).unwrap_or(full)
}

/// The fade from where each cover is now up to its base (show) or down to 0
/// (hide), over the share of its layer's transition left to go. A cover
/// already there is left out. Pure, so the planning is unit-tested without a
/// mock Resolume.
pub(super) fn plan_cover_fade(
    covers: &[CoverClip],
    tracked: &TrackedOpacities,
    direction: CoverDirection,
) -> Vec<FadeTarget> {
    covers
        .iter()
        .filter_map(|cover| {
            let now = cover_now(cover, tracked);
            let to = match direction {
                CoverDirection::Show => now.base,
                CoverDirection::Hide => 0.0,
            };
            ((now.level - to).abs() > SAME_OPACITY).then_some(FadeTarget {
                clip_id: cover.clip_id,
                param_id: cover.param_id,
                base: now.base,
                from: now.level,
                to,
                duration: cover_duration(cover.fade, now.level, to, now.base),
                mapped: cover.mapped,
            })
        })
        .collect()
}

/// Whether a target is still short of its `to`, and the level to PUT for
/// each such target `elapsed` into the fade. A level equal to the tracked
/// one is left out, so the first step (at 0) PUTs nothing (#848 second
/// review: it re-sent the level each fade starts from).
fn cover_levels(
    targets: &[FadeTarget],
    tracked: &TrackedOpacities,
    elapsed: Duration,
) -> (bool, Vec<(i64, f64)>) {
    let mut pending = false;
    let mut levels = Vec::new();
    for target in targets {
        let Some(now) = tracked.get(target.param_id) else {
            continue;
        };
        if (now.level - target.to).abs() <= SAME_OPACITY {
            continue;
        }
        pending = true;
        let level = fade_toward(target.from, target.to, elapsed, target.duration);
        if (level - now.level).abs() > SAME_OPACITY {
            levels.push((target.param_id, level));
        }
    }
    (pending, levels)
}

/// The clip ids of a fade: its targets once planned, else its covers.
fn fade_clip_ids(fade: &CoverFade) -> Vec<i64> {
    if fade.targets.is_empty() {
        fade.covers.iter().map(|cover| cover.clip_id).collect()
    } else {
        fade.targets.iter().map(|target| target.clip_id).collect()
    }
}

fn fade_levels(fade: &CoverFade, level: impl Fn(&FadeTarget) -> f64) -> Vec<f64> {
    fade.targets.iter().map(level).collect()
}

/// Milliseconds since the fade's first step, for the logs.
fn fade_ms(fade: &CoverFade) -> f64 {
    fade.started
        .map(|start| duration_ms(start.elapsed()))
        .unwrap_or(0.0)
}

/// `GET /parameter/by-id/{id}` (Resolume REST `get_parameter_by_id`): an
/// opacity is a `RangeParameter` whose `value` is the level. `Ok(None)` when
/// Resolume has no such parameter (404).
async fn read_opacity(
    client: &Client,
    endpoint: &ResolvedEndpoint,
    param_id: i64,
) -> anyhow::Result<Option<f64>> {
    let url = format!("{}/parameter/by-id/{}", endpoint.base_url, param_id);
    let mut request = client.get(&url);
    if let Some(host) = &endpoint.host_header {
        request = request.header(HOST, host.clone());
    }
    let response = request
        .timeout(ACTION_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("failed to read opacity parameter {param_id}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(anyhow!(
            "opacity parameter read failed with status {}",
            response.status()
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("invalid JSON for opacity parameter {param_id}"))?;
    body.get("value")
        .and_then(serde_json::Value::as_f64)
        .map(Some)
        .ok_or_else(|| anyhow!("opacity parameter {param_id} has no numeric value"))
}

impl HostDriver {
    /// When the scheduled cover fade's next step is due; `None` without one.
    pub(super) fn cover_fade_due(&self) -> Option<Instant> {
        self.cover.fade.as_ref().map(|fade| fade.due)
    }

    /// Schedule a cover fade in `direction` over the covers `mapping` lists
    /// and every remembered one, replacing the pending fade. Called once the
    /// push's own writes landed; the worker runs the first step as soon as no
    /// command waits.
    pub(super) fn schedule_cover_fade(&mut self, mapping: &ClipMapping, direction: CoverDirection) {
        for cover in mapped_covers(mapping) {
            remember_cover(&mut self.cover.remembered, cover);
        }
        let covers = cover_candidates(mapping, &self.cover.remembered);
        if let Some(replaced) = self.cover.fade.take() {
            self.log_cover_replaced(&replaced, direction);
        }
        if covers.is_empty() {
            return;
        }
        self.cover.fade = Some(CoverFade {
            direction,
            covers,
            mapping_fetched: self.last_mapping_refresh,
            targets: Vec::new(),
            started: None,
            due: Instant::now(),
            steps: 0,
            failed_step: None,
            forgotten: Vec::new(),
            stale: Vec::new(),
        });
    }

    /// The worker's deadline branch: one step of the scheduled cover fade.
    /// A failed step gets ONE final step at each target `FADE_STEP` later;
    /// a second failure stops the fade (at most 2 failed PUTs per cover, plus
    /// the first step's read; never a retry loop that holds the worker).
    pub(super) async fn step_cover_fade(&mut self) {
        let Some(mut fade) = self.cover.fade.take() else {
            return;
        };
        if !self.config.is_enabled {
            return;
        }
        match self.cover_step(&mut fade).await {
            CoverStep::Continue => self.next_cover_step(fade),
            CoverStep::Done => self.log_cover_done(&fade),
            CoverStep::Failed(err) if fade.failed_step.is_none() => {
                let error = format!("{err:#}");
                debug!(
                    host = %self.config.host,
                    direction = fade.direction.as_str(),
                    clip_ids = ?fade_clip_ids(&fade),
                    error = %error,
                    "resolume bible: a #bible-clear cover fade step failed; one final step puts each cover's target opacity"
                );
                fade.failed_step = Some(error);
                self.next_cover_step(fade);
            }
            CoverStep::Failed(err) => self.log_cover_failed(&fade, &err),
        }
    }

    fn next_cover_step(&mut self, mut fade: CoverFade) {
        fade.due = Instant::now() + FADE_STEP;
        self.cover.fade = Some(fade);
    }

    /// One step. The first reads Arena and plans (it PUTs only a cover on a
    /// layer without a transition: its target); each later one PUTs every
    /// cover's level for the time since the first step, and the step after a
    /// failed one is the final one, at each target. The fade ends at the step
    /// where that time reaches the longest duration (every level sent then
    /// is its target). A cover answering 404 is left out, the others go on.
    async fn cover_step(&mut self, fade: &mut CoverFade) -> CoverStep {
        let endpoint = match self.endpoint().await {
            Ok(endpoint) => endpoint,
            Err(err) => return CoverStep::Failed(err),
        };
        let first = fade.started.is_none();
        if first {
            self.plan_cover_from_arena(fade, &endpoint).await;
            fade.started = Some(Instant::now());
        }
        let elapsed = match fade.started {
            _ if fade.failed_step.is_some() => Duration::MAX,
            Some(start) if !first => start.elapsed(),
            _ => Duration::ZERO,
        };
        let (pending, levels) = cover_levels(&fade.targets, &self.opacities, elapsed);
        if !pending {
            return CoverStep::Done;
        }
        if levels.is_empty() {
            return CoverStep::Continue;
        }
        let puts = put_opacities(&self.client, &endpoint, &levels).await;
        for &(param_id, level) in &levels {
            if puts.landed.contains(&param_id) {
                self.opacities.record_level(param_id, level);
            }
        }
        for param_id in &puts.missing {
            self.drop_gone_cover(fade, *param_id, puts.error.as_ref());
        }
        if let Some(err) = puts.error {
            return CoverStep::Failed(err);
        }
        fade.steps += 1;
        let longest = fade
            .targets
            .iter()
            .map(|target| target.duration)
            .max()
            .unwrap_or_default();
        if elapsed >= longest {
            CoverStep::Done
        } else {
            CoverStep::Continue
        }
    }

    /// The first step: read every cover's opacity in Arena and track it, then
    /// plan. A cover answering 404 is left out ([`Self::drop_cover`]); a
    /// failed read keeps the tracked value. The targets are tracked BEFORE
    /// the first PUT, so a fade cut short is continued from where it stopped.
    async fn plan_cover_from_arena(&mut self, fade: &mut CoverFade, endpoint: &ResolvedEndpoint) {
        let client = &self.client;
        let reads = join_all(
            fade.covers
                .iter()
                .map(|cover| read_opacity(client, endpoint, cover.param_id)),
        )
        .await;
        let mut gone = Vec::new();
        for (cover, read) in fade.covers.iter().zip(reads) {
            match read {
                Ok(Some(shown)) => {
                    let now =
                        cover_read(self.opacities.get(cover.param_id), shown, cover.mapped_base);
                    self.opacities.set(cover.param_id, now);
                }
                Ok(None) => gone.push(*cover),
                Err(err) => debug!(
                    host = %self.config.host,
                    clip_id = cover.clip_id,
                    error = %format!("{err:#}"),
                    "resolume bible: reading the #bible-clear opacity failed; fading from the tracked value"
                ),
            }
        }
        for cover in &gone {
            self.drop_cover(fade, cover, None);
        }
        fade.targets = plan_cover_fade(&fade.covers, &self.opacities, fade.direction);
        for target in &fade.targets {
            let start = TrackedOpacity {
                base: target.base,
                level: target.from,
            };
            self.opacities.track_if_absent(target.param_id, start);
        }
    }

    /// The cover whose PUT answered 404 is left out ([`Self::drop_cover`]).
    /// `other` is any other error of the same step, for the stale WARN.
    fn drop_gone_cover(
        &mut self,
        fade: &mut CoverFade,
        param_id: i64,
        other: Option<&anyhow::Error>,
    ) {
        let cover = fade
            .covers
            .iter()
            .find(|cover| cover.param_id == param_id)
            .copied();
        match cover {
            Some(cover) => self.drop_cover(fade, &cover, other),
            None => debug!(
                host = %self.config.host,
                param_id,
                "resolume bible: a 404 for an opacity parameter that is not a cover of this fade"
            ),
        }
    }

    /// Leave a cover that answered 404 out of this fade; the others go on
    /// (#848 third review: one stale cover stopped them all, and a remembered
    /// title still playing stayed over the verse). A remembered one is gone
    /// and forgotten; a mapped one means a stale mapping ([`Self::cover_stale`]).
    fn drop_cover(
        &mut self,
        fade: &mut CoverFade,
        cover: &CoverClip,
        other: Option<&anyhow::Error>,
    ) {
        fade.covers.retain(|kept| kept.param_id != cover.param_id);
        fade.targets.retain(|kept| kept.param_id != cover.param_id);
        if cover.mapped {
            fade.stale.push(cover.clip_id);
            self.cover_stale(fade, cover, other);
        } else {
            self.forget_cover(fade, cover);
        }
    }

    /// Stop fading a remembered cover Resolume no longer knows (404), in this
    /// fade and later ones. It says nothing about the mapping, so nothing is
    /// refetched.
    fn forget_cover(&mut self, fade: &mut CoverFade, cover: &CoverClip) {
        self.opacities.forget(cover.param_id);
        forget_remembered_cover(&mut self.cover.remembered, cover.param_id);
        fade.forgotten.push(cover.clip_id);
        info!(
            host = %self.config.host,
            clip_id = cover.clip_id,
            param_id = cover.param_id,
            layer = cover.layer,
            "resolume bible: a remembered #bible-clear clip from outside the mapping is gone (404), no longer fading it"
        );
    }

    /// A mapped cover answered 404 and is left out of the fade
    /// ([`Self::drop_cover`]). The mapping is invalidated as `stale-id` (the
    /// next push or tick refetches it) when [`Self::cover_404_refetch`]
    /// allows; otherwise the WARN goes through the #563h limiter, so a cover
    /// that stays broken is not a WARN on every clear and verse.
    fn cover_stale(&mut self, fade: &CoverFade, cover: &CoverClip, other: Option<&anyhow::Error>) {
        let other_error = other.map_or_else(|| "none".to_owned(), |err| format!("{err:#}"));
        let first_error = fade.failed_step.as_deref().unwrap_or("none");
        let decision = self.cover_404_refetch(cover.param_id, fade.mapping_fetched);
        if decision == CoverRefetch::Refetch {
            self.refetch_for_cover_404(cover.param_id, fade.mapping_fetched);
            warn!(
                host = %self.config.host,
                direction = fade.direction.as_str(),
                clip_id = cover.clip_id,
                param_id = cover.param_id,
                fading_clip_ids = ?fade_clip_ids(fade),
                steps = fade.steps,
                other_error = %other_error,
                first_error,
                ok = false,
                "resolume bible: Resolume has no opacity parameter for a mapped #bible-clear clip (404); it is left out of the cover fade, the others go on, and the composition is refetched (stale-id)"
            );
            return;
        }
        let reason = decision.reason(self.last_mapping_refresh.is_some());
        if self.should_warn_cover_404(cover.param_id) {
            warn!(
                host = %self.config.host,
                direction = fade.direction.as_str(),
                clip_id = cover.clip_id,
                param_id = cover.param_id,
                fading_clip_ids = ?fade_clip_ids(fade),
                steps = fade.steps,
                other_error = %other_error,
                first_error,
                ok = false,
                "resolume bible: Resolume has no opacity parameter for a mapped #bible-clear clip (404); it is left out of the cover fade, the others go on, and nothing is refetched: {}",
                reason
            );
        } else {
            debug!(
                host = %self.config.host,
                direction = fade.direction.as_str(),
                clip_id = cover.clip_id,
                param_id = cover.param_id,
                other_error = %other_error,
                first_error,
                "resolume bible: a mapped #bible-clear clip still has no opacity parameter (404); it is left out of the cover fade, nothing is refetched: {}",
                reason
            );
        }
    }

    /// Invalidate the mapping as `stale-id` for the mapped cover `param_id`
    /// (the next push or tick refetches it) and remember that this id and
    /// this mapping did, so neither refetches again
    /// ([`Self::cover_404_refetch`]).
    pub(super) fn refetch_for_cover_404(
        &mut self,
        param_id: i64,
        mapping_fetched: Option<Instant>,
    ) {
        self.invalidate_mapping(FetchReason::StaleId);
        self.cover.refetched_ids.insert(param_id);
        self.cover.refetched_mapping = mapping_fetched;
    }

    /// Whether a mapped cover's 404 refetches the mapping: only while the
    /// mapping it came from is still the cached one, never again for an id
    /// that already did (the refetched mapping still lists it, so another
    /// refetch cannot help), at most once per fetched mapping, and not while
    /// stale-id refetches are paused (#808). A refetch invalidates the
    /// mapping, so "once per fetched mapping" is already kept by the first
    /// check; `AlreadyForMapping` stays as the guard should a refetch ever
    /// keep `last_mapping_refresh`.
    pub(super) fn cover_404_refetch(
        &self,
        param_id: i64,
        mapping_fetched: Option<Instant>,
    ) -> CoverRefetch {
        if mapping_fetched.is_none() || mapping_fetched != self.last_mapping_refresh {
            return CoverRefetch::MappingChanged;
        }
        if self.cover.refetched_ids.contains(&param_id) {
            CoverRefetch::AlreadyForId
        } else if self.cover.refetched_mapping == mapping_fetched {
            CoverRefetch::AlreadyForMapping
        } else if self.stale_refetch_paused() {
            CoverRefetch::Paused
        } else {
            CoverRefetch::Refetch
        }
    }

    /// #563h: whether a mapped cover's 404 that refetched nothing WARNs now
    /// (key `cover-404:<id>`): at most once per [`MISSING_CLIP_WARN_INTERVAL`]
    /// per id, DEBUG in between.
    pub(super) fn should_warn_cover_404(&mut self, param_id: i64) -> bool {
        let now = Instant::now();
        let warn_now = self
            .cover
            .warned_404
            .get(&param_id)
            .is_none_or(|last| now.duration_since(*last) >= MISSING_CLIP_WARN_INTERVAL);
        if warn_now {
            self.cover.warned_404.insert(param_id, now);
        }
        warn_now
    }

    fn log_cover_done(&self, fade: &CoverFade) {
        if fade.targets.is_empty() {
            debug!(
                host = %self.config.host,
                direction = fade.direction.as_str(),
                clip_ids = ?fade_clip_ids(fade),
                forgotten_clip_ids = ?fade.forgotten,
                stale_clip_ids = ?fade.stale,
                "resolume bible: no #bible-clear clip to fade (each is at its target opacity already, or gone)"
            );
            return;
        }
        info!(
            host = %self.config.host,
            direction = fade.direction.as_str(),
            clip_ids = ?fade_clip_ids(fade),
            from = ?fade_levels(fade, |target| target.from),
            to = ?fade_levels(fade, |target| target.to),
            forgotten_clip_ids = ?fade.forgotten,
            stale_clip_ids = ?fade.stale,
            steps = fade.steps,
            t_ms = fade_ms(fade),
            error_before_final_step = fade.failed_step.as_deref().unwrap_or("none"),
            ok = true,
            "resolume bible: faded the #bible-clear cover, no clip triggered"
        );
    }

    fn log_cover_replaced(&self, replaced: &CoverFade, by: CoverDirection) {
        if replaced.started.is_none() {
            debug!(
                host = %self.config.host,
                direction = replaced.direction.as_str(),
                replaced_by = by.as_str(),
                "resolume bible: a new #bible-clear cover fade replaced one that had not started"
            );
            return;
        }
        info!(
            host = %self.config.host,
            direction = replaced.direction.as_str(),
            replaced_by = by.as_str(),
            clip_ids = ?fade_clip_ids(replaced),
            from = ?fade_levels(replaced, |target| target.from),
            to = ?fade_levels(replaced, |target| target.to),
            forgotten_clip_ids = ?replaced.forgotten,
            stale_clip_ids = ?replaced.stale,
            steps = replaced.steps,
            t_ms = fade_ms(replaced),
            first_error = replaced.failed_step.as_deref().unwrap_or("none"),
            ok = true,
            "resolume bible: a new #bible-clear cover fade replaced this one before it ended; the new one starts where it got"
        );
    }

    fn log_cover_failed(&self, fade: &CoverFade, err: &anyhow::Error) {
        warn!(
            host = %self.config.host,
            direction = fade.direction.as_str(),
            clip_ids = ?fade_clip_ids(fade),
            steps = fade.steps,
            t_ms = fade_ms(fade),
            ok = false,
            first_error = fade.failed_step.as_deref().unwrap_or("none"),
            error = %format!("{err:#}"),
            "resolume bible: a #bible-clear cover fade step failed and so did its final step; the fade stops, the push it followed stays successful"
        );
    }
}
