//! `pulpit` (#852 28.11): the pure policy, the selector on explicit instants,
//! and the real task on the paused clock.

use std::time::Duration;

use super::*;
use crate::tracing_capture::Capture;

const MS: fn(u64) -> Duration = Duration::from_millis;

fn view(voiced_ago_ms: Option<u64>, recent_dbfs: f32) -> MicView {
    MicView {
        voiced_ago: voiced_ago_ms.map(MS),
        recent_dbfs,
        language: None,
    }
}

fn labelled(recent_dbfs: f32, language: Option<SpokenLanguage>) -> MicView {
    MicView {
        voiced_ago: Some(MS(0)),
        recent_dbfs,
        language,
    }
}

/// A 20 ms chunk at an amplitude (0 = digital silence, 8000 ≈ −12 dBFS).
fn chunk(level: i16) -> MicChunk {
    (0..320)
        .map(|i| if i % 20 < 10 { level } else { -level })
        .collect::<Vec<i16>>()
        .into()
}

/// The first `n` fixed Dante mics (language `off` = unknown).
fn mics(n: usize) -> Vec<MicConfig> {
    presenter_core::listen::mics::fixed_dante_mics()
        .into_iter()
        .take(n)
        .collect()
}

#[test]
fn it_switches_only_after_the_forwarded_mic_was_quiet_past_the_hangover() {
    let other = view(Some(0), -20.0);
    let within = HANGOVER.as_millis() as u64;
    assert_eq!(
        choose(Some(0), &[view(Some(within), -60.0), other.clone()]),
        Decision::Stay,
        "exactly the hangover: still the same sentence"
    );
    assert_eq!(
        choose(Some(0), &[view(Some(within + 1), -60.0), other.clone()]),
        Decision::Switch {
            to: 1,
            reason: SwitchReason::CurrentQuiet
        }
    );
    assert_eq!(
        choose(Some(0), &[view(None, SILENCE_DBFS), other]),
        Decision::Switch {
            to: 1,
            reason: SwitchReason::CurrentQuiet
        },
        "a current mic that never spoke is quiet"
    );
}

#[test]
fn it_stays_on_a_voiced_mic_even_when_another_is_louder() {
    assert_eq!(
        choose(Some(0), &[view(Some(20), -40.0), view(Some(0), -6.0)]),
        Decision::Stay
    );
}

#[test]
fn without_another_voiced_mic_it_keeps_the_quiet_one_and_never_picks_a_stale_one() {
    let stale = CANDIDATE_WINDOW.as_millis() as u64 + 1;
    assert_eq!(
        choose(
            Some(0),
            &[view(Some(60_000), -90.0), view(Some(stale), -10.0)]
        ),
        Decision::Stay
    );
    assert_eq!(choose(None, &[view(None, SILENCE_DBFS)]), Decision::Stay);
    assert_eq!(choose(None, &[]), Decision::Stay);
}

#[test]
fn the_loudest_voiced_mic_wins_then_the_channel_order() {
    let mics = [
        view(Some(0), -30.0),
        view(Some(40), -20.0),
        view(Some(0), -40.0),
        view(Some(500), -3.0),
    ];
    assert_eq!(
        choose(None, &mics),
        Decision::Switch {
            to: 1,
            reason: SwitchReason::First
        },
        "the 4th is louder but not voiced now"
    );
    assert_eq!(
        choose(None, &[view(Some(0), -25.0), view(Some(0), -25.0)]),
        Decision::Switch {
            to: 0,
            reason: SwitchReason::First
        }
    );
}

#[test]
fn a_foreign_mic_beats_an_unknown_one_which_beats_a_slovak_one() {
    use SpokenLanguage::{Foreign, Slovak};
    let pick = |mics: &[MicView]| match choose(None, mics) {
        Decision::Switch { to, .. } => Some(to),
        Decision::Stay => None,
    };
    // the interpreter (Slovak) is louder: the guest still wins
    assert_eq!(
        pick(&[labelled(-6.0, Some(Slovak)), labelled(-30.0, Some(Foreign))]),
        Some(1)
    );
    assert_eq!(
        pick(&[labelled(-6.0, Some(Slovak)), labelled(-30.0, None)]),
        Some(1)
    );
    assert_eq!(
        pick(&[labelled(-6.0, None), labelled(-30.0, Some(Foreign))]),
        Some(1)
    );
    // the same rank: the level decides
    assert_eq!(
        pick(&[labelled(-30.0, Some(Slovak)), labelled(-6.0, Some(Slovak))]),
        Some(1)
    );
    // a label never breaks the hangover's stickiness
    assert_eq!(
        choose(
            Some(0),
            &[labelled(-30.0, Some(Slovak)), labelled(-6.0, Some(Foreign))]
        ),
        Decision::Stay
    );
}

#[test]
fn the_selector_forwards_only_the_chosen_mics_chunks_and_cuts_frames_of_five() {
    let mut selector = Selector::new(&mics(2));
    let t0 = Instant::now();
    let quiet = selector.push(0, chunk(0), t0);
    assert_eq!(quiet, Forwarded::default(), "nothing before a voice");
    assert_eq!(selector.current(), None);

    let mut chunks = 0;
    let mut frames = Vec::new();
    for n in 0..10u32 {
        let at = t0 + MS(20) * n;
        let first = selector.push(0, chunk(8_000), at);
        if n == 0 {
            assert_eq!(
                first.switch,
                Some((None, "dante-mic-1".to_string(), SwitchReason::First))
            );
            assert_eq!(
                first.chunks.len(),
                2,
                "the onset: the quiet chunk before it, then the voiced one"
            );
        }
        chunks += first.chunks.len();
        frames.extend(first.frames);
        // the second mic also speaks, louder: never forwarded while mic 1 speaks
        let second = selector.push(1, chunk(16_000), at);
        assert_eq!(second, Forwarded::default());
    }
    assert_eq!(chunks, 11);
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame.len() == 1_600));
    assert_eq!(&frames[0][..320], &chunk(0)[..]);
    assert_eq!(&frames[0][320..640], &chunk(8_000)[..]);
}

#[test]
fn after_the_hangover_the_selector_moves_to_the_voiced_mic_and_sends_its_held_onset() {
    let mut selector = Selector::new(&mics(2));
    let t0 = Instant::now();
    selector.push(0, chunk(8_000), t0);
    assert_eq!(selector.current(), Some("dante-mic-1"));
    // mic 2 is quiet 200 ms, then speaks while mic 1 is silent: mic 1's
    // silence is forwarded until its hangover ends
    let mut step = 1u32;
    loop {
        let at = t0 + MS(20) * step;
        let first = selector.push(0, chunk(0), at);
        let level = if step <= 10 { 0 } else { 8_000 };
        let second = selector.push(1, chunk(level), at);
        if at - t0 <= HANGOVER {
            assert_eq!(first.chunks.len(), 1, "{step}");
            assert!(second.chunks.is_empty(), "{step}");
        } else {
            let (from, to, reason) = first
                .switch
                .expect("the switch on the first chunk past the hangover");
            assert_eq!(
                (from.as_deref(), to.as_str(), reason),
                (
                    Some("dante-mic-1"),
                    "dante-mic-2",
                    SwitchReason::CurrentQuiet
                )
            );
            // mic 1's last silent chunk, then mic 2's held 2 s (100 chunks:
            // steps 1..=100), from 10 quiet chunks before its first voiced one
            assert_eq!(first.chunks.len(), 1 + 100);
            assert_eq!(first.chunks[0][0], 0);
            assert!(first.chunks[1..11].iter().all(|c| c[0] == 0));
            assert!(first.chunks[11..].iter().all(|c| c[0] == 8_000));
            assert_eq!(second.chunks.len(), 1, "then mic 2 live");
            assert_eq!(selector.push(0, chunk(0), at), Forwarded::default());
            break;
        }
        step += 1;
    }
    assert_eq!(step, 101, "2 s of 20 ms chunks, then the switch");
}

#[test]
fn the_held_onset_leaves_the_quiet_before_its_pre_roll_behind() {
    let mut selector = Selector::new(&mics(2));
    let t0 = Instant::now();
    selector.push(0, chunk(8_000), t0);
    for step in 1..=100u32 {
        let at = t0 + MS(20) * step;
        selector.push(0, chunk(0), at);
        // 80 quiet chunks, then voice
        selector.push(1, chunk(if step <= 80 { 0 } else { 8_000 }), at);
    }
    let out = selector.push(0, chunk(0), t0 + MS(20) * 101);
    assert!(out.switch.is_some());
    assert_eq!(
        out.chunks.len(),
        1 + SWITCH_PRE_ROLL_CHUNKS + 20,
        "mic 1's chunk, 200 ms pre-roll, the 20 voiced chunks"
    );
}

#[test]
fn a_labelled_mic_is_picked_by_the_selector_too() {
    let mut selector = Selector::new(&mics(2));
    selector.set_language(0, Some(SpokenLanguage::Slovak));
    selector.set_language(1, Some(SpokenLanguage::Foreign));
    let t0 = Instant::now();
    // both speak in the same 20 ms: the Slovak (louder) one arrives first
    selector.push(0, chunk(16_000), t0);
    assert_eq!(
        selector.current(),
        Some("dante-mic-1"),
        "the only voiced mic yet"
    );
    selector.push(1, chunk(4_000), t0);
    assert_eq!(selector.current(), Some("dante-mic-1"), "sticky");
}

#[test]
fn a_mic_configured_slovak_or_english_carries_that_language() {
    use presenter_core::listen::mics::MicLanguage;
    let mut configured = mics(3);
    configured[0].language = MicLanguage::Sk;
    configured[1].language = MicLanguage::En;
    let selector = Selector::new(&configured);
    let languages: Vec<Option<SpokenLanguage>> =
        selector.inputs.iter().map(|input| input.language).collect();
    assert_eq!(
        languages,
        [
            Some(SpokenLanguage::Slovak),
            Some(SpokenLanguage::Foreign),
            None
        ]
    );
}

#[test]
fn switch_lines_are_limited_per_minute_and_the_rest_summarised() {
    let t0 = Instant::now();
    let mut log = SwitchLog::new(t0);
    let admitted = (0..30).filter(|_| log.admit(t0).0).count();
    assert_eq!(admitted, SWITCH_LINES_PER_MINUTE as usize);
    assert_eq!(
        log.roll(t0 + Duration::from_secs(59)),
        None,
        "the minute runs"
    );
    // the summary comes on the minute's tick, with no further switch
    assert_eq!(log.roll(t0 + Duration::from_secs(60)), Some(10));
    assert_eq!(log.admit(t0 + Duration::from_secs(61)), (true, None));
    assert_eq!(
        log.roll(t0 + Duration::from_secs(121)),
        None,
        "nothing suppressed"
    );
}

#[test]
fn pulpit_is_a_source_only_while_a_dante_mic_is_published() {
    let listen = ListenService::new();
    assert!(!listen.has_source(PULPIT_MIC_ID));
    listen.set_dante_mics(true);
    assert!(listen.has_source(PULPIT_MIC_ID));
    assert!(listen.has_source("dante-mic-4"));
    assert!(!listen.has_source("guest"));
    assert!(
        listen.mics().iter().all(|mic| mic.id != PULPIT_MIC_ID),
        "pulpit is no mic of the list"
    );
}

async fn send_for(listen: &ListenService, mic: &str, level: i16, chunks: usize) {
    let sender = listen.chunk_sender(mic);
    for _ in 0..chunks {
        let _ = sender.send(chunk(level));
        tokio::time::sleep(MS(20)).await;
    }
}

fn drain(rx: &mut broadcast::Receiver<MicChunk>) -> Vec<MicChunk> {
    let mut out = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        out.push(chunk);
    }
    out
}

#[tokio::test(start_paused = true)]
async fn a_mic_that_stops_sending_never_stalls_pulpit() {
    let capture = Capture::install();
    let listen = ListenService::new();
    listen.set_dante_mics(true);
    let mut pulpit = listen.subscribe_chunks(PULPIT_MIC_ID);
    let mut frames = listen.subscribe_frames(PULPIT_MIC_ID);
    tokio::spawn(supervise(listen.clone()));
    tokio::time::sleep(MS(1)).await;

    // mic 1 speaks 200 ms, then its chunks stop arriving at all (unpatched, lagging)
    send_for(&listen, "dante-mic-1", 8_000, 10).await;
    assert_eq!(drain(&mut pulpit).len(), 10);
    // mic 2 speaks on; mic 1 sends nothing: nothing waits for it, mic 2 is
    // forwarded once mic 1's hangover is over
    send_for(&listen, "dante-mic-2", 12_000, 150).await;
    let forwarded = drain(&mut pulpit);
    // mic 1's last voiced chunk at 181 ms; mic 2's k-th at 201 + 20k ms: the
    // first past the 2 s hangover is k = 100, sent with its held 100 chunks
    // (k = 1..=100), then k = 101..=149 live
    assert_eq!(forwarded.len(), 149);
    assert!(forwarded.iter().all(|c| c[0] == 12_000));
    let mut frame_count = 0;
    while frames.try_recv().is_ok() {
        frame_count += 1;
    }
    assert_eq!(frame_count, (10 + 149) / 5);
    // a saved-list change leaves the Dante mics and the selection alone
    listen.apply(Vec::new());
    send_for(&listen, "dante-mic-2", 12_000, 5).await;
    assert_eq!(drain(&mut pulpit).len(), 5);
    assert!(capture
        .events_with_message("pulpit restarts its selection")
        .is_empty());
    let switches = capture.events_with_message("pulpit switched mic");
    let last = switches.last().expect("a switch line");
    assert_eq!(last.field("from"), Some("\"dante-mic-1\""));
    assert_eq!(last.field("to"), Some("\"dante-mic-2\""));
    assert_eq!(
        last.field("reason"),
        Some("\"the forwarded mic went quiet\"")
    );
    assert_eq!(switches.len(), 2, "first voice, then the move");
}

#[tokio::test(start_paused = true)]
async fn a_lagging_input_is_skipped_and_warned_about_once_and_never_stalls_pulpit() {
    let capture = Capture::install();
    let listen = ListenService::new();
    listen.set_dante_mics(true);
    let mut pulpit = listen.subscribe_chunks(PULPIT_MIC_ID);
    tokio::spawn(supervise(listen.clone()));
    tokio::time::sleep(MS(1)).await;
    // mic 3 floods its channel past its capacity before pulpit can read
    let flood = listen.chunk_sender("dante-mic-3");
    for _ in 0..crate::listen::CHUNK_CHANNEL_CAPACITY + 88 {
        let _ = flood.send(chunk(0));
    }
    send_for(&listen, "dante-mic-2", 12_000, 10).await;
    let forwarded = drain(&mut pulpit);
    assert_eq!(forwarded.len(), 10, "mic 2 is forwarded all the same");
    let warns = capture.events_with_message("pulpit fell behind a Dante mic");
    assert_eq!(warns.len(), 1);
    assert_eq!(warns[0].field("input"), Some("\"dante-mic-3\""));
    assert_eq!(warns[0].field("missed"), Some("88"));
}
