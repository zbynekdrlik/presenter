//! #848 second review: the steps of the deferred `#bible-clear` cover fade.
//!
//! The REAL worker (`run_host_worker`) against the stateful mock Arena, with
//! the `Worker` and the waits of `bible_clear_cover_worker_tests.rs`:
//!
//! - a fade lasts for the share of its transition that the distance left is
//!   of the cover's base, so a hide right after a short show is short;
//! - the first step starts each cover where it is: it PUTs nothing;
//! - a failed step jumps to ONE final step that PUTs the target, so one
//!   transient error never leaves the cover halfway;
//! - covers are remembered per layer: a newer cover PLAYING on the layer
//!   replaces the remembered one;
//! - every branch a cover 404 takes (refetch once, forget, paused, newer
//!   mapping), a failed read, and a config change mid-fade;
//! - third review: a failed step's final step is the LAST PUT, and a mapped
//!   cover's 404 leaves only that cover out of the fade.
//!
//! Step counts are bounded by the step spacing (a step is due `FADE_STEP`
//! after the previous one ended), never by wall-clock time.

use super::bible_clear::FADE_STEP;
use super::bible_clear_cover_tests::{
    assert_hidden, cover_layer, cover_layers, timer_layer, BIBLE_FADE, COVER, COVER_FADE,
    COVER_OPACITY, COVER_TEXT, SHORT_COVER_FADE, TIMER_TEXT,
};
use super::bible_clear_cover_worker_tests::{
    answer_param_read, assert_healthy, compositions, cover_reads, is_connect, last_index,
    param_requests, settled, wait_for, wait_for_opacity, Worker, DECK_TWO,
};
use super::bible_clear_tests::{
    answer_param, clear_update, clip, deck_layers, layer, mount_arena, opacities_put, param_path,
    start_arena, verse_update,
};
use super::driver::HostCommand;
use chrono::Utc;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wiremock::{MockServer, Request};

/// Indices of the composition reads among `requests`.
fn composition_reads(requests: &[Request]) -> Vec<usize> {
    requests
        .iter()
        .enumerate()
        .filter(|(_, req)| req.method.as_str() == "GET" && req.url.path() == "/api/v1/composition")
        .map(|(index, _)| index)
        .collect()
}

fn is_cover_read(req: &Request) -> bool {
    req.method.as_str() == "GET" && req.url.path() == param_path(COVER_OPACITY)
}

/// The second deck with its title on ANOTHER layer than the first deck's (the
/// first deck's layer then holds the timer), shown at `cover_opacity`.
fn deck_two_title_elsewhere(cover_transition: Option<Duration>, cover_opacity: f64) -> Vec<Value> {
    let mut layers = deck_layers(DECK_TWO, None);
    layers.push(timer_layer(DECK_TWO));
    layers.push(cover_layer(DECK_TWO, cover_transition, cover_opacity));
    layers
}

/// Switch the mock to the second deck (`layers`): the composition lists only
/// its clips, while the first deck's title still plays at full opacity.
async fn switch_to(server: &MockServer, worker: &Worker, layers: Vec<Value>) {
    server.reset().await;
    let params = mount_arena(server, layers).await;
    params.set(COVER_OPACITY, 1.0); // the first deck's title still exists
    worker.refresh_mapping().await;
}

/// Wait until the #484 backoff window that a failed push opened has passed.
async fn wait_out_backoff(worker: &Worker) {
    let start = Instant::now();
    loop {
        let retry_at = worker.status().await.next_retry_at;
        if retry_at.is_some_and(|at| (Utc::now() - at).num_milliseconds() > 50) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the backoff window never passed: {retry_at:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Finding 1: a verse that cut the clear's show short hid the cover over its
/// WHOLE transition, from the little opacity the show had reached, so quick
/// verses stretched the hide (the title showed behind the verse ~2.3 s on
/// SNV). The hide now lasts `fade × reached / base`, so it sends at most that
/// over `FADE_STEP` plus the final step.
#[tokio::test]
async fn a_hide_right_after_a_short_show_lasts_only_for_the_distance_left() {
    let (server, _params) = start_arena(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for(&server, "the show under way", |requests| {
        opacities_put(requests, COVER_OPACITY)
            .last()
            .is_some_and(|last| *last > 0.0)
    })
    .await;

    worker.bible(verse_update()).await;
    let requests = wait_for(&server, "the cover hidden", |requests| {
        last_index(requests, is_connect).is_some_and(|connect| {
            opacities_put(&requests[connect..], COVER_OPACITY).last() == Some(&0.0)
        })
    })
    .await;

    let first_connect = requests.iter().position(is_connect).expect("connects");
    let last_connect = last_index(&requests, is_connect).expect("connects");
    let reached = opacities_put(&requests[..first_connect], COVER_OPACITY)
        .last()
        .copied()
        .unwrap_or_default();
    let hidden = opacities_put(&requests[last_connect..], COVER_OPACITY);
    let left = COVER_FADE.as_secs_f64() * reached;
    let most = (left / FADE_STEP.as_secs_f64()).floor() as usize + 2;
    assert!(
        hidden.len() <= most,
        "a hide from {reached} lasts {left:.3} s, so at most {most} steps; got {}: {hidden:?}",
        hidden.len()
    );
}

/// Finding 11: the first PUT of every fade re-sent the opacity the cover
/// started from. The first step only reads and plans, so the first PUT is at
/// least one step into the fade, up and down.
#[tokio::test]
async fn a_cover_fade_never_puts_the_opacity_it_starts_from() {
    let (server, _params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    let one_step = FADE_STEP.as_secs_f64() / SHORT_COVER_FADE.as_secs_f64();

    worker.bible(clear_update()).await;
    let shown = opacities_put(
        &wait_for_opacity(&server, COVER_OPACITY, 1.0).await,
        COVER_OPACITY,
    );
    worker.bible(verse_update()).await;
    let requests = wait_for_opacity(&server, COVER_OPACITY, 0.0).await;
    let hidden = opacities_put(&requests, COVER_OPACITY)[shown.len()..].to_vec();

    assert!(
        shown[0] >= one_step - 1e-9,
        "the show's first PUT is a step up from 0: {shown:?}"
    );
    assert!(
        hidden[0] <= 1.0 - one_step + 1e-9,
        "the hide's first PUT is a step down from 1: {hidden:?}"
    );
}

/// Finding 3: one transient PUT error stopped the fade and left the cover
/// halfway (the title half visible). A failed step now jumps to ONE final
/// step that PUTs the target `FADE_STEP` later. The third PUT is the one that
/// fails (the first two land on a route ahead of the stateful one), so the
/// PUT right after it must be the target and the LAST one: the requests are
/// read again `5 × FADE_STEP` after the barrier (#848 third review, finding
/// 2: the barrier runs before a further step could be due).
#[tokio::test]
async fn one_failed_cover_step_jumps_to_a_final_step_at_the_target() {
    let (server, _params) = start_arena(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    answer_param(&server, COVER_OPACITY, 200, 2, 1).await;
    answer_param(&server, COVER_OPACITY, 500, 1, 2).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    worker.barrier(&server).await;
    let requests = settled(&server).await;

    let shown = opacities_put(&requests, COVER_OPACITY);
    assert_eq!(
        shown.len(),
        4,
        "two steps, the one that failed, then ONE final step: {shown:?}"
    );
    assert!(
        shown[..3].windows(2).all(|pair| pair[1] > pair[0]),
        "the cover only goes up: {shown:?}"
    );
    assert!(
        (shown[3] - 1.0).abs() < 1e-9,
        "the PUT after the failed one is the target, and the last: {shown:?}"
    );
    assert_healthy(&worker).await;
}

/// Finding 4: remembered covers only grew. They are kept per layer: the
/// second deck's title PLAYING on the first deck's title layer replaces the
/// remembered one (Resolume plays one clip per layer), so the next verse
/// leaves the first deck's title alone. A title that is NOT playing keeps the
/// remembered one (`a_remembered_cover_is_hidden_after_a_deck_switch`).
#[tokio::test]
async fn a_newer_cover_playing_on_the_same_layer_replaces_the_remembered_one() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let mut playing = cover_layer(DECK_TWO, None, 1.0);
    playing["clips"][0]["connected"] = json!({ "value": "Connected" });
    let mut layers = deck_layers(DECK_TWO, None);
    layers.push(playing);
    layers.push(timer_layer(DECK_TWO));
    switch_to(&server, &worker, layers).await;

    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY + DECK_TWO, 0.0).await;
    let requests = worker.barrier(&server).await;

    assert_eq!(
        param_requests(&requests, "GET", COVER_OPACITY)
            + param_requests(&requests, "PUT", COVER_OPACITY),
        0,
        "the first deck's title is no longer faded once a newer one plays on its layer"
    );
}

/// Finding 2: a mapped cover whose read answers but whose PUT is a 404 makes
/// the mapping stale: ONE refetch, and the same id failing on the refetched
/// mapping refetches nothing.
#[tokio::test]
async fn a_mapped_cover_put_404_refetches_the_mapping_once() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    answer_param(&server, COVER_OPACITY, 404, 100, 1).await;
    let worker = Worker::spawn(&server);

    worker.bible(clear_update()).await;
    wait_for(&server, "the clear's cover PUT", |requests| {
        param_requests(requests, "PUT", COVER_OPACITY) == 1
    })
    .await;
    worker.barrier(&server).await;
    worker.bible(clear_update()).await;
    wait_for(&server, "the second clear's cover PUT", |requests| {
        param_requests(requests, "PUT", COVER_OPACITY) == 2
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(cover_reads(&requests), 2, "each fade read the cover first");
    assert_eq!(
        compositions(&requests),
        2,
        "the cold fetch and ONE stale-id refetch (by the barrier's timer frame)"
    );
    assert_healthy(&worker).await;
}

/// Finding 2: a remembered cover whose PUT is a 404 in the middle of a fade
/// is forgotten (no refetch), and the other cover keeps fading to its target.
#[tokio::test]
async fn a_remembered_cover_gone_mid_fade_is_forgotten_and_the_others_keep_fading() {
    let (server, _params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    switch_to(
        &server,
        &worker,
        deck_two_title_elsewhere(Some(SHORT_COVER_FADE), 1.0),
    )
    .await;
    answer_param(&server, COVER_OPACITY, 404, 100, 1).await;

    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY + DECK_TWO, 0.0).await;
    let requests = worker.barrier(&server).await;

    assert_eq!(cover_reads(&requests), 1, "the remembered title answered");
    assert_eq!(
        param_requests(&requests, "PUT", COVER_OPACITY),
        1,
        "its PUT found nothing, so it is forgotten"
    );
    assert_hidden(&opacities_put(&requests, COVER_OPACITY + DECK_TWO));
    assert_eq!(
        compositions(&requests),
        1,
        "only the manual refresh: a gone remembered cover refetches nothing"
    );
    assert_healthy(&worker).await;
}

/// #848 third review, finding 4: a mapped cover's 404 stopped EVERY cover in
/// the fade, so a remembered title still playing stayed over the verse. The
/// second deck's title (mapped, on another layer) has no opacity parameter on
/// its READ: it alone is left out, the first deck's remembered title is still
/// hidden, and the mapping is refetched once.
#[tokio::test]
async fn a_mapped_cover_read_404_leaves_the_other_covers_fading() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    switch_to(&server, &worker, deck_two_title_elsewhere(None, 1.0)).await;
    answer_param_read(&server, COVER_OPACITY + DECK_TWO, 404, 100).await;

    worker.bible(verse_update()).await;
    wait_for(&server, "the verse's cover reads", |requests| {
        cover_reads(requests) == 1 && param_requests(requests, "GET", COVER_OPACITY + DECK_TWO) == 1
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0],
        "the remembered title is still hidden"
    );
    assert_eq!(
        param_requests(&requests, "PUT", COVER_OPACITY + DECK_TWO),
        0
    );
    assert_eq!(
        compositions(&requests),
        2,
        "the manual refresh and ONE stale-id refetch (by the barrier's timer frame)"
    );
    assert_healthy(&worker).await;
}

/// #848 third review, finding 4: the same in the middle of a fade. The second
/// deck's title answers its first PUT with 404; it alone is left out and the
/// first deck's remembered title fades on down to 0.
#[tokio::test]
async fn a_mapped_cover_put_404_mid_fade_leaves_the_other_covers_fading() {
    let (server, _params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    switch_to(
        &server,
        &worker,
        deck_two_title_elsewhere(Some(SHORT_COVER_FADE), 1.0),
    )
    .await;
    answer_param(&server, COVER_OPACITY + DECK_TWO, 404, 100, 1).await;

    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 0.0).await;
    let requests = worker.barrier(&server).await;

    assert_hidden(&opacities_put(&requests, COVER_OPACITY));
    assert_eq!(
        param_requests(&requests, "PUT", COVER_OPACITY + DECK_TWO),
        1,
        "its first PUT found nothing, so it is left out of the fade"
    );
    assert_eq!(
        compositions(&requests),
        2,
        "the manual refresh and ONE stale-id refetch (by the barrier's timer frame)"
    );
    assert_healthy(&worker).await;
}

/// #848 third review, finding 4: a mapped cover's 404 no longer stops the
/// fade, so a second broken cover is reached too. Each broken id refetches
/// the mapping ONCE (the refetched mapping lists it again), so two of them
/// cost two refetches in all and never take turns refetching on every fade.
#[tokio::test]
async fn two_broken_mapped_covers_refetch_the_mapping_once_each() {
    let second = COVER_OPACITY + 1;
    let mut layers = cover_layers(None, None, 0.0);
    layers.push(layer(
        vec![clip(COVER + 1, "#bible-clear", COVER_TEXT + 1, 0.0)],
        None,
    ));
    let (server, _params) = start_arena(layers).await;
    answer_param_read(&server, COVER_OPACITY, 404, 100).await;
    answer_param_read(&server, second, 404, 100).await;
    let worker = Worker::spawn(&server);

    let pushes = [
        clear_update(),
        verse_update(),
        verse_update(),
        verse_update(),
    ];
    for (fade, push) in pushes.into_iter().enumerate() {
        worker.bible(push).await;
        wait_for(&server, "both covers read", |requests| {
            cover_reads(requests) > fade && param_requests(requests, "GET", second) > fade
        })
        .await;
        worker.barrier(&server).await;
    }
    let requests = server.received_requests().await.expect("requests");

    assert_eq!(
        compositions(&requests),
        3,
        "the cold fetch and ONE stale-id refetch per broken cover"
    );
    assert_healthy(&worker).await;
}

/// Finding 2: a failed read (5xx) keeps the tracked level. The operator
/// dimmed the shown title to 0.4, but the read could not report it, so the
/// hide starts from the 1.0 Presenter tracks.
#[tokio::test]
async fn a_failed_cover_read_fades_from_the_tracked_opacity() {
    let (server, params) = start_arena(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    let after_clear = wait_for_opacity(&server, COVER_OPACITY, 1.0).await.len();
    params.set(COVER_OPACITY, 0.4);
    answer_param_read(&server, COVER_OPACITY, 500, 1).await;

    worker.bible(verse_update()).await;
    let requests = wait_for(&server, "the cover hidden", |requests| {
        requests.len() > after_clear
            && opacities_put(&requests[after_clear..], COVER_OPACITY).last() == Some(&0.0)
    })
    .await;

    let steps = opacities_put(&requests[after_clear..], COVER_OPACITY);
    assert!(
        steps[0] > 0.4,
        "hidden from the tracked 1.0, not from Arena's unread 0.4: {steps:?}"
    );
    assert_hidden(&steps);
    assert_healthy(&worker).await;
}

/// Finding 2: while #808 pauses stale-id refetches (a timer frame got a 404
/// on the mapping it fetched itself), a mapped cover's 404 refetches nothing.
#[tokio::test]
async fn a_mapped_cover_404_while_stale_refetches_are_paused_refetches_nothing() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    answer_param(&server, TIMER_TEXT, 404, 1, 1).await;
    answer_param_read(&server, COVER_OPACITY, 404, 100).await;
    let worker = Worker::spawn(&server);
    worker.timer().await;
    wait_for(&server, "the rejected timer frame", |requests| {
        param_requests(requests, "PUT", TIMER_TEXT) == 1
    })
    .await;
    wait_out_backoff(&worker).await;

    worker.bible(clear_update()).await;
    wait_for(&server, "the clear's cover read", |requests| {
        cover_reads(requests) == 1
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(compositions(&requests), 1, "the cold fetch only");
    assert_healthy(&worker).await;
}

/// Finding 2: the operator's "Refresh mapping" ran between the clear and its
/// first cover step (it waited for the clear's Bible fade, and commands go
/// first), so the cover's 404 belongs to an older mapping: nothing is
/// invalidated, the barrier's timer frame refetches nothing.
#[tokio::test]
async fn a_mapped_cover_404_after_a_newer_mapping_was_fetched_refetches_nothing() {
    let (server, _params) = start_arena(cover_layers(Some(BIBLE_FADE), None, 0.0)).await;
    answer_param_read(&server, COVER_OPACITY, 404, 100).await;
    let worker = Worker::spawn(&server);

    worker.bible(clear_update()).await;
    worker.refresh_mapping().await;
    wait_for(&server, "the cover read", |requests| {
        cover_reads(requests) == 1
    })
    .await;
    let requests = worker.barrier(&server).await;

    let refreshed = composition_reads(&requests)[1];
    let read = requests.iter().position(is_cover_read).expect("cover read");
    assert!(
        refreshed < read,
        "the cover step ran after the refresh (composition #{refreshed}, read #{read})"
    );
    assert_eq!(
        compositions(&requests),
        2,
        "the cold fetch and the manual refresh only"
    );
    assert_healthy(&worker).await;
}

/// Finding 2: a config change mid-fade drops the fade with the rest of the
/// host state: no cover PUT after it, and the cover stays where the fade got.
#[tokio::test]
async fn a_config_change_mid_fade_drops_the_cover_fade() {
    let (server, _params) = start_arena(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    let start = Instant::now();
    worker.bible(clear_update()).await;
    wait_for(&server, "the show under way", |requests| {
        param_requests(requests, "PUT", COVER_OPACITY) >= 2
    })
    .await;

    worker
        .send(HostCommand::RefreshConfig(worker.config.clone()))
        .await;
    let barrier = worker.barrier(&server).await;
    let refetched = composition_reads(&barrier)[1];
    // Only waiting out the time the dropped fade would still run proves it
    // does not.
    let requests = wait_for(&server, "the end of the dropped fade's time", |_| {
        start.elapsed() >= COVER_FADE + FADE_STEP * 10
    })
    .await;

    let late = opacities_put(&requests[refetched..], COVER_OPACITY);
    assert!(
        late.is_empty(),
        "the fade went on after the config change: {late:?}"
    );
    let shown = opacities_put(&requests, COVER_OPACITY);
    assert!(
        shown.last().is_some_and(|last| *last < 1.0 - 1e-9),
        "the cover stays where the dropped fade got: {shown:?}"
    );
    // A config change leaves the host `Connecting` until a stage/Bible push
    // or a tick (timer frames never mark it connected), so only no failure
    // and no backoff window are asserted.
    let snap = worker.status().await;
    assert_eq!(snap.consecutive_failures, 0, "{snap:?}");
    assert!(snap.next_retry_at.is_none(), "no backoff window: {snap:?}");
}
