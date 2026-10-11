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
//! crossfades it in from an invisible clip. Before a faded clip is connected
//! again, [`HostDriver::restore_opacity`] puts its opacity back.
//!
//! [`DimmedClips`] remembers every clip that may be off its base opacity. A
//! clip enters it BEFORE its first fade PUT and leaves it only after a
//! successful restore, so a fade that fails halfway can never leave a clip
//! stuck semi-transparent.

use super::clip_map::ClipMapping;
use super::driver::{duration_ms, HostDriver, ACTION_TIMEOUT};
use super::mapping_refresh::StaleIdError;
use super::types::{ClipOpacity, ClipTarget, ResolvedEndpoint};
use anyhow::{anyhow, Context};
use futures_util::{stream::FuturesUnordered, StreamExt};
use reqwest::{Client, StatusCode};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::{debug, info};

/// Upper bound of a clear's fade. The host worker is serial, so a fade holds
/// every later push of this host for its whole length.
pub(super) const MAX_CLEAR_FADE: Duration = Duration::from_secs(3);
/// Time between two fade steps (about 33 opacity updates per second).
const FADE_STEP: Duration = Duration::from_millis(30);

/// A clip whose opacity a clear lowered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DimmedClip {
    /// The opacity a restore puts back.
    pub(super) base: f64,
    /// The last opacity Resolume accepted from us.
    pub(super) level: f64,
}

/// The clips that may be off their base opacity, keyed by opacity parameter
/// id. Kept across mapping refetches (the ids survive them); reset with the
/// host config.
#[derive(Debug, Default)]
pub(super) struct DimmedClips {
    clips: HashMap<i64, DimmedClip>,
    /// The mapping fetch (`last_mapping_refresh`) whose invisible clips were
    /// adopted, so a cached mapping is scanned once, not on every push.
    adopted_from: Option<Instant>,
}

impl DimmedClips {
    pub(super) fn get(&self, param_id: i64) -> Option<DimmedClip> {
        self.clips.get(&param_id).copied()
    }

    /// Adopt every Bible lane clip the mapping saw at opacity 0 that is not
    /// tracked yet: a clear left it invisible, possibly one of a previous
    /// process. It goes back to full opacity before its next connect.
    pub(super) fn adopt_invisible(&mut self, mapping: &ClipMapping, fetched: Option<Instant>) {
        if fetched.is_some() && self.adopted_from == fetched {
            return;
        }
        self.adopted_from = fetched;
        for opacity in bible_lane_clips(mapping).filter_map(|target| target.opacity) {
            if opacity.base <= 0.0 {
                self.clips.entry(opacity.param_id).or_insert(DimmedClip {
                    base: 1.0,
                    level: 0.0,
                });
            }
        }
    }
}

/// One clip of a clear's fade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FadeTarget {
    pub(super) clip_id: i64,
    pub(super) param_id: i64,
    /// The opacity a restore puts back.
    pub(super) base: f64,
    /// The opacity the fade starts from.
    pub(super) from: f64,
    pub(super) duration: Duration,
}

/// Every Bible lane clip of `mapping`, A and B sides.
fn bible_lane_clips(mapping: &ClipMapping) -> impl Iterator<Item = &ClipTarget> {
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

/// The fade a clear runs: each Bible lane clip with an opacity parameter that
/// is still visible, from where it is now. A clip tagged twice fades once.
/// Pure, so the selection is unit-tested without a mock Resolume.
pub(super) fn plan_clear_fade(mapping: &ClipMapping, dimmed: &DimmedClips) -> Vec<FadeTarget> {
    let mut seen = HashSet::new();
    bible_lane_clips(mapping)
        .filter_map(|target| {
            let opacity = target.opacity?;
            if !seen.insert(opacity.param_id) {
                return None;
            }
            let (base, from) = match dimmed.get(opacity.param_id) {
                Some(clip) => (clip.base, clip.level),
                None => (opacity.base, opacity.base),
            };
            (from > 0.0).then_some(FadeTarget {
                clip_id: target.clip_id,
                param_id: opacity.param_id,
                base,
                from,
                duration: target.fade,
            })
        })
        .collect()
}

/// The opacity of a fading clip `elapsed` into its fade: linear from `from`
/// down to 0, reached exactly at `duration` (at once for a zero duration).
pub(super) fn fade_level(from: f64, elapsed: Duration, duration: Duration) -> f64 {
    if elapsed >= duration {
        return 0.0;
    }
    let remaining = 1.0 - elapsed.as_secs_f64() / duration.as_secs_f64();
    (from * remaining).max(0.0)
}

impl HostDriver {
    /// Fade every visible Bible lane clip of this host to opacity 0, without
    /// triggering a clip or writing text (see the module docs).
    pub(super) async fn handle_bible_clear(&mut self, mapping: &ClipMapping) -> anyhow::Result<()> {
        let fade = plan_clear_fade(mapping, &self.dimmed);
        let fade_ms = fade
            .iter()
            .map(|target| duration_ms(target.duration))
            .fold(0.0, f64::max);
        let faded_clip_ids: Vec<i64> = fade.iter().map(|target| target.clip_id).collect();
        let start = Instant::now();
        let result = self.run_fade(&fade).await;
        let steps = result.as_ref().ok().copied().unwrap_or(0);
        info!(
            host = %self.config.host,
            ?faded_clip_ids,
            fade_ms,
            steps,
            t_ms = duration_ms(start.elapsed()),
            ok = result.is_ok(),
            "resolume bible clear: faded the Bible clips' opacity to 0, no clip triggered"
        );
        result.map(|_| ())
    }

    /// Step every target's opacity down to 0 over its duration. Returns the
    /// number of steps sent.
    async fn run_fade(&mut self, fade: &[FadeTarget]) -> anyhow::Result<usize> {
        if fade.is_empty() {
            return Ok(0);
        }
        // Tracked BEFORE the first PUT: a fade that fails halfway must still
        // be restored before these clips are connected again.
        for target in fade {
            self.dimmed
                .clips
                .entry(target.param_id)
                .or_insert(DimmedClip {
                    base: target.base,
                    level: target.from,
                });
        }
        let endpoint = self.endpoint().await?;
        let start = Instant::now();
        let mut steps = 0;
        loop {
            let elapsed = start.elapsed();
            let levels: Vec<(i64, f64)> = fade
                .iter()
                .filter(|target| {
                    self.dimmed
                        .get(target.param_id)
                        .is_some_and(|c| c.level > 0.0)
                })
                .map(|target| {
                    let level = fade_level(target.from, elapsed, target.duration);
                    (target.param_id, level)
                })
                .collect();
            if levels.is_empty() {
                return Ok(steps);
            }
            put_opacities(&self.client, &endpoint, &levels).await?;
            for (param_id, level) in levels {
                if let Some(clip) = self.dimmed.clips.get_mut(&param_id) {
                    clip.level = level;
                }
            }
            steps += 1;
            sleep(FADE_STEP).await;
        }
    }

    /// Put back the base opacity of every clip in `targets` that a clear
    /// lowered, before those clips are connected again (their text is
    /// already written, and the A/B side being connected is not live).
    pub(super) async fn restore_opacity(&mut self, targets: &[ClipTarget]) -> anyhow::Result<()> {
        let mut seen = HashSet::new();
        let restores: Vec<(i64, f64)> = targets
            .iter()
            .filter_map(|target| target.opacity)
            .filter(|opacity: &ClipOpacity| seen.insert(opacity.param_id))
            .filter_map(|opacity| {
                self.dimmed
                    .get(opacity.param_id)
                    .map(|clip| (opacity.param_id, clip.base))
            })
            .collect();
        if restores.is_empty() {
            return Ok(());
        }
        let endpoint = self.endpoint().await?;
        put_opacities(&self.client, &endpoint, &restores).await?;
        for (param_id, _) in &restores {
            self.dimmed.clips.remove(param_id);
        }
        debug!(
            host = %self.config.host,
            restored = ?restores,
            "resolume bible: restored the opacity of faded clips before connecting them"
        );
        Ok(())
    }
}

/// PUT each `(opacity param id, value)` concurrently; the first failure wins.
/// A 404 is a stale mapping (#808), so the push refetches and retries.
async fn put_opacities(
    client: &Client,
    endpoint: &ResolvedEndpoint,
    levels: &[(i64, f64)],
) -> anyhow::Result<()> {
    let mut futures = FuturesUnordered::new();
    for &(param_id, value) in levels {
        let url = format!("{}/parameter/by-id/{}", endpoint.base_url, param_id);
        let mut request = client.put(&url);
        if let Some(host) = &endpoint.host_header {
            request = request.header(reqwest::header::HOST, host.clone());
        }
        futures.push(async move {
            let response = request
                .json(&serde_json::json!({ "value": value }))
                .timeout(ACTION_TIMEOUT)
                .send()
                .await
                .with_context(|| format!("failed to update opacity parameter {param_id}"))?;
            if response.status() == StatusCode::NOT_FOUND {
                return Err(StaleIdError::opacity_parameter(param_id));
            }
            if !response.status().is_success() {
                return Err(anyhow!(
                    "opacity parameter update failed with status {}",
                    response.status()
                ));
            }
            Ok(())
        });
    }
    while let Some(result) = futures.next().await {
        result?;
    }
    Ok(())
}
