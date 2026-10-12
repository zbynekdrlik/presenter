//! #848: the clip tagged `#bible-clear` (the sermon title on SNV) is a COVER.
//! A Bible clear fades its opacity UP to its base, a verse fades it DOWN to 0,
//! and it is never triggered.
//!
//! Owner, 2026-10-12: "ked sa clearne v presentery vers alebo je clearnuty tak
//! ma sa zapnut opacita bible clear a ked sa zapne vers tak sa ma stiahnut
//! opacita bible clear aby bol vers vyditelny". The contract pinned here:
//!
//! - `#bible-clear` / `#bibleclear` map to `ClipMapping::bible_clear`, an
//!   optional kind (PP has none): never a missing clip, never a destination
//!   kind;
//! - a clear fades the cover up over its OWN layer's transition while the
//!   Bible clips still fade down (#841), and connects nothing;
//! - a clear while the Bible clips are already invisible still shows the
//!   cover;
//! - a verse fades the cover down only after its clips are connected, and a
//!   failed cover PUT never fails the verse (the retry of a failed push would
//!   connect the verse again);
//! - the verse after that sends no cover PUT;
//! - a fetch shows what Arena has, for the cover too.

use super::bible_clear::fade_toward;
use super::bible_clear_deck_tests::assert_faded;
use super::bible_clear_tests::{
    answer_param, bible_layers, clear_update, clip, connects, driver_for, layer, mount_arena,
    opacities_put, param_path, position, start_resolume, values_put, verse_update, A_SIDE_OPACITY,
    BIBLE_A, B_SIDE_OPACITY, FADE,
};
use super::clip_map::ClipMapping;
use super::types::ClipOpacity;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wiremock::Request;

/// The cover clip, its text parameter and its opacity parameter (text + 100).
const COVER: i64 = 500;
const COVER_TEXT: i64 = 50;
const COVER_OPACITY: i64 = 150;
/// The cover layer's transition: longer than the Bible layers' [`FADE`], as
/// on SNV (2.3 s against 0.5 s).
const COVER_FADE: Duration = Duration::from_millis(900);

/// The layer of the sermon title, named like the SNV clip, at `opacity`.
fn cover_layer(transition: Option<Duration>, opacity: f64) -> Value {
    layer(
        vec![clip(COVER, "NAZOV #bible-clear", COVER_TEXT, opacity)],
        transition,
    )
}

/// The SNV shape: the Bible layers plus the cover on a layer of its own.
fn cover_layers(
    transition: Option<Duration>,
    cover_transition: Option<Duration>,
    cover_opacity: f64,
) -> Vec<Value> {
    let mut layers = bible_layers(transition);
    layers.push(cover_layer(cover_transition, cover_opacity));
    layers
}

/// Assert that the cover was stepped UP to full opacity from near 0: a fade,
/// not a cut.
fn assert_shown(requests: &[Request]) {
    let steps = opacities_put(requests, COVER_OPACITY);
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
        (last - 1.0).abs() < 1e-9,
        "the cover ends at full opacity: {steps:?}"
    );
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

#[tokio::test]
async fn a_clear_fades_the_cover_up_over_its_layer_and_the_bible_clips_down() {
    let server = start_resolume(cover_layers(Some(FADE), Some(COVER_FADE), 0.0)).await;
    let (mut driver, status) = driver_for(&server);

    let start = Instant::now();
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let took = start.elapsed();

    let requests = server.received_requests().await.expect("requests");
    assert!(
        connects(&requests).is_empty(),
        "a clear triggers no clip, not even the cover"
    );
    assert!(
        values_put(&requests, COVER_TEXT).is_empty(),
        "the cover's text is never written"
    );
    assert_shown(&requests);
    assert_faded(&requests, A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY));
    assert!(
        took >= COVER_FADE,
        "the cover fades over its own layer's transition ({COVER_FADE:?}), took {took:?}"
    );
}

#[tokio::test]
async fn a_verse_fades_the_cover_down_after_connecting_its_clips() {
    let server = start_resolume(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let after_clear = server.received_requests().await.expect("requests").len();

    let start = Instant::now();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");
    let took = start.elapsed();

    let requests = server.received_requests().await.expect("requests");
    let verse = &requests[after_clear..];
    let steps = opacities_put(verse, COVER_OPACITY);
    assert!(
        steps.len() >= 3 && steps[0] > 0.5,
        "the cover is faded out in steps from full opacity: {steps:?}"
    );
    assert!(
        steps.windows(2).all(|pair| pair[1] < pair[0]),
        "the cover only goes down: {steps:?}"
    );
    assert_eq!(steps.last(), Some(&0.0), "the cover ends at 0: {steps:?}");
    let last_connect = verse
        .iter()
        .rposition(|req| req.url.path().ends_with("/connect"))
        .expect("the verse connects its clips");
    let first_cover = position(verse, |req| req.url.path() == param_path(COVER_OPACITY))
        .expect("the verse fades the cover");
    assert!(
        last_connect < first_cover,
        "the verse is on air before the cover starts to fade"
    );
    assert!(
        took >= COVER_FADE,
        "the cover fades over its layer's transition ({COVER_FADE:?}), took {took:?}"
    );
    for param in A_SIDE_OPACITY {
        assert_eq!(
            opacities_put(verse, param),
            vec![1.0],
            "param {param} is restored before the connect, as before (#841)"
        );
    }
}

#[tokio::test]
async fn the_verse_after_that_sends_no_cover_put() {
    let server = start_resolume(cover_layers(None, None, 0.0)).await;
    let (mut driver, status) = driver_for(&server);
    for (update, what) in [(clear_update(), "clear"), (verse_update(), "verse 1")] {
        driver.handle_bible(update, &status).await.expect(what);
    }
    let after_first = server.received_requests().await.expect("requests").len();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse 2");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![1.0, 0.0],
        "shown by the clear, hidden by the first verse, then left alone"
    );
    assert_eq!(
        connects(&requests[after_first..]).len(),
        4,
        "the second verse is connected"
    );
}

#[tokio::test]
async fn a_clear_while_the_bible_clips_are_invisible_still_shows_the_cover() {
    let mut layers = bible_layers(None);
    for clip in layers
        .iter_mut()
        .filter_map(|layer| layer["clips"].as_array_mut())
        .flatten()
    {
        clip["video"]["opacity"]["value"] = json!(0.0);
    }
    layers.push(cover_layer(None, 0.0));
    let server = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);

    for what in ["clear", "second clear"] {
        driver
            .handle_bible(clear_update(), &status)
            .await
            .expect(what);
    }

    let requests = server.received_requests().await.expect("requests");
    for param in A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY) {
        assert!(
            opacities_put(&requests, param).is_empty(),
            "param {param} is already invisible"
        );
    }
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![1.0],
        "the clear shows the cover although no verse was visible; the second sends nothing"
    );
    assert!(connects(&requests).is_empty());
}

/// The cover fade runs after the verse is connected and the lanes flipped. A
/// failure there must not fail the push: `dispatch_push` retries a failed push
/// whole, which would connect the verse again on the other side.
#[tokio::test]
async fn a_failed_cover_fade_does_not_fail_the_verse() {
    let server = start_resolume(cover_layers(None, None, 0.0)).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    answer_param(&server, COVER_OPACITY, 500, 10, 1).await;
    let after_clear = server.received_requests().await.expect("requests").len();

    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("a failed cover fade does not fail the verse");
    let requests = server.received_requests().await.expect("requests");
    let verse = &requests[after_clear..];
    assert_eq!(connects(verse).len(), 4, "the verse is on air");
    assert_eq!(
        opacities_put(verse, COVER_OPACITY),
        vec![0.0],
        "the cover fade was tried"
    );

    let after_verse = requests.len();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse 2");
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        values_put(&requests[after_verse..], 31).len(),
        1,
        "the next verse lands on #bible-b: the lanes flipped despite the failed cover fade"
    );
}

/// The operator dims the shown sermon title in Arena. The next fetch shows it,
/// the verse fades it from there, and the clear after that brings it back to
/// the dimmed opacity, not to full.
#[tokio::test]
async fn a_cover_dimmed_in_arena_comes_back_at_that_opacity() {
    let server = start_resolume(cover_layers(None, None, 0.0)).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("clear shows the cover");

    server.reset().await;
    mount_arena(&server, cover_layers(None, None, 0.6)).await;
    driver.refresh_mapping().await.expect("refetch");
    for (update, what) in [(verse_update(), "verse"), (clear_update(), "clear")] {
        driver.handle_bible(update, &status).await.expect(what);
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0, 0.6],
        "hidden from the dimmed value, then shown at it"
    );
}

/// A cover a previous process (or the operator) left at a lower opacity is
/// already shown: the clear leaves it, the verse hides it, and the next clear
/// brings it back to that opacity.
#[tokio::test]
async fn a_cover_mapped_below_full_opacity_is_shown_at_that_value() {
    let server = start_resolume(cover_layers(None, None, 0.6)).await;
    let (mut driver, status) = driver_for(&server);
    for (update, what) in [
        (clear_update(), "clear"),
        (verse_update(), "verse"),
        (clear_update(), "clear again"),
    ] {
        driver.handle_bible(update, &status).await.expect(what);
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0, 0.6],
        "nothing for the first clear, then hidden and shown at 0.6"
    );
}
