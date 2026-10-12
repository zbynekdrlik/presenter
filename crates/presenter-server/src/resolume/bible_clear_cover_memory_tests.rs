//! #848 third review, finding 1: the `#bible-clear` covers a host remembers
//! across deck switches are a small SET per layer, not one cover per layer.
//!
//! On SNV every title sits on layer 12. "One cover per layer, replaced only by
//! one seen playing" therefore became "remember only playing covers": a title
//! the operator triggers in Arena after its deck's fetch was never remembered,
//! so after a switch back every verse hid the old title, never that one, and
//! it stayed over the verse. The REAL worker against the stateful mock Arena:
//!
//! - a title not playing at its deck's fetch is still remembered, so a verse
//!   on another deck hides it and a clear there shows it;
//! - a title seen PLAYING drops every other title remembered on its layer;
//! - a layer keeps at most four titles, the oldest goes first;
//! - a title moved to another layer (a layer inserted in Arena) is remembered
//!   once.
//!
//! A deck switch is `server.reset()` + a new mount + "Refresh mapping": the
//! composition lists only the new deck, while the other decks' titles still
//! exist in Arena (their parameters are kept in the new mount).

use super::bible_clear_cover_tests::{cover_layers, cover_layers_on, COVER_OPACITY};
use super::bible_clear_cover_worker_tests::{
    cover_reads, param_requests, wait_for, wait_for_opacity, Worker, DECK_TWO,
};
use super::bible_clear_tests::{
    clear_update, layer, mount_arena, opacities_put, start_arena, verse_update,
};
use super::BibleUpdate;
use serde_json::{json, Value};
use wiremock::{MockServer, Request};

/// Switch the mock to the deck `layers`; each `(param, opacity)` in `kept` is
/// a title of another deck that still exists in Arena at that opacity.
async fn switch_deck(
    server: &MockServer,
    worker: &Worker,
    layers: Vec<Value>,
    kept: &[(i64, f64)],
) {
    server.reset().await;
    let params = mount_arena(server, layers).await;
    for &(param, opacity) in kept {
        params.set(param, opacity);
    }
    worker.refresh_mapping().await;
}

/// Send `push` and wait until the cover fade it schedules has read `param`
/// and finished that first step (the barrier waits for the step in flight).
/// The titles here have no transition, so the first step sets each target.
async fn push_and_read(
    server: &MockServer,
    worker: &Worker,
    push: BibleUpdate,
    param: i64,
) -> Vec<Request> {
    worker.bible(push).await;
    wait_for(server, &format!("the cover read of {param}"), |requests| {
        param_requests(requests, "GET", param) >= 1
    })
    .await;
    worker.barrier(server).await
}

/// `layers` with its `#bible-clear` title PLAYING (Resolume `connected`).
fn with_title_playing(mut layers: Vec<Value>) -> Vec<Value> {
    let clips = layers
        .iter_mut()
        .filter_map(|layer| layer["clips"].as_array_mut())
        .flatten();
    for clip in clips {
        if clip["name"]["value"]
            .as_str()
            .is_some_and(|name| name.contains("#bible-clear"))
        {
            clip["connected"] = json!({ "value": "Connected" });
        }
    }
    layers
}

/// Deck A's title shown by a clear; deck B (its title on the SAME layer, not
/// playing) whose verse hides deck A's title. The operator then triggers deck
/// B's title in Arena, where it plays at `title_opacity`, and switches back to
/// deck A: Presenter only sees the switch.
async fn back_on_deck_a_with_deck_b_title(title_opacity: f64) -> (MockServer, Worker) {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let deck_b = cover_layers_on(DECK_TWO, None, None, 0.0);
    switch_deck(&server, &worker, deck_b, &[(COVER_OPACITY, 1.0)]).await;
    push_and_read(&server, &worker, verse_update(), COVER_OPACITY + DECK_TWO).await;

    let kept = [(COVER_OPACITY + DECK_TWO, title_opacity)];
    switch_deck(&server, &worker, cover_layers(None, None, 0.0), &kept).await;
    (server, worker)
}

/// The scenario both reviewers found: deck B's title, triggered in Arena
/// after deck B's fetch, plays on over deck A. The verse on deck A hides it.
#[tokio::test]
async fn a_title_triggered_after_its_decks_fetch_is_hidden_by_a_verse_on_another_deck() {
    let (server, worker) = back_on_deck_a_with_deck_b_title(1.0).await;

    let requests = push_and_read(&server, &worker, verse_update(), COVER_OPACITY).await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY + DECK_TWO),
        vec![0.0],
        "deck B's title still plays over deck A, so the verse hides it"
    );
}

/// The mirror: deck B's title plays hidden (a verse is on air), and a clear
/// on deck A shows it again.
#[tokio::test]
async fn a_title_triggered_after_its_decks_fetch_is_shown_by_a_clear_on_another_deck() {
    let (server, worker) = back_on_deck_a_with_deck_b_title(0.0).await;

    let requests = push_and_read(&server, &worker, clear_update(), COVER_OPACITY).await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY + DECK_TWO),
        vec![1.0],
        "deck B's title still plays on its layer, so the clear shows it"
    );
}

/// A title seen PLAYING collapses its layer's set to itself: Resolume plays
/// one clip per layer, so the titles remembered there are no longer visible.
#[tokio::test]
async fn a_title_seen_playing_drops_every_other_title_remembered_on_its_layer() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let deck_b = cover_layers_on(DECK_TWO, None, None, 0.0);
    switch_deck(&server, &worker, deck_b, &[(COVER_OPACITY, 1.0)]).await;
    push_and_read(&server, &worker, verse_update(), COVER_OPACITY + DECK_TWO).await;

    let deck_c = with_title_playing(cover_layers_on(2 * DECK_TWO, None, None, 1.0));
    let kept = [(COVER_OPACITY, 1.0), (COVER_OPACITY + DECK_TWO, 1.0)];
    switch_deck(&server, &worker, deck_c, &kept).await;
    let title_c = COVER_OPACITY + 2 * DECK_TWO;
    let requests = push_and_read(&server, &worker, verse_update(), title_c).await;

    for param in [COVER_OPACITY, COVER_OPACITY + DECK_TWO] {
        assert_eq!(
            param_requests(&requests, "GET", param) + param_requests(&requests, "PUT", param),
            0,
            "title {param} is no longer faded once deck C's title plays on its layer"
        );
    }
    assert_eq!(opacities_put(&requests, title_c), vec![0.0]);
}

/// A layer remembers at most four titles; a fifth drops the oldest that is
/// not playing. Five decks, each title on the same layer, none playing.
#[tokio::test]
async fn a_layer_remembers_at_most_four_titles_and_drops_the_oldest() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let titles: Vec<i64> = (0..5).map(|deck| COVER_OPACITY + deck * DECK_TWO).collect();
    for (deck, &title) in titles.iter().enumerate().take(4).skip(1) {
        // The earlier titles exist, hidden: the verse there sends them nothing.
        let kept: Vec<(i64, f64)> = titles[..deck].iter().map(|&param| (param, 0.0)).collect();
        let layers = cover_layers_on(title - COVER_OPACITY, None, None, 0.0);
        switch_deck(&server, &worker, layers, &kept).await;
        push_and_read(&server, &worker, verse_update(), title).await;
    }

    let kept: Vec<(i64, f64)> = titles[..4].iter().map(|&param| (param, 1.0)).collect();
    switch_deck(
        &server,
        &worker,
        cover_layers_on(4 * DECK_TWO, None, None, 0.0),
        &kept,
    )
    .await;
    let requests = push_and_read(&server, &worker, verse_update(), titles[4]).await;

    assert_eq!(
        param_requests(&requests, "GET", titles[0]) + param_requests(&requests, "PUT", titles[0]),
        0,
        "the oldest title was dropped when the fifth came"
    );
    for &param in &titles[1..4] {
        assert_eq!(
            opacities_put(&requests, param),
            vec![0.0],
            "title {param} is remembered, so the verse hides it"
        );
    }
}

/// A layer inserted in Arena shifts every layer index: the same title, now on
/// the next layer, is remembered ONCE (it was kept on both layers, so a later
/// fade read and PUT it twice per step).
#[tokio::test]
async fn a_title_moved_to_another_layer_is_remembered_once() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let mut shifted = vec![layer(Vec::new(), None)];
    shifted.extend(cover_layers(None, None, 1.0));
    switch_deck(&server, &worker, shifted, &[]).await;
    push_and_read(&server, &worker, verse_update(), COVER_OPACITY).await;

    let deck_b = cover_layers_on(DECK_TWO, None, None, 0.0);
    switch_deck(&server, &worker, deck_b, &[(COVER_OPACITY, 1.0)]).await;
    let requests = push_and_read(&server, &worker, verse_update(), COVER_OPACITY + DECK_TWO).await;

    assert_eq!(cover_reads(&requests), 1, "the moved title is read once");
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0],
        "and hidden once"
    );
}
