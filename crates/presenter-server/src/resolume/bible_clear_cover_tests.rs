//! #848: the clip tagged `#bible-clear` (the sermon title on SNV) is a COVER.
//! A Bible clear fades its opacity UP to its base, a verse fades it DOWN to 0,
//! and it is never triggered.
//!
//! Owner, 2026-10-12: "ked sa clearne v presentery vers alebo je clearnuty tak
//! ma sa zapnut opacita bible clear a ked sa zapne vers tak sa ma stiahnut
//! opacita bible clear aby bol vers vyditelny". The cover fades over its OWN
//! layer's transition (SNV 2.3 s), so it is NOT part of the push (#848
//! review): the push returns after the Bible fade and the connects, and the
//! host worker steps the cover afterwards, between its other commands. This
//! file pins the mapping, the planners and the push side;
//! `bible_clear_cover_worker_tests.rs` drives the deferred fade through the
//! real host worker.

use super::bible_clear::{
    fade_toward, plan_clear_fade, plan_cover_hide, FadeTarget, TrackedOpacities,
};
use super::bible_clear_deck_tests::assert_faded;
use super::bible_clear_tests::{
    bible_layers, clear_update, clip, connects, deck_layers, driver_for, layer, opacities_put,
    start_resolume, values_put, verse_update, A_SIDE_OPACITY, BIBLE_A, B_SIDE_OPACITY, FADE,
};
use super::clip_map::ClipMapping;
use super::types::ClipOpacity;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wiremock::Request;

/// The cover clip, its text parameter and its opacity parameter (text + 100).
pub(super) const COVER: i64 = 500;
pub(super) const COVER_TEXT: i64 = 50;
pub(super) const COVER_OPACITY: i64 = 150;
/// The cover layer's transition where the length matters: far longer than the
/// Bible layers' [`BIBLE_FADE`], as on SNV (2.3 s against 0.5 s), so "the
/// push returned before the cover faded" has a wide margin on a loaded runner.
pub(super) const COVER_FADE: Duration = Duration::from_millis(1500);
/// A shorter cover transition for tests about the fade's shape, not its length.
pub(super) const SHORT_COVER_FADE: Duration = Duration::from_millis(450);
/// The Bible layers' transition in the timing tests.
pub(super) const BIBLE_FADE: Duration = Duration::from_millis(300);
/// The `#timer` clip and its text parameter: the worker tests use timer frames
/// as commands that must not wait for a cover fade.
const TIMER: i64 = 600;
pub(super) const TIMER_TEXT: i64 = 60;

/// The SNV shape on deck `offset` (every id shifted by it, as a deck switch
/// does): the Bible layers, the sermon title on a layer of its own at
/// `cover_opacity`, and a `#timer` clip.
pub(super) fn cover_layers_on(
    offset: i64,
    transition: Option<Duration>,
    cover_transition: Option<Duration>,
    cover_opacity: f64,
) -> Vec<Value> {
    let mut layers = deck_layers(offset, transition);
    layers.push(cover_layer(offset, cover_transition, cover_opacity));
    layers.push(timer_layer(offset));
    layers
}

/// [`cover_layers_on`] the first deck.
pub(super) fn cover_layers(
    transition: Option<Duration>,
    cover_transition: Option<Duration>,
    cover_opacity: f64,
) -> Vec<Value> {
    cover_layers_on(0, transition, cover_transition, cover_opacity)
}

/// The layer of the sermon title, named like the SNV clip, at `opacity`.
pub(super) fn cover_layer(offset: i64, transition: Option<Duration>, opacity: f64) -> Value {
    layer(
        vec![clip(
            COVER + offset,
            "NAZOV #bible-clear",
            COVER_TEXT + offset,
            opacity,
        )],
        transition,
    )
}

pub(super) fn timer_layer(offset: i64) -> Value {
    layer(
        vec![clip(TIMER + offset, "#timer", TIMER_TEXT + offset, 1.0)],
        None,
    )
}

/// Assert that the cover opacities `steps` went only up, in at least three
/// steps from near 0, and ended at `base`: a fade, not a cut.
pub(super) fn assert_shown(steps: &[f64], base: f64) {
    assert!(
        steps.len() >= 3 && steps[0] < 0.5,
        "the cover is faded in steps from near 0, not cut: {steps:?}"
    );
    assert!(
        steps.windows(2).all(|pair| pair[1] > pair[0]),
        "the cover only goes up: {steps:?}"
    );
    let last = steps.last().copied().unwrap_or_default();
    assert!(
        (last - base).abs() < 1e-9,
        "the cover ends at {base}: {steps:?}"
    );
}

/// Assert that the cover opacities `steps` went only down, in at least three
/// steps, and ended at 0.
pub(super) fn assert_hidden(steps: &[f64]) {
    assert!(
        steps.len() >= 3,
        "the cover is faded out in steps, not cut: {steps:?}"
    );
    assert!(
        steps.windows(2).all(|pair| pair[1] < pair[0]),
        "the cover only goes down: {steps:?}"
    );
    assert_eq!(steps.last(), Some(&0.0), "the cover ends at 0: {steps:?}");
}

#[test]
fn the_sermon_title_tagged_bible_clear_is_an_optional_cover() {
    let mapping = ClipMapping::from_composition(
        &json!({ "layers": cover_layers(Some(FADE), Some(COVER_FADE), 0.0) }),
    )
    .expect("mapping");
    let covers: Vec<(i64, Option<ClipOpacity>, Duration)> = mapping
        .bible_clear
        .iter()
        .map(|target| (target.clip_id, target.opacity, target.fade))
        .collect();
    assert_eq!(
        covers,
        vec![(
            COVER,
            Some(ClipOpacity {
                param_id: COVER_OPACITY,
                base: 0.0
            }),
            COVER_FADE
        )],
        "the tag after other words maps the clip with its opacity and layer fade"
    );
    assert!(mapping
        .bible_a
        .iter()
        .all(|target| target.clip_id == BIBLE_A));

    // PP has no cover: it is optional, so never reported as missing.
    let without =
        ClipMapping::from_composition(&json!({ "layers": bible_layers(None) })).expect("mapping");
    assert!(without.bible_clear.is_empty());
    for mapping in [&mapping, &without] {
        assert!(!mapping.missing_tokens().iter().any(|k| k.contains("clear")));
        assert!(!mapping
            .destination_kinds()
            .iter()
            .any(|k| k.contains("clear")));
    }
}

#[test]
fn fade_toward_runs_linearly_up_or_down_to_its_target() {
    let duration = Duration::from_millis(500);
    let half = Duration::from_millis(250);
    assert_eq!(fade_toward(0.2, 1.0, Duration::ZERO, duration), 0.2);
    assert!((fade_toward(0.2, 1.0, half, duration) - 0.6).abs() < 1e-9);
    assert!((fade_toward(0.8, 0.0, half, duration) - 0.4).abs() < 1e-9);
    assert_eq!(fade_toward(0.2, 0.7, duration, duration), 0.7);
    assert_eq!(fade_toward(0.2, 0.7, Duration::from_secs(9), duration), 0.7);
    assert_eq!(fade_toward(0.0, 1.0, Duration::ZERO, Duration::ZERO), 1.0);
}

/// #848 review, finding 1: the cover's 2.3 s fade ran inside the clear and
/// held the serial host worker. The clear's own fade is only the Bible clips
/// (#841); the cover is the deferred fade's job.
#[test]
fn the_clear_fade_leaves_the_covers_to_the_deferred_cover_fade() {
    let mapping = ClipMapping::from_composition(
        &json!({ "layers": cover_layers(Some(FADE), Some(COVER_FADE), 0.0) }),
    )
    .expect("mapping");
    let mut tracked = TrackedOpacities::default();
    tracked.sync_with_fetch(&mapping, None);

    let plan = plan_clear_fade(&mapping, &tracked);
    let mut params: Vec<i64> = plan.iter().map(|target| target.param_id).collect();
    params.sort_unstable();
    let mut bible: Vec<i64> = A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY).collect();
    bible.sort_unstable();
    assert_eq!(params, bible, "only the Bible clips, no cover: {plan:?}");
    assert!(plan.iter().all(|target| target.to == 0.0), "{plan:?}");
}

/// The hide plan: each cover once, from its mapped opacity down to 0. A cover
/// that is already invisible gets nothing, and a clip tagged as a cover AND a
/// Bible lane clip is left to the lane (the verse just showed it).
#[test]
fn the_hide_plan_fades_each_cover_once_from_where_it_is() {
    let mut layers = bible_layers(None);
    layers.push(layer(
        vec![
            clip(COVER, "NAZOV #bible-clear", COVER_TEXT, 0.7),
            clip(501, "#bible-clear", 51, 0.0),
            clip(302, "#bible-a #bible-clear", 32, 1.0),
        ],
        Some(SHORT_COVER_FADE),
    ));
    let mapping = ClipMapping::from_composition(&json!({ "layers": layers })).expect("mapping");

    let plan = plan_cover_hide(&mapping, &TrackedOpacities::default());
    assert_eq!(
        plan,
        vec![FadeTarget {
            clip_id: COVER,
            param_id: COVER_OPACITY,
            base: 0.7,
            from: 0.7,
            to: 0.0,
            duration: SHORT_COVER_FADE,
            mapped: true,
        }]
    );
}

/// #848 review, finding 1: the clear held the worker for the cover's whole
/// transition, so the `#timer` countdown froze and the next verse waited. The
/// push returns once the Bible clips are faded; the cover is not touched
/// inside it.
#[tokio::test]
async fn a_clear_returns_before_the_cover_has_faded() {
    let server = start_resolume(cover_layers(Some(BIBLE_FADE), Some(COVER_FADE), 0.0)).await;
    let (mut driver, status) = driver_for(&server);

    let start = Instant::now();
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let took = start.elapsed();

    assert!(
        took < COVER_FADE,
        "the clear returned after {took:?}, it must not wait for the cover's {COVER_FADE:?}"
    );
    let requests = server.received_requests().await.expect("requests");
    assert_faded(&requests, A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY));
    assert!(connects(&requests).is_empty(), "a clear triggers no clip");
    assert!(values_put(&requests, COVER_TEXT).is_empty());
    assert!(
        opacities_put(&requests, COVER_OPACITY).is_empty(),
        "the cover is faded after the push, not inside it"
    );
}

/// The verse after a clear is on air at once: it does not wait for the cover
/// to fade out either.
#[tokio::test]
async fn a_verse_returns_before_the_cover_has_faded() {
    let server = start_resolume(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");

    let start = Instant::now();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");
    let took = start.elapsed();

    assert!(
        took < COVER_FADE,
        "the verse returned after {took:?}, it must not wait for the cover's {COVER_FADE:?}"
    );
    let requests: Vec<Request> = server.received_requests().await.expect("requests");
    assert_eq!(connects(&requests).len(), 4, "the verse is on air");
}
