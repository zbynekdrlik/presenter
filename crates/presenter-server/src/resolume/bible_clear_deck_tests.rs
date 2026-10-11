//! #841 final-review follow-ups of the Bible clear fade, split from
//! `bible_clear_tests.rs` (whose helpers they share) to keep both files under
//! the file-size warning:
//!
//! - after a Resolume deck switch the layer keeps playing the previous deck's
//!   clip, so a clear must also fade the clips the verse connected;
//! - a fetch shows what Resolume really has, so the tracked opacities follow
//!   it (a clip shown again or dimmed in Arena);
//! - each layer fades over its own transition;
//! - after a start, the first verse lands on the side that is not live.

use super::bible_clear_tests::{
    answer_param, bible_layers, body_value, clear_update, clip, connects, deck_layers, driver_for,
    layer, mount_arena, opacities_put, param_path, position, start_resolume, values_put,
    verse_update, A_SIDE_OPACITY, BIBLE_A, B_SIDE_OPACITY, FADE,
};
use serde_json::json;
use std::time::Duration;
use wiremock::Request;

/// The id shift of the second deck in the deck-switch tests.
const DECK_TWO: i64 = 1000;

/// Every Bible opacity parameter of the second deck.
fn deck_two_opacity() -> impl Iterator<Item = i64> {
    A_SIDE_OPACITY
        .into_iter()
        .chain(B_SIDE_OPACITY)
        .map(|param| param + DECK_TWO)
}

/// Assert that every param in `params` was stepped down to 0 from near full
/// opacity: a fade, not a cut and not skipped.
fn assert_faded(requests: &[Request], params: impl IntoIterator<Item = i64>) {
    for param in params {
        let steps = opacities_put(requests, param);
        assert_eq!(
            steps.last(),
            Some(&0.0),
            "param {param} ends at 0: {steps:?}"
        );
        assert!(
            steps.windows(2).all(|pair| pair[1] < pair[0]),
            "param {param} only goes down: {steps:?}"
        );
        assert!(
            steps.len() >= 3 && steps[0] > 0.5,
            "param {param} is faded in steps from near full opacity: {steps:?}"
        );
    }
}

/// Review finding (#841): after a deck switch Resolume keeps playing the
/// previous deck's clip on its layer, but `/composition` lists only the new
/// deck. A clear that fades only the mapped clips left the verse on the wall;
/// on main the clear connected the new deck's clear clip on that layer, so this
/// worked. The clear must also fade the clips the verse connected.
#[tokio::test]
async fn a_clear_after_a_deck_switch_fades_the_clips_the_verse_left_playing() {
    let server = start_resolume(bible_layers(Some(FADE))).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    server.reset().await;
    mount_arena(&server, deck_layers(DECK_TWO, Some(FADE))).await;
    driver.refresh_mapping().await.expect("deck 2 mapping");
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");

    let requests = server.received_requests().await.expect("requests");
    assert!(connects(&requests).is_empty(), "a clear triggers no clip");
    assert_faded(&requests, A_SIDE_OPACITY);
    assert_faded(&requests, deck_two_opacity());
}

/// A verse clip that Resolume no longer knows (404) after a deck switch is
/// dropped from the fade. It is outside the mapping, so its 404 says nothing
/// about the mapping: no stale-mapping error, the clear still fades every other
/// clip, and the forgotten clip gets no further writes.
#[tokio::test]
async fn a_verse_clip_gone_after_a_deck_switch_is_forgotten_not_a_stale_mapping() {
    let server = start_resolume(bible_layers(Some(FADE))).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    server.reset().await;
    answer_param(&server, 130, 404, 10, 1).await;
    mount_arena(&server, deck_layers(DECK_TWO, Some(FADE))).await;
    driver.refresh_mapping().await.expect("deck 2 mapping");
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("a 404 outside the mapping does not fail the clear");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, 130).len(),
        1,
        "the gone clip is tried once, then forgotten"
    );
    assert_faded(&requests, [135, 140, 145]);
    assert_faded(&requests, deck_two_opacity());
    let after_clear = requests.len();

    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse on deck 2");
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("second clear");
    let requests = server.received_requests().await.expect("requests");
    assert!(
        opacities_put(&requests[after_clear..], 130).is_empty(),
        "a forgotten clip is not written again"
    );
}

/// Review finding (#841): a fetch shows what Resolume really has, so the tracked
/// level must follow it. Someone shows the faded clips again in Arena; the next
/// fetch sees them at full opacity, and the next clear fades them instead of
/// skipping them as already invisible.
#[tokio::test]
async fn a_clip_shown_again_in_arena_after_a_clear_is_faded_by_the_next_clear() {
    let server = start_resolume(bible_layers(Some(FADE))).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("first clear");

    // The mock composition still reports every clip at 1.0: shown again.
    driver.refresh_mapping().await.expect("refetch");
    let after_first = server.received_requests().await.expect("requests").len();
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("second clear");

    let requests = server.received_requests().await.expect("requests");
    assert_faded(
        &requests[after_first..],
        A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY),
    );
}

/// Each clip fades over its OWN layer's transition, and a clip that reached 0
/// gets no further steps while a slower layer is still fading.
#[tokio::test]
async fn each_layer_fades_over_its_own_transition() {
    let mut layers = bible_layers(Some(FADE));
    layers[1] = layer(
        vec![
            clip(BIBLE_A, "#bible-a", 30, 1.0),
            clip(301, "#bible-b", 31, 1.0),
        ],
        Some(Duration::from_millis(150)),
    );
    let server = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");

    let requests = server.received_requests().await.expect("requests");
    assert_faded(&requests, [135, 136, 140, 141, 145, 146]);
    for param in [130, 131] {
        let steps = opacities_put(&requests, param);
        assert_eq!(steps.last(), Some(&0.0), "param {param}: {steps:?}");
        assert_eq!(
            steps.iter().filter(|level| **level == 0.0).count(),
            1,
            "param {param} gets no step after reaching 0: {steps:?}"
        );
    }
    let first_zero = |param: i64| {
        let wanted = param_path(param);
        position(&requests, |req| {
            req.url.path() == wanted && body_value(req).and_then(|v| v.as_f64()) == Some(0.0)
        })
    };
    assert!(
        first_zero(130) < first_zero(135),
        "the faster text layer reaches 0 before the reference layer"
    );
}

/// Final review (#841, Minor #1): an operator dims a visible Bible clip in Arena.
/// The next fetch shows it, the clear fades it from there, and when that side
/// is connected again it comes back at the dimmed opacity, not at full.
#[tokio::test]
async fn a_clip_dimmed_in_arena_keeps_that_opacity_after_the_next_verse() {
    let server = start_resolume(bible_layers(None)).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse on A");

    let mut dimmed = bible_layers(None);
    dimmed[1] = layer(
        vec![
            clip(BIBLE_A, "#bible-a", 30, 0.5),
            clip(301, "#bible-b", 31, 1.0),
        ],
        None,
    );
    server.reset().await;
    mount_arena(&server, dimmed).await;
    driver.refresh_mapping().await.expect("refetch");
    for (update, what) in [
        (clear_update(), "clear"),
        (verse_update(), "verse on B"),
        (verse_update(), "verse on A again"),
    ] {
        driver.handle_bible(update, &status).await.expect(what);
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, 130),
        vec![0.0, 0.5],
        "faded from the dimmed value, then restored to it"
    );
}

/// Final review (#841, Minor #5): a Bible clip whose composition opacity is
/// below full is faded from that value and restored to it.
#[tokio::test]
async fn a_clip_mapped_below_full_opacity_is_restored_to_that_value() {
    let mut layers = bible_layers(None);
    layers[1] = layer(
        vec![
            clip(BIBLE_A, "#bible-a", 30, 0.6),
            clip(301, "#bible-b", 31, 1.0),
        ],
        None,
    );
    let server = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);
    for (update, what) in [
        (verse_update(), "verse on A"),
        (clear_update(), "clear"),
        (verse_update(), "verse on B"),
        (verse_update(), "verse on A again"),
    ] {
        driver.handle_bible(update, &status).await.expect(what);
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, 130),
        vec![0.0, 0.6],
        "no restore before the first verse, then faded and restored to 0.6"
    );
}

/// Final review (#841, Minor #3): after a start the lanes follow the clip
/// Resolume shows connected. `#bible-a` is live, so the first verse fills and
/// connects the B side (a crossfade) instead of rewriting the live clip.
#[tokio::test]
async fn the_first_verse_after_a_start_lands_on_the_side_that_is_not_live() {
    let mut layers = bible_layers(None);
    let mut live = clip(BIBLE_A, "#bible-a", 30, 1.0);
    live["connected"] = json!({ "value": "Connected" });
    layers[1] = layer(vec![live, clip(301, "#bible-b", 31, 1.0)], None);
    let server = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    let requests = server.received_requests().await.expect("requests");
    assert!(
        values_put(&requests, 30).is_empty(),
        "the live #bible-a text is not rewritten"
    );
    assert_eq!(
        values_put(&requests, 31).len(),
        1,
        "the verse goes to #bible-b"
    );
    let connected: Vec<&str> = connects(&requests)
        .iter()
        .map(|req| req.url.path())
        .collect();
    assert!(
        connected.contains(&"/api/v1/composition/clips/by-id/301/connect"),
        "{connected:?}"
    );
    assert!(
        !connected.contains(&"/api/v1/composition/clips/by-id/300/connect"),
        "{connected:?}"
    );
}
