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
//!   its target gets nothing. A failed read keeps the tracked value.
//! - It is best-effort. An error never fails a push and never touches the host
//!   status or the #484 backoff: the fade logs one WARN and stops. A mapped
//!   cover's 404 also invalidates the mapping as `stale-id`, at most once per
//!   fetched mapping and once per id, so a cover that stays broken cannot pull
//!   the 16 MB composition on every clear and verse.
//! - Every cover a push's mapping listed is remembered (like the clips a verse
//!   connected, #841): after a deck switch the old deck's cover still plays on
//!   its layer, so later fades include it. One answering 404 is forgotten.

use super::bible_clear::{
    bible_lane_clips, fade_toward, put_opacities, FadeTarget, TrackedOpacities, TrackedOpacity,
    FADE_STEP, MAX_CLEAR_FADE, SAME_OPACITY,
};
use super::clip_map::ClipMapping;
use super::driver::{duration_ms, FetchReason, HostDriver, ACTION_TIMEOUT};
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
    /// Steps that went out without an error.
    steps: usize,
}

/// The mapped cover 404 that last invalidated the mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StaleCoverRefetch {
    param_id: i64,
    mapping_fetched: Option<Instant>,
}

/// One host's cover fade state; reset with its config.
#[derive(Debug, Default)]
pub(super) struct CoverState {
    fade: Option<CoverFade>,
    /// Every cover a Bible push's mapping listed, by opacity parameter id.
    remembered: HashMap<i64, CoverClip>,
    stale_refetch: Option<StaleCoverRefetch>,
}

/// How one step ended.
enum CoverStep {
    Continue,
    Done,
    /// A mapped cover answered 404.
    Stale(i64),
    Failed(anyhow::Error),
}

/// The worker's deadline for the next cover step: pending while none is
/// scheduled, so the `select!` branch never fires on its own.
pub(super) async fn cover_fade_deadline(due: Option<Instant>) {
    match due {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// The covers a fade scheduled with `mapping` works on: each `#bible-clear`
/// clip of the mapping with an opacity parameter, then each remembered cover
/// the mapping no longer lists (`mapped: false`). Each once, and none that is
/// also a Bible lane clip of the mapping (the lane owns its opacity).
pub(super) fn cover_candidates(
    mapping: &ClipMapping,
    remembered: &HashMap<i64, CoverClip>,
) -> Vec<CoverClip> {
    let mut seen: HashSet<i64> = bible_lane_clips(mapping)
        .filter_map(|target| target.opacity)
        .map(|opacity| opacity.param_id)
        .collect();
    let mut covers: Vec<CoverClip> = mapping
        .bible_clear
        .iter()
        .filter_map(|target| {
            let opacity = target.opacity?;
            seen.insert(opacity.param_id).then_some(CoverClip {
                clip_id: target.clip_id,
                param_id: opacity.param_id,
                mapped_base: opacity.base,
                fade: target.fade,
                mapped: true,
            })
        })
        .collect();
    let mut gone: Vec<CoverClip> = remembered
        .values()
        .filter(|cover| !seen.contains(&cover.param_id))
        .map(|cover| CoverClip {
            mapped: false,
            ..*cover
        })
        .collect();
    gone.sort_unstable_by_key(|cover| cover.param_id);
    covers.extend(gone);
    covers
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

/// The fade from where each cover is now up to its base (show) or down to 0
/// (hide), over its layer's transition. A cover already there is left out.
/// Pure, so the planning is unit-tested without a mock Resolume.
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
                duration: cover.fade.min(MAX_CLEAR_FADE),
                mapped: cover.mapped,
            })
        })
        .collect()
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
        let covers = cover_candidates(mapping, &self.cover.remembered);
        for cover in covers.iter().filter(|cover| cover.mapped) {
            self.cover.remembered.insert(cover.param_id, *cover);
        }
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
        });
    }

    /// The worker's deadline branch: one step of the scheduled cover fade.
    pub(super) async fn step_cover_fade(&mut self) {
        let Some(mut fade) = self.cover.fade.take() else {
            return;
        };
        if !self.config.is_enabled {
            return;
        }
        match self.cover_step(&mut fade).await {
            CoverStep::Continue => {
                fade.due = Instant::now() + FADE_STEP;
                self.cover.fade = Some(fade);
            }
            CoverStep::Done => self.log_cover_done(&fade),
            CoverStep::Stale(param_id) => self.cover_stale(&fade, param_id),
            CoverStep::Failed(err) => self.log_cover_failed(&fade, &err),
        }
    }

    /// Plan on the first step, then PUT each cover's level for the time since
    /// the first step. The fade ends at the step where that time reaches the
    /// longest transition (every level sent then is its target).
    async fn cover_step(&mut self, fade: &mut CoverFade) -> CoverStep {
        let endpoint = match self.endpoint().await {
            Ok(endpoint) => endpoint,
            Err(err) => return CoverStep::Failed(err),
        };
        if fade.started.is_none() {
            if let Some(param_id) = self.plan_cover_from_arena(fade, &endpoint).await {
                return CoverStep::Stale(param_id);
            }
            fade.started = Some(Instant::now());
        }
        let elapsed = fade
            .started
            .map(|start| start.elapsed())
            .unwrap_or_default();
        let levels: Vec<(i64, f64)> = fade
            .targets
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
            return CoverStep::Done;
        }
        let puts = put_opacities(&self.client, &endpoint, &levels).await;
        for &(param_id, level) in &levels {
            if puts.landed.contains(&param_id) {
                self.opacities.record_level(param_id, level);
            }
        }
        if let Some(param_id) = self.drop_gone_covers(fade, &puts.missing) {
            return CoverStep::Stale(param_id);
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
    /// plan. A cover outside the mapping answering 404 is forgotten; a mapped
    /// one is returned (the mapping is stale). A failed read keeps the
    /// tracked value. The targets are tracked BEFORE the first PUT, so a fade
    /// cut short is continued from where it stopped.
    async fn plan_cover_from_arena(
        &mut self,
        fade: &mut CoverFade,
        endpoint: &ResolvedEndpoint,
    ) -> Option<i64> {
        let client = &self.client;
        let reads = join_all(
            fade.covers
                .iter()
                .map(|cover| read_opacity(client, endpoint, cover.param_id)),
        )
        .await;
        let mut gone = HashSet::new();
        for (cover, read) in fade.covers.iter().zip(reads) {
            match read {
                Ok(Some(shown)) => {
                    let now =
                        cover_read(self.opacities.get(cover.param_id), shown, cover.mapped_base);
                    self.opacities.set(cover.param_id, now);
                }
                Ok(None) if cover.mapped => return Some(cover.param_id),
                Ok(None) => {
                    self.forget_cover(cover);
                    gone.insert(cover.param_id);
                }
                Err(err) => debug!(
                    host = %self.config.host,
                    clip_id = cover.clip_id,
                    error = %format!("{err:#}"),
                    "resolume bible: reading the #bible-clear opacity failed; fading from the tracked value"
                ),
            }
        }
        fade.covers.retain(|cover| !gone.contains(&cover.param_id));
        fade.targets = plan_cover_fade(&fade.covers, &self.opacities, fade.direction);
        for target in &fade.targets {
            let start = TrackedOpacity {
                base: target.base,
                level: target.from,
            };
            self.opacities.track_if_absent(target.param_id, start);
        }
        None
    }

    /// Forget every cover outside the mapping that answered 404 (it is gone,
    /// the fade goes on without it). Returns a mapped one: a stale mapping.
    fn drop_gone_covers(&mut self, fade: &CoverFade, missing: &[i64]) -> Option<i64> {
        let mut stale = None;
        for &param_id in missing {
            match fade.covers.iter().find(|cover| cover.param_id == param_id) {
                Some(cover) if !cover.mapped => self.forget_cover(cover),
                _ if stale.is_none() => stale = Some(param_id),
                _ => {}
            }
        }
        stale
    }

    /// Stop fading a remembered cover Resolume no longer knows (404). It says
    /// nothing about the mapping, so nothing is refetched.
    fn forget_cover(&mut self, cover: &CoverClip) {
        self.opacities.forget(cover.param_id);
        self.cover.remembered.remove(&cover.param_id);
        info!(
            host = %self.config.host,
            clip_id = cover.clip_id,
            param_id = cover.param_id,
            "resolume bible: a remembered #bible-clear clip from outside the mapping is gone (404), no longer fading it"
        );
    }

    /// A mapped cover answered 404: the fade stops, and the mapping is
    /// invalidated as `stale-id` (the next push or tick refetches it) unless
    /// [`Self::cover_404_refetches`] says no.
    fn cover_stale(&mut self, fade: &CoverFade, param_id: i64) {
        let refetch = self.cover_404_refetches(param_id, fade.mapping_fetched);
        if refetch {
            self.invalidate_mapping(FetchReason::StaleId);
            self.cover.stale_refetch = Some(StaleCoverRefetch {
                param_id,
                mapping_fetched: fade.mapping_fetched,
            });
        }
        let outcome = if refetch {
            "the composition is refetched (stale-id)"
        } else {
            "the mapping was refetched for it already"
        };
        warn!(
            host = %self.config.host,
            direction = fade.direction.as_str(),
            clip_ids = ?fade_clip_ids(fade),
            param_id,
            refetch,
            steps = fade.steps,
            ok = false,
            "resolume bible: Resolume has no opacity parameter for a mapped #bible-clear clip (404); the cover fade stops and {}",
            outcome
        );
    }

    /// Whether a mapped cover's 404 refetches the mapping: only while the
    /// mapping it came from is still the cached one, at most once per fetched
    /// mapping, never again for an id that already did (the refetched mapping
    /// still lists it, so another refetch cannot help), and not while stale-id
    /// refetches are paused (#808).
    fn cover_404_refetches(&self, param_id: i64, mapping_fetched: Option<Instant>) -> bool {
        let repeated = self.cover.stale_refetch.is_some_and(|last| {
            last.param_id == param_id || last.mapping_fetched == mapping_fetched
        });
        mapping_fetched.is_some()
            && mapping_fetched == self.last_mapping_refresh
            && !repeated
            && !self.stale_refetch_paused()
    }

    fn log_cover_done(&self, fade: &CoverFade) {
        if fade.targets.is_empty() {
            debug!(
                host = %self.config.host,
                direction = fade.direction.as_str(),
                clip_ids = ?fade_clip_ids(fade),
                "resolume bible: every #bible-clear clip is at its target opacity already, nothing to fade"
            );
            return;
        }
        info!(
            host = %self.config.host,
            direction = fade.direction.as_str(),
            clip_ids = ?fade_clip_ids(fade),
            from = ?fade_levels(fade, |target| target.from),
            to = ?fade_levels(fade, |target| target.to),
            steps = fade.steps,
            t_ms = fade_ms(fade),
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
            steps = replaced.steps,
            t_ms = fade_ms(replaced),
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
            error = %format!("{err:#}"),
            "resolume bible: the #bible-clear cover fade failed and stops; the push it followed stays successful"
        );
    }
}
