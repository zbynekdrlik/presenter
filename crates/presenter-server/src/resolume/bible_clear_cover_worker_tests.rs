//! #848 review: the `#bible-clear` cover fade is a deferred, best-effort,
//! interruptible step machine in the host worker, not part of the push.
//!
//! These tests run the REAL worker (`run_host_worker`) against a mock Arena
//! and read the requests it received, in arrival order. The contract:
//!
//! - a clear schedules a cover SHOW after its Bible fade, a connected verse a
//!   HIDE after its connects; other commands (a timer frame, a verse) run
//!   between two cover steps, and a new schedule replaces the running fade
//!   from where it got;
//! - the first step reads each cover's opacity in Arena and plans from there:
//!   a change made in Arena counts, and a cover already at its target gets no
//!   PUT;
//! - a cover failure never fails the push and never touches the host status
//!   or the #484 backoff; a mapped cover's 404 refetches the mapping once;
//! - covers are remembered across a deck switch; a remembered cover answering
//!   404 is forgotten.
//!
//! A command only waits for a cover step that already started, so "the step
//! ran" is proven by the cover's opacity READ (a GET), never by a sleep.
//! Timing is asserted as a lower bound only.

use super::bible_clear::FADE_STEP;
use super::bible_clear_cover_tests::{
    assert_hidden, assert_shown, cover_layer, cover_layers, cover_layers_on, timer_layer,
    BIBLE_FADE, COVER, COVER_FADE, COVER_OPACITY, SHORT_COVER_FADE,
};
use super::bible_clear_deck_tests::assert_faded;
use super::bible_clear_tests::{
    answer_param, bible_layers, body_value, clear_update, connects, driver_for, mount_arena,
    opacities_put, param_path, start_arena, values_put, verse_update, ArenaParams, A_SIDE_OPACITY,
    B_SIDE_OPACITY,
};
use super::driver::{run_host_worker, HostCommand};
use super::port_claims::PortClaims;
use super::{BibleUpdate, ResolumeConnectionSnapshot, ResolumeConnectionState, TimerFrame};
use presenter_core::ResolumeHost;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, RwLock};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// How long a test waits for the worker before it fails.
const WAIT_LIMIT: Duration = Duration::from_secs(10);
/// The second deck's id shift in the deck-switch tests.
pub(super) const DECK_TWO: i64 = 1000;

/// A host worker running against `server`, as `ResolumeRegistry` spawns it.
pub(super) struct Worker {
    commands: mpsc::Sender<HostCommand>,
    status: Arc<RwLock<ResolumeConnectionSnapshot>>,
    timer_frames: AtomicU32,
    /// The host config it runs with (a `RefreshConfig` resends it).
    pub(super) config: ResolumeHost,
}

impl Worker {
    pub(super) fn spawn(server: &MockServer) -> Self {
        let (driver, _) = driver_for(server);
        let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));
        let (commands, receiver) = mpsc::channel(16);
        tokio::spawn(run_host_worker(
            driver.client.clone(),
            driver.config.clone(),
            Arc::clone(&status),
            receiver,
            None,
            None,
            PortClaims::default(),
        ));
        Self {
            commands,
            status,
            timer_frames: AtomicU32::new(0),
            config: driver.config.clone(),
        }
    }

    pub(super) async fn send(&self, command: HostCommand) {
        self.commands.send(command).await.expect("the worker runs");
    }

    pub(super) async fn bible(&self, update: BibleUpdate) {
        self.send(HostCommand::Bible(update)).await;
    }

    /// Send a timer frame with a new text and return that text.
    pub(super) async fn timer(&self) -> serde_json::Value {
        let n = self.timer_frames.fetch_add(1, Ordering::Relaxed);
        let text = format!("00:{n:02}");
        self.send(HostCommand::Timer(TimerFrame::new(text.clone())))
            .await;
        json!(text)
    }

    /// A timer frame, waited for: every command sent before it, and the cover
    /// step that was running when it arrived, are done. Matched by its unique
    /// text, so it works on any deck (a deck switch shifts the timer's ids).
    pub(super) async fn barrier(&self, server: &MockServer) -> Vec<Request> {
        let text = self.timer().await;
        wait_for(server, "the barrier timer frame", |requests| {
            requests
                .iter()
                .any(|req| req.method.as_str() == "PUT" && body_value(req).as_ref() == Some(&text))
        })
        .await
    }

    /// The operator's "Refresh mapping", waited for.
    pub(super) async fn refresh_mapping(&self) {
        let (reply, answer) = oneshot::channel();
        self.send(HostCommand::RefreshMapping(reply)).await;
        assert!(answer.await.expect("reply").success, "mapping refreshed");
    }

    pub(super) async fn status(&self) -> ResolumeConnectionSnapshot {
        self.status.read().await.clone()
    }
}

/// Poll the mock until `done` holds for the requests it received.
pub(super) async fn wait_for(
    server: &MockServer,
    what: &str,
    done: impl Fn(&[Request]) -> bool,
) -> Vec<Request> {
    let start = Instant::now();
    loop {
        let requests = server.received_requests().await.expect("requests");
        if done(&requests) {
            return requests;
        }
        assert!(
            start.elapsed() < WAIT_LIMIT,
            "timed out waiting for {what}; cover PUTs {:?}, {} requests",
            opacities_put(&requests, COVER_OPACITY),
            requests.len()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Wait a bounded `5 × FADE_STEP`, then return every request received.
/// Proving an ABSENCE (no step after a fade's last one) is the one wait that
/// is not a poll for a request: a 20 Hz retry loop would send several steps
/// in that time (#848 third review, finding 2).
pub(super) async fn settled(server: &MockServer) -> Vec<Request> {
    tokio::time::sleep(FADE_STEP * 5).await;
    server.received_requests().await.expect("requests")
}

/// Wait until the last opacity PUT of `param` is `value`.
pub(super) async fn wait_for_opacity(server: &MockServer, param: i64, value: f64) -> Vec<Request> {
    wait_for(server, &format!("param {param} at {value}"), |requests| {
        opacities_put(requests, param)
            .last()
            .is_some_and(|last| (last - value).abs() < 1e-9)
    })
    .await
}

/// Requests of `method` to parameter `param`, any answer.
pub(super) fn param_requests(requests: &[Request], method: &str, param: i64) -> usize {
    let wanted = param_path(param);
    requests
        .iter()
        .filter(|req| req.method.as_str() == method && req.url.path() == wanted)
        .count()
}

/// The cover's opacity reads: one per cover fade's first step.
pub(super) fn cover_reads(requests: &[Request]) -> usize {
    param_requests(requests, "GET", COVER_OPACITY)
}

pub(super) fn compositions(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|req| req.method.as_str() == "GET" && req.url.path() == "/api/v1/composition")
        .count()
}

/// Index of the last request matching `predicate`.
pub(super) fn last_index(
    requests: &[Request],
    predicate: impl Fn(&Request) -> bool,
) -> Option<usize> {
    requests.iter().rposition(predicate)
}

pub(super) fn is_connect(req: &Request) -> bool {
    req.url.path().ends_with("/connect")
}

fn is_cover_put(req: &Request) -> bool {
    req.method.as_str() == "PUT" && req.url.path() == param_path(COVER_OPACITY)
}

/// Answer `status` to `times` GETs of parameter `param_id`, ahead of the
/// stateful catch-all route.
pub(super) async fn answer_param_read(server: &MockServer, param_id: i64, status: u16, times: u64) {
    Mock::given(method("GET"))
        .and(path(param_path(param_id)))
        .respond_with(ResponseTemplate::new(status))
        .up_to_n_times(times)
        .with_priority(1)
        .mount(server)
        .await;
}

/// The host stayed healthy: no failure counted, no backoff window.
pub(super) async fn assert_healthy(worker: &Worker) {
    let snap = worker.status().await;
    assert_eq!(snap.state, ResolumeConnectionState::Connected, "{snap:?}");
    assert_eq!(snap.consecutive_failures, 0, "{snap:?}");
    assert!(snap.next_retry_at.is_none(), "no backoff window: {snap:?}");
}

/// The clear's push ends after the Bible fade; the cover then fades up over
/// its own transition while the worker keeps taking commands: a timer frame
/// sent mid-fade lands before the fade ends (the `#timer` countdown on SNV
/// layer 15 must not freeze for 2.3 s).
#[tokio::test]
async fn a_timer_frame_lands_while_the_cover_fades_up_after_a_clear() {
    let (server, _params) =
        start_arena(cover_layers(Some(BIBLE_FADE), Some(COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);

    let start = Instant::now();
    worker.bible(clear_update()).await;
    wait_for(&server, "the cover fade under way", |requests| {
        opacities_put(requests, COVER_OPACITY)
            .last()
            .is_some_and(|last| *last > 0.0)
    })
    .await;
    let timer_text = worker.timer().await;
    let requests = wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let took = start.elapsed();

    assert_shown(&opacities_put(&requests, COVER_OPACITY), 1.0);
    assert_faded(&requests, A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY));
    let timer_put = requests
        .iter()
        .position(|req| body_value(req).as_ref() == Some(&timer_text))
        .expect("the timer frame landed");
    let last_cover = last_index(&requests, is_cover_put).expect("cover PUTs");
    assert!(
        timer_put < last_cover,
        "the timer frame waited for the whole cover fade (timer #{timer_put}, last cover step #{last_cover})"
    );
    let last_bible = A_SIDE_OPACITY
        .into_iter()
        .chain(B_SIDE_OPACITY)
        .filter_map(|param| last_index(&requests, |req| req.url.path() == param_path(param)))
        .max()
        .expect("Bible fade");
    let first_cover = requests.iter().position(is_cover_put).expect("cover PUTs");
    assert!(
        last_bible < first_cover,
        "the cover fades after the clear's Bible fade"
    );
    assert!(
        took >= COVER_FADE,
        "the cover fades over its layer's transition ({COVER_FADE:?}), took {took:?}"
    );
    assert!(connects(&requests).is_empty(), "nothing is triggered");
}

/// A verse is on air first; the cover fades out after its last `/connect`.
#[tokio::test]
async fn a_verse_hides_the_cover_after_its_last_connect() {
    let (server, _params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    let after_clear = wait_for_opacity(&server, COVER_OPACITY, 1.0).await.len();

    worker.bible(verse_update()).await;
    let requests = wait_for(&server, "the cover hidden after the verse", |requests| {
        requests.len() > after_clear
            && opacities_put(&requests[after_clear..], COVER_OPACITY).last() == Some(&0.0)
    })
    .await;

    let verse = &requests[after_clear..];
    assert_eq!(connects(verse).len(), 4, "the verse is on air");
    let last_connect = last_index(verse, is_connect).expect("connects");
    let first_cover = verse.iter().position(is_cover_put).expect("cover hide");
    assert!(
        last_connect < first_cover,
        "the verse is on air before the cover starts to fade"
    );
    let steps = opacities_put(verse, COVER_OPACITY);
    assert!(steps[0] > 0.5, "hidden from full opacity: {steps:?}");
    assert_hidden(&steps);
    for param in A_SIDE_OPACITY {
        assert_eq!(
            opacities_put(verse, param),
            vec![1.0],
            "param {param} is restored before the connect, as before (#841)"
        );
    }
}

/// A verse while the clear's show is still running replaces it with a hide
/// from the opacity the show reached: no jump up to full first.
#[tokio::test]
async fn a_verse_mid_show_hides_the_cover_from_where_the_show_got() {
    let (server, _params) = start_arena(cover_layers(None, Some(COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for(&server, "the show under way", |requests| {
        let steps = opacities_put(requests, COVER_OPACITY);
        steps.len() >= 2 && steps.last().is_some_and(|last| *last > 0.0)
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
    let shown = opacities_put(&requests[..first_connect], COVER_OPACITY);
    let reached = shown.last().copied().unwrap_or_default();
    assert!(
        reached < 1.0 - 1e-9,
        "the verse cut the show short: {shown:?}"
    );
    let hidden = opacities_put(&requests[last_connect..], COVER_OPACITY);
    assert!(
        hidden[0] <= reached + 1e-9,
        "the hide starts where the show got ({reached}), no jump up: {hidden:?}"
    );
    assert!(
        hidden.windows(2).all(|pair| pair[1] <= pair[0]),
        "the cover only goes down: {hidden:?}"
    );
}

/// clear → verse → clear: the second clear shows the cover again from 0.
#[tokio::test]
async fn the_next_clear_shows_the_cover_again_from_zero() {
    let (server, _params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    worker.bible(verse_update()).await;
    let after_verse = wait_for_opacity(&server, COVER_OPACITY, 0.0).await.len();

    worker.bible(clear_update()).await;
    let requests = wait_for(&server, "the cover shown again", |requests| {
        opacities_put(&requests[after_verse..], COVER_OPACITY)
            .last()
            .is_some_and(|last| (last - 1.0).abs() < 1e-9)
    })
    .await;

    assert_shown(&opacities_put(&requests[after_verse..], COVER_OPACITY), 1.0);
    assert!(connects(&requests[after_verse..]).is_empty());
}

/// Shown by the clear, hidden by the first verse, then left alone: the second
/// verse reads the cover at 0 and sends nothing.
#[tokio::test]
async fn the_verse_after_that_sends_no_cover_put() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 0.0).await;

    worker.bible(verse_update()).await;
    wait_for(&server, "the second verse's cover read", |requests| {
        connects(requests).len() == 8 && cover_reads(requests) == 3
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(opacities_put(&requests, COVER_OPACITY), vec![1.0, 0.0]);
    assert_eq!(
        values_put(&requests, 31).len(),
        1,
        "the second verse landed on #bible-b"
    );
}

/// "alebo je clearnuty": a clear while the Bible clips are already invisible
/// still shows the cover; a second clear finds it shown and sends nothing.
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
    layers.push(cover_layer(0, None, 0.0));
    layers.push(timer_layer(0));
    let (server, _params) = start_arena(layers).await;
    let worker = Worker::spawn(&server);

    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    worker.bible(clear_update()).await;
    wait_for(&server, "the second clear's cover read", |requests| {
        cover_reads(requests) == 2
    })
    .await;
    let requests = worker.barrier(&server).await;

    for param in A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY) {
        assert!(
            opacities_put(&requests, param).is_empty(),
            "param {param} is already invisible"
        );
    }
    assert_eq!(opacities_put(&requests, COVER_OPACITY), vec![1.0]);
    assert!(connects(&requests).is_empty());
}

/// #848 review, finding 2: a cover 5xx failed the clear, which opened a #484
/// backoff window that silently dropped the next verse. Now the Bible clips
/// still reach 0, the push succeeds and the host stays healthy. A failed step
/// gets ONE final step that PUTs the target (#848 second review, finding 3);
/// when that fails too, the fade stops: at most 2 failed PUTs, the clear
/// itself is not retried. The barrier frame runs before a third step could be
/// due, so the requests are read again `5 × FADE_STEP` later (#848 third
/// review, finding 2: a retry storm would pass a read at the barrier).
#[tokio::test]
async fn a_failed_cover_put_during_a_clear_fails_nothing() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    answer_param(&server, COVER_OPACITY, 500, 100, 1).await;
    let worker = Worker::spawn(&server);

    worker.bible(clear_update()).await;
    wait_for(&server, "the cover PUT and its final step", |requests| {
        param_requests(requests, "PUT", COVER_OPACITY) == 2
    })
    .await;
    worker.barrier(&server).await;
    let requests = settled(&server).await;

    for param in A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY) {
        assert_eq!(opacities_put(&requests, param), vec![0.0], "param {param}");
    }
    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![1.0, 1.0],
        "exactly the failed step and one final step at the target; then the fade stops"
    );
    assert_healthy(&worker).await;
}

/// The hide runs after the verse is connected and the lanes flipped. Its
/// failure fails nothing: the next verse lands on the other side.
#[tokio::test]
async fn a_failed_cover_fade_does_not_fail_the_verse() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    answer_param(&server, COVER_OPACITY, 500, 100, 1).await;

    worker.bible(verse_update()).await;
    // The failed step's final step may follow (#848 second review, finding 3).
    wait_for(&server, "the cover hide tried", |requests| {
        param_requests(requests, "PUT", COVER_OPACITY) >= 2
    })
    .await;
    worker.barrier(&server).await;
    assert_healthy(&worker).await;
    worker.bible(verse_update()).await;
    let requests = wait_for(&server, "the second verse", |requests| {
        connects(requests).len() == 8
    })
    .await;

    assert_eq!(
        values_put(&requests, 31).len(),
        1,
        "the next verse lands on #bible-b: the lanes flipped despite the failed cover fade"
    );
}

/// #848 review, finding 3: a stale cover id never refetched. A mapped cover's
/// 404 now invalidates the mapping (`stale-id`) once, and the next push
/// refetches it; the same id failing on that fresh mapping refetches nothing,
/// so a broken cover cannot pull the 16 MB composition on every verse. No
/// push fails.
#[tokio::test]
async fn a_mapped_cover_404_refetches_the_mapping_once() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    answer_param(&server, COVER_OPACITY, 404, 100, 1).await;
    answer_param_read(&server, COVER_OPACITY, 404, 100).await;
    let worker = Worker::spawn(&server);

    worker.bible(clear_update()).await;
    wait_for(&server, "the clear's cover request", |requests| {
        cover_reads(requests) + param_requests(requests, "PUT", COVER_OPACITY) >= 1
    })
    .await;
    worker.bible(verse_update()).await;
    wait_for(&server, "the verse's cover request", |requests| {
        connects(requests).len() == 4
            && cover_reads(requests) + param_requests(requests, "PUT", COVER_OPACITY) >= 2
    })
    .await;
    worker.barrier(&server).await;
    worker.bible(verse_update()).await;
    wait_for(&server, "the second verse", |requests| {
        connects(requests).len() == 8
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(
        compositions(&requests),
        2,
        "the cold fetch and ONE stale-id refetch for the cover"
    );
    assert_healthy(&worker).await;
}

/// #848 review (opacity source): the first step reads the cover's opacity in
/// Arena. The operator dimmed the shown title to 0.4 there; the verse hides
/// it from 0.4, never from the 1.0 Presenter remembers.
#[tokio::test]
async fn the_hide_starts_from_the_opacity_arena_reports() {
    let (server, params) = start_arena(cover_layers(None, Some(SHORT_COVER_FADE), 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    let after_clear = wait_for_opacity(&server, COVER_OPACITY, 1.0).await.len();
    params.set(COVER_OPACITY, 0.4);

    worker.bible(verse_update()).await;
    let requests = wait_for(&server, "the cover hidden", |requests| {
        requests.len() > after_clear
            && opacities_put(&requests[after_clear..], COVER_OPACITY).last() == Some(&0.0)
    })
    .await;

    let steps = opacities_put(&requests[after_clear..], COVER_OPACITY);
    assert!(
        steps.iter().all(|level| *level <= 0.4 + 1e-9),
        "hidden from the 0.4 Arena shows: {steps:?}"
    );
    assert_hidden(&steps);
}

/// The operator hid the title in Arena already: the verse reads it at 0 and
/// sends no PUT.
#[tokio::test]
async fn a_cover_already_at_its_target_gets_no_put() {
    let (server, params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    params.set(COVER_OPACITY, 0.0);

    worker.bible(verse_update()).await;
    wait_for(&server, "the verse's cover read", |requests| {
        connects(requests).len() == 4 && cover_reads(requests) == 2
    })
    .await;
    let requests = worker.barrier(&server).await;

    assert_eq!(opacities_put(&requests, COVER_OPACITY), vec![1.0]);
}

/// A shown title dimmed in Arena keeps that opacity as its base: the verse
/// hides it from there and the next clear brings it back to it, not to full.
#[tokio::test]
async fn a_cover_dimmed_in_arena_comes_back_at_that_opacity() {
    let (server, params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    params.set(COVER_OPACITY, 0.6);

    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 0.0).await;
    worker.bible(clear_update()).await;
    let requests = wait_for(&server, "the cover shown again", |requests| {
        opacities_put(requests, COVER_OPACITY).len() == 3
    })
    .await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![1.0, 0.0, 0.6],
        "shown, hidden from the dimmed value, then shown at it"
    );
}

/// A cover a previous process (or the operator) left at a lower opacity is
/// already shown: the clear leaves it, the verse hides it, and the next clear
/// brings it back to that opacity.
#[tokio::test]
async fn a_cover_mapped_below_full_opacity_is_shown_at_that_value() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.6)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    worker.bible(verse_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 0.0).await;
    worker.bible(clear_update()).await;
    let requests = wait_for_opacity(&server, COVER_OPACITY, 0.6).await;

    assert_eq!(
        opacities_put(&requests, COVER_OPACITY),
        vec![0.0, 0.6],
        "nothing for the first clear, then hidden and shown at 0.6"
    );
}

/// Switch to the second deck: the first deck's title stays on its layer, and
/// `/composition` lists only the second deck's. Returns the new deck's params.
async fn switch_deck(server: &MockServer, worker: &Worker) -> ArenaParams {
    server.reset().await;
    let params = mount_arena(server, cover_layers_on(DECK_TWO, None, None, 0.0)).await;
    worker.refresh_mapping().await;
    params
}

/// #848 review (deck switch): the cover the first deck showed is remembered,
/// like a verse's connected clips (#841), so the next verse hides it although
/// the mapping now lists only the second deck's cover.
#[tokio::test]
async fn a_remembered_cover_is_hidden_after_a_deck_switch() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    let params = switch_deck(&server, &worker).await;
    params.set(COVER_OPACITY, 1.0); // the first deck's title still exists

    worker.bible(verse_update()).await;
    let requests = wait_for_opacity(&server, COVER_OPACITY, 0.0).await;

    assert_eq!(opacities_put(&requests, COVER_OPACITY), vec![0.0]);
    assert!(
        opacities_put(&requests, COVER_OPACITY + DECK_TWO).is_empty(),
        "the second deck's title is already hidden"
    );
    let cover_connect = format!("/api/v1/composition/clips/by-id/{COVER}/connect");
    assert!(connects(&requests)
        .iter()
        .all(|req| req.url.path() != cover_connect));
}

/// A remembered cover Resolume no longer knows (404) is forgotten: no
/// refetch, and later fades leave it alone.
#[tokio::test]
async fn a_remembered_cover_gone_after_a_deck_switch_is_forgotten() {
    let (server, _params) = start_arena(cover_layers(None, None, 0.0)).await;
    let worker = Worker::spawn(&server);
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY, 1.0).await;
    switch_deck(&server, &worker).await;

    worker.bible(verse_update()).await;
    wait_for(&server, "the gone cover's read", |requests| {
        connects(requests).len() == 4 && cover_reads(requests) == 1
    })
    .await;
    worker.barrier(&server).await;
    worker.bible(clear_update()).await;
    wait_for_opacity(&server, COVER_OPACITY + DECK_TWO, 1.0).await;
    let requests = worker.barrier(&server).await;

    assert_eq!(cover_reads(&requests), 1, "read once, then forgotten");
    assert_eq!(param_requests(&requests, "PUT", COVER_OPACITY), 0);
    assert_eq!(
        compositions(&requests),
        1,
        "only the manual refresh: a gone remembered cover refetches nothing"
    );
    assert_healthy(&worker).await;
}
