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
//! - a layer keeps at most four titles, the oldest not playing goes first;
//! - a title moved to another layer (a layer inserted in Arena) is remembered
//!   once.
//!
//! #848 fourth review: two titles of ONE deck on one layer are remembered
//! alike whichever column the playing one is in (a worker test and a pure
//! `remember_mapping` test); fifth review: so are two PLAYING titles and a
//! third one of one fetch, both playing titles kept. Pure `remember_mapping`
//! tests on small compositions also pin what the cap and layer-shift worker
//! tests cannot see: a full layer keeps its playing title and drops the
//! oldest that is not playing, and a title remembered on another layer
//! leaves its old one in the map itself (sixth review: through the fetch
//! path, since a lone-cover entry point existed only for these tests).
//!
//! A deck switch is `server.reset()` + a new mount + "Refresh mapping": the
//! composition lists only the new deck, while the other decks' titles still
//! exist in Arena (their parameters are kept in the new mount).

use super::bible_clear_cover_memory::{remember_mapping, RememberedCovers, REMEMBERED_PER_LAYER};
use super::bible_clear_cover_tests::{
    cover_layers, cover_layers_on, timer_layer, COVER, COVER_OPACITY, COVER_TEXT,
};
use super::bible_clear_cover_worker_tests::{
    cover_reads, param_requests, wait_for, wait_for_opacity, Worker, DECK_TWO,
};
use super::bible_clear_tests::{
    clear_update, clip, deck_layers, layer, mount_arena, opacities_put, start_arena, verse_update,
};
use super::clip_map::ClipMapping;
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

/// A layer remembers at most four titles; a fifth drops the oldest. Five
/// decks, each title on the same layer, none playing, so this cannot tell
/// "the oldest not playing" from "the first":
/// `a_full_layer_keeps_its_playing_title_and_drops_the_oldest_other` does.
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
/// the next layer, is read and hidden once. `cover_candidates` dedups by
/// parameter too, so this passes without the cross-layer forget;
/// `a_title_remembered_on_another_layer_leaves_its_old_layer` pins the map.
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

/// `clip` PLAYING (Resolume `connected`).
fn playing(mut clip: Value) -> Value {
    clip["connected"] = json!({ "value": "Connected" });
    clip
}

/// Two titles of the first deck for one layer, in column order: the sermon
/// title (not playing), then a second title PLAYING.
fn titles_playing_second() -> Vec<Value> {
    vec![
        clip(COVER, "NAZOV #bible-clear", COVER_TEXT, 0.0),
        playing(clip(
            COVER + 1,
            "INY NAZOV #bible-clear",
            COVER_TEXT + 1,
            0.0,
        )),
    ]
}

/// Title `n` of one deck (opacity parameter `COVER_OPACITY + n`), not
/// playing.
fn title_clip(n: i64) -> Value {
    clip(COVER + n, "NAZOV #bible-clear", COVER_TEXT + n, 0.0)
}

/// Remember the covers of one fetch of `layers`, as every fade scheduled
/// with that mapping does ([`remember_mapping`]).
fn remember_fetch(remembered: &mut RememberedCovers, layers: Vec<Value>) {
    let composition = json!({ "layers": layers });
    let mapping = ClipMapping::from_composition(&composition).expect("mapping");
    remember_mapping(remembered, &mapping);
}

/// Each layer's remembered opacity parameters, the oldest first, by layer.
fn layer_params(remembered: &RememberedCovers) -> Vec<(usize, Vec<i64>)> {
    let mut layers: Vec<(usize, Vec<i64>)> = remembered
        .iter()
        .map(|(index, covers)| (*index, covers.iter().map(|cover| cover.param_id).collect()))
        .collect();
    layers.sort_unstable_by_key(|(index, _)| *index);
    layers
}

/// #848 fourth review, finding 1: two titles of the first deck on one layer,
/// the second one PLAYING at the fetch. Remembered in column order, it
/// collapsed the layer after the first title was added, so the first was
/// lost. The operator then triggers the first title in Arena and switches to
/// the second deck: the verse there must hide it.
#[tokio::test]
async fn a_title_beside_a_later_playing_one_is_hidden_after_a_deck_switch() {
    let mut deck_a = deck_layers(0, None);
    deck_a.push(layer(titles_playing_second(), None));
    deck_a.push(timer_layer(0));
    let (server, _params) = start_arena(deck_a).await;
    let worker = Worker::spawn(&server);
    push_and_read(&server, &worker, clear_update(), COVER_OPACITY).await;

    // Both titles were shown; the first one now plays in Arena.
    let kept = [(COVER_OPACITY, 1.0), (COVER_OPACITY + 1, 1.0)];
    let deck_b = cover_layers_on(DECK_TWO, None, None, 0.0);
    switch_deck(&server, &worker, deck_b, &kept).await;
    let requests = push_and_read(&server, &worker, verse_update(), COVER_OPACITY + DECK_TWO).await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0],
        "the first title plays over the second deck, so the verse hides it"
    );
}

/// #848 fourth review, finding 1, pure: `schedule_cover_fade` remembers a
/// fetch's covers ([`remember_mapping`]), and a playing one collapsed its
/// layer when it was reached, so a title in a column BEFORE the playing one
/// was lost and the result depended on the columns. Both orders remember the
/// same.
#[test]
fn a_fetchs_titles_are_remembered_alike_in_either_column_order() {
    let in_order = titles_playing_second();
    let swapped: Vec<Value> = in_order.iter().rev().cloned().collect();
    let results: Vec<Vec<(usize, Vec<i64>)>> = [in_order, swapped]
        .into_iter()
        .map(|clips| {
            let composition = json!({ "layers": [layer(clips, None)] });
            let mapping = ClipMapping::from_composition(&composition).expect("mapping");
            let mut remembered = RememberedCovers::default();
            remember_mapping(&mut remembered, &mapping);
            layer_params(&remembered)
        })
        .collect();

    let expected = vec![(1, vec![COVER_OPACITY + 1, COVER_OPACITY])];
    assert_eq!(
        results[0], expected,
        "the playing title in the later column"
    );
    assert_eq!(
        results[1], expected,
        "the playing title in the earlier column"
    );
}

/// Three titles of the first deck for one layer, in column order: the sermon
/// title (not playing), then two titles PLAYING.
fn two_titles_playing() -> Vec<Value> {
    let mut clips = titles_playing_second();
    clips.push(playing(clip(
        COVER + 2,
        "TRETI NAZOV #bible-clear",
        COVER_TEXT + 2,
        0.0,
    )));
    clips
}

/// #848 fifth review, item 1: two titles PLAYING on one layer in one fetch.
/// Both came first, and the second one's collapse discarded the first, so the
/// later column won. A fetch collapses each layer at most once and keeps all
/// of its playing titles, then adds the others: both column orders remember
/// the same set, with both playing titles in it.
#[test]
fn a_fetchs_two_playing_titles_on_one_layer_are_both_remembered_in_either_column_order() {
    let in_order = two_titles_playing();
    let swapped: Vec<Value> = in_order.iter().rev().cloned().collect();
    let results: Vec<Vec<(usize, Vec<i64>)>> = [in_order, swapped]
        .into_iter()
        .map(|clips| {
            let composition = json!({ "layers": [layer(clips, None)] });
            let mapping = ClipMapping::from_composition(&composition).expect("mapping");
            let mut remembered = RememberedCovers::default();
            remember_mapping(&mut remembered, &mapping);
            let mut sets = layer_params(&remembered);
            for (_, params) in &mut sets {
                params.sort_unstable();
            }
            sets
        })
        .collect();

    let expected = vec![(1, vec![COVER_OPACITY, COVER_OPACITY + 1, COVER_OPACITY + 2])];
    assert_eq!(
        results[0], expected,
        "the playing titles in the later columns"
    );
    assert_eq!(
        results[1], expected,
        "the playing titles in the earlier columns"
    );
}

/// #848 fourth review, finding 3: the cap worker test has no playing title,
/// so dropping the first title unconditionally would pass it. A fetch whose
/// layer holds a PLAYING title and four others keeps the playing one and
/// drops the oldest one not playing.
#[test]
fn a_full_layer_keeps_its_playing_title_and_drops_the_oldest_other() {
    let others: Vec<i64> = (1..).take(REMEMBERED_PER_LAYER).collect();
    let mut clips = vec![playing(title_clip(0))];
    clips.extend(others.iter().map(|&n| title_clip(n)));
    let mut remembered = RememberedCovers::default();
    remember_fetch(&mut remembered, vec![layer(clips, None)]);

    let mut kept = vec![COVER_OPACITY];
    kept.extend(others[1..].iter().map(|&n| COVER_OPACITY + n));
    assert_eq!(
        layer_params(&remembered),
        vec![(1, kept)],
        "the playing title stays, the oldest one not playing ({}) goes",
        COVER_OPACITY + others[0]
    );
}

/// #848 fourth review, finding 3: `cover_candidates` dedups by parameter, so
/// the layer-shift worker test reads the title once with or without the
/// cross-layer forget. On the map itself, over three fetches: a title
/// remembered on another layer leaves its old one, and a layer left empty is
/// gone.
#[test]
fn a_title_remembered_on_another_layer_leaves_its_old_layer() {
    let mut remembered = RememberedCovers::default();
    remember_fetch(
        &mut remembered,
        vec![layer(vec![title_clip(0), title_clip(1)], None)],
    );

    let empty = || layer(Vec::new(), None);
    remember_fetch(
        &mut remembered,
        vec![empty(), layer(vec![title_clip(0)], None)],
    );
    assert_eq!(
        layer_params(&remembered),
        vec![(1, vec![COVER_OPACITY + 1]), (2, vec![COVER_OPACITY])]
    );

    let moved_playing = layer(vec![playing(title_clip(1))], None);
    remember_fetch(&mut remembered, vec![empty(), moved_playing]);
    assert_eq!(
        layer_params(&remembered),
        vec![(2, vec![COVER_OPACITY + 1])],
        "playing, it collapses its new layer, and its emptied old layer is gone"
    );
}
