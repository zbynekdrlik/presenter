//! The session machine on hand-made `Instant`s: no runtime, no socket.
//! A frame's first two samples carry its index (`id`), so a test can read
//! back exactly which frames went out on which connection.

use std::sync::Arc;

use super::super::wire::{RawToken, SonioxError};
use super::*;

/// Frame `i` of a test: 1600 samples, the index in the first two.
fn frame(i: u32) -> MicFrame {
    let mut pcm = vec![0_i16; 1600];
    pcm[0] = (i % 30_000) as i16;
    pcm[1] = (i / 30_000) as i16;
    Arc::from(pcm)
}

fn id(pcm: &[i16]) -> u32 {
    pcm[0] as u32 + pcm[1] as u32 * 30_000
}

fn at(t0: Instant, ms: u64) -> Instant {
    t0 + Duration::from_millis(ms)
}

fn keyed() -> SonioxCore {
    SonioxCore::new(MachineConfig { key_present: true })
}

/// Frame `i` at `i × 100 ms` (`offset_ms` shifts the clock).
fn feed(core: &mut SonioxCore, t0: Instant, i: u32, voiced: bool) -> Vec<Action> {
    feed_at(core, t0, i, voiced, u64::from(i) * 100)
}

fn feed_at(core: &mut SonioxCore, t0: Instant, i: u32, voiced: bool, ms: u64) -> Vec<Action> {
    core.on_input(Input::Frame {
        pcm: frame(i),
        voiced,
        at: at(t0, ms),
    })
}

fn connected(core: &mut SonioxCore, t0: Instant, conn: u32, ms: u64) -> Vec<Action> {
    core.on_report(Report::Connected {
        conn,
        at: at(t0, ms),
    })
}

fn token(text: &str, start: u64, end: u64, is_final: bool) -> RawToken {
    RawToken {
        text: text.to_string(),
        start_ms: Some(start),
        end_ms: Some(end),
        confidence: Some(0.9),
        is_final: Some(is_final),
        language: Some("en".to_string()),
        translation_status: None,
    }
}

fn response(tokens: Vec<RawToken>) -> SonioxResponse {
    SonioxResponse {
        tokens,
        final_audio_proc_ms: None,
        total_audio_proc_ms: None,
        finished: false,
        error: None,
    }
}

fn error_response(code: u16) -> SonioxResponse {
    SonioxResponse {
        error: Some(SonioxError {
            code: Some(code),
            kind: None,
            message: "test".to_string(),
        }),
        ..response(Vec::new())
    }
}

fn finished_response() -> SonioxResponse {
    SonioxResponse {
        finished: true,
        ..response(Vec::new())
    }
}

fn fin() -> RawToken {
    RawToken {
        start_ms: None,
        end_ms: None,
        ..token("<fin>", 0, 0, true)
    }
}

fn respond(
    core: &mut SonioxCore,
    t0: Instant,
    conn: u32,
    resp: SonioxResponse,
    ms: u64,
) -> Vec<Action> {
    core.on_report(Report::Response {
        conn,
        resp,
        at: at(t0, ms),
        received_at: Utc::now(),
    })
}

/// The ids of the frames sent, per connection, in order.
fn sent(actions: &[Action]) -> Vec<(u32, u32)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::SendAudio { conn, pcm } => Some((*conn, id(pcm))),
            _ => None,
        })
        .collect()
}

fn connects(actions: &[Action]) -> Vec<(u32, u64, ConnectKind)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Connect {
                conn,
                offset_ms,
                kind,
            } => Some((*conn, *offset_ms, *kind)),
            _ => None,
        })
        .collect()
}

fn controls(actions: &[Action]) -> Vec<Control> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::SendControl { control, .. } => Some(*control),
            _ => None,
        })
        .collect()
}

fn states(actions: &[Action]) -> Vec<TranscriptionState> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::State { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn finished(actions: &[Action]) -> Option<Option<OffReason>> {
    actions.iter().find_map(|a| match a {
        Action::Finished(reason) => Some(*reason),
        _ => None,
    })
}

/// A session streaming on connection 1: frame 0 voiced at 0 ms, connected
/// at 50 ms.
fn streaming(t0: Instant) -> SonioxCore {
    let mut core = keyed();
    let _ = feed(&mut core, t0, 0, true);
    let _ = connected(&mut core, t0, 1, 50);
    core
}

#[test]
fn nothing_connects_before_a_voiced_frame() {
    let t0 = Instant::now();
    let mut core = keyed();
    let mut all = Vec::new();
    for i in 0..300 {
        all.extend(feed(&mut core, t0, i, false));
        all.extend(core.on_tick(at(t0, u64::from(i) * 100 + 50)));
    }
    assert!(all.is_empty(), "{all:?}");
    assert_eq!(core.state(), TranscriptionState::Armed);
    assert_eq!(core.next_deadline(), None);
}

#[test]
fn the_first_voiced_frame_connects_and_sends_the_pre_roll_first() {
    let t0 = Instant::now();
    let mut core = keyed();
    for i in 0..12 {
        assert!(feed(&mut core, t0, i, false).is_empty());
    }
    let actions = feed(&mut core, t0, 12, true);
    assert_eq!(connects(&actions), vec![(1, 0, ConnectKind::Lazy)]);
    assert_eq!(states(&actions), vec![TranscriptionState::Connecting]);
    let actions = connected(&mut core, t0, 1, 1_300);
    let expected: Vec<(u32, u32)> = (3..=12).map(|i| (1, i)).collect();
    assert_eq!(sent(&actions), expected);
    assert_eq!(states(&actions), vec![TranscriptionState::Streaming]);
    // The next frame goes out at once on the same connection.
    assert_eq!(sent(&feed(&mut core, t0, 13, true)), vec![(1, 13)]);
}

#[test]
fn frames_while_connecting_are_queued_up_to_five_seconds_and_counted_when_dropped() {
    let t0 = Instant::now();
    let mut core = keyed();
    let mut all = feed(&mut core, t0, 0, true);
    for i in 1..=60 {
        all.extend(feed(&mut core, t0, i, true));
    }
    assert!(sent(&all).is_empty());
    assert_eq!(connects(&all).len(), 1);
    let actions = connected(&mut core, t0, 1, 6_050);
    let expected: Vec<(u32, u32)> = (11..=60).map(|i| (1, i)).collect();
    assert_eq!(sent(&actions), expected);
    assert_eq!(core.stats().dropped_frames, 11);
}

#[test]
fn ten_quiet_seconds_finalize_once_then_keep_alive_every_five_seconds() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let mut before = Vec::new();
    for i in 1..100 {
        before.extend(feed(&mut core, t0, i, false));
    }
    // Quiet frames within 10 s of the voice still go out.
    assert_eq!(sent(&before).len(), 99);
    assert!(controls(&before).is_empty());
    let gate = feed(&mut core, t0, 100, false);
    assert_eq!(controls(&gate), vec![Control::Finalize]);
    assert_eq!(states(&gate), vec![TranscriptionState::Gated]);
    assert!(sent(&gate).is_empty());
    let mut after = Vec::new();
    for i in 101..=210 {
        after.extend(feed(&mut core, t0, i, false));
    }
    assert!(sent(&after).is_empty());
    assert_eq!(
        controls(&after),
        vec![Control::Keepalive, Control::Keepalive]
    );
    assert_eq!(core.state(), TranscriptionState::Gated);
}

#[test]
fn no_frames_for_ten_seconds_also_gates() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    assert_eq!(core.next_deadline(), Some(at(t0, 10_000)));
    assert!(core.on_tick(at(t0, 9_999)).is_empty());
    let actions = core.on_tick(at(t0, 10_000));
    assert_eq!(controls(&actions), vec![Control::Finalize]);
    assert_eq!(states(&actions), vec![TranscriptionState::Gated]);
    assert!(controls(&core.on_tick(at(t0, 14_999))).is_empty());
    let actions = core.on_tick(at(t0, 15_000));
    assert_eq!(controls(&actions), vec![Control::Keepalive]);
}

#[test]
fn voice_after_the_gate_resumes_on_the_same_connection_with_the_pre_roll() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    for i in 100..=150 {
        assert!(sent(&feed(&mut core, t0, i, false)).is_empty());
    }
    let actions = feed(&mut core, t0, 151, true);
    assert!(connects(&actions).is_empty());
    let expected: Vec<(u32, u32)> = (142..=151).map(|i| (1, i)).collect();
    assert_eq!(sent(&actions), expected);
    assert_eq!(states(&actions), vec![TranscriptionState::Streaming]);
    assert_eq!(sent(&feed(&mut core, t0, 152, false)), vec![(1, 152)]);
}

#[test]
fn five_gated_minutes_park_and_the_next_voice_reconnects_without_backoff() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    // The finalize is answered: everything sent is confirmed.
    let _ = respond(&mut core, t0, 1, response(vec![fin()]), 10_300);
    let actions = core.on_tick(at(t0, 309_999));
    assert!(!actions.iter().any(|a| matches!(a, Action::EndAudio { .. })));
    let actions = core.on_tick(at(t0, 310_000));
    assert!(actions.contains(&Action::EndAudio { conn: 1 }));
    assert!(actions.contains(&Action::ConnectionEnded { conn: 1 }));
    assert_eq!(states(&actions), vec![TranscriptionState::Parked]);
    let actions = feed_at(&mut core, t0, 5_000, true, 400_000);
    assert_eq!(connects(&actions), vec![(2, 100, ConnectKind::Lazy)]);
    let actions = connected(&mut core, t0, 2, 400_100);
    // Nothing to resend: only the new voice.
    assert_eq!(sent(&actions), vec![(2, 5_000)]);
    assert_eq!(core.stats().reconnects, 0);
}

#[test]
fn a_17_second_outage_retries_on_schedule_and_resends_from_the_last_final() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    for i in 1..30 {
        let _ = feed(&mut core, t0, i, true);
    }
    let _ = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" amen", 2_000, 2_350, true)]),
        2_950,
    );
    let drop = core.on_report(Report::Closed {
        conn: 1,
        code: None,
        reason: "reset".to_string(),
        at: at(t0, 3_000),
    });
    assert!(drop.contains(&Action::Drop { conn: 1 }));
    assert!(drop.contains(&Action::ConnectionEnded { conn: 1 }));
    assert_eq!(states(&drop), vec![TranscriptionState::Reconnecting]);
    let mut attempts = Vec::new();
    for i in 30..=290 {
        let actions = feed(&mut core, t0, i, true);
        for (conn, _, kind) in connects(&actions) {
            assert_eq!(kind, ConnectKind::Retry);
            attempts.push(u64::from(i) * 100);
            if i < 290 {
                let _ = core.on_report(Report::ConnectFailed {
                    conn,
                    status: None,
                    error: "dns".to_string(),
                    at: at(t0, u64::from(i) * 100),
                });
            }
        }
    }
    assert_eq!(attempts, vec![4_000, 9_000, 29_000]);
    assert_eq!(core.state(), TranscriptionState::Reconnecting);
    let actions = connected(&mut core, t0, 4, 29_000);
    let out = sent(&actions);
    // The resend starts at 2 350 ms: frame 23 cut in half, then 24 … 29.
    let first = actions
        .iter()
        .find_map(|a| match a {
            Action::SendAudio { pcm, .. } => Some(pcm.len()),
            _ => None,
        })
        .unwrap_or(0);
    assert_eq!(first, 800);
    let resent: Vec<u32> = out.iter().skip(1).take(6).map(|(_, i)| *i).collect();
    assert_eq!(resent, (24..=29).collect::<Vec<u32>>());
    let queued: Vec<u32> = out.iter().skip(7).map(|(_, i)| *i).collect();
    assert_eq!(queued, (241..=290).collect::<Vec<u32>>());
    assert!(out.iter().all(|(conn, _)| *conn == 4));
    assert_eq!(states(&actions), vec![TranscriptionState::Streaming]);
}

#[test]
fn a_17_second_outage_connects_with_the_offset_of_the_last_final() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    for i in 1..30 {
        let _ = feed(&mut core, t0, i, true);
    }
    let _ = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" amen", 2_000, 2_350, true)]),
        2_950,
    );
    let _ = core.on_report(Report::Closed {
        conn: 1,
        code: Some(1006),
        reason: String::new(),
        at: at(t0, 3_000),
    });
    assert_eq!(core.state(), TranscriptionState::Reconnecting);
    assert_eq!(core.next_deadline(), Some(at(t0, 4_000)));
    let actions = core.on_tick(at(t0, 4_000));
    assert_eq!(connects(&actions), vec![(2, 2_350, ConnectKind::Retry)]);
}

#[test]
fn a_stall_while_audio_flows_reconnects() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let mut dropped_at = None;
    for i in 1..200 {
        let actions = feed(&mut core, t0, i, true);
        if actions.contains(&Action::Drop { conn: 1 }) {
            assert_eq!(states(&actions), vec![TranscriptionState::Reconnecting]);
            dropped_at = Some(i);
            break;
        }
    }
    // Connected at 50 ms, so 15 s without a response ends at 15 050 ms.
    assert_eq!(dropped_at, Some(151));
    assert_eq!(core.stats().reconnects, 0);
    // One second later: the last 5 s of sent audio go again (nothing was
    // confirmed), frame 151 waits in the queue.
    let actions = core.on_tick(at(t0, 16_100));
    assert_eq!(connects(&actions), vec![(2, 10_100, ConnectKind::Retry)]);
    let actions = connected(&mut core, t0, 2, 16_200);
    let ids: Vec<u32> = sent(&actions).iter().map(|(_, i)| *i).collect();
    assert_eq!(ids, (101..=151).collect::<Vec<u32>>());
}

#[test]
fn no_stall_while_audio_gets_answers() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    for i in 1..400 {
        let actions = feed(&mut core, t0, i, true);
        assert!(!actions.contains(&Action::Drop { conn: 1 }));
        if i % 10 == 0 {
            let _ = respond(&mut core, t0, 1, response(Vec::new()), u64::from(i) * 100);
        }
    }
    assert_eq!(core.state(), TranscriptionState::Streaming);
}

#[test]
fn no_stall_while_gated() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let mut all = Vec::new();
    for i in 1..2_400 {
        all.extend(feed(&mut core, t0, i, false));
    }
    assert!(!all.iter().any(|a| matches!(a, Action::Drop { .. })));
    assert!(!states(&all).contains(&TranscriptionState::Reconnecting));
    assert_eq!(core.state(), TranscriptionState::Gated);
}

#[test]
fn auth_payment_and_bad_request_stop_without_retry() {
    let cases = [
        (401, OffReason::Auth),
        (403, OffReason::Auth),
        (402, OffReason::Payment),
        (400, OffReason::Refused),
    ];
    for (code, reason) in cases {
        let t0 = Instant::now();
        let mut core = streaming(t0);
        let actions = respond(&mut core, t0, 1, error_response(code), 500);
        assert_eq!(finished(&actions), Some(Some(reason)), "{code}");
        assert!(actions.contains(&Action::Drop { conn: 1 }));
        assert_eq!(states(&actions), vec![TranscriptionState::Stopped]);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::State { error: Some(e), .. } if e.contains(&code.to_string())
        )));
        // Never again: no reconnect, whatever comes.
        assert!(feed(&mut core, t0, 10, true).is_empty());
        assert!(core.on_tick(at(t0, 3_600_000)).is_empty());
        assert_eq!(core.next_deadline(), None);
    }
}

#[test]
fn a_refused_upgrade_stops_without_retry() {
    let t0 = Instant::now();
    let mut core = keyed();
    let _ = feed(&mut core, t0, 0, true);
    let actions = core.on_report(Report::ConnectFailed {
        conn: 1,
        status: Some(401),
        error: "HTTP 401".to_string(),
        at: at(t0, 100),
    });
    assert_eq!(finished(&actions), Some(Some(OffReason::Auth)));
    assert!(connects(&feed(&mut core, t0, 2, true)).is_empty());
}

#[test]
fn rate_limit_and_server_errors_back_off() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let actions = respond(&mut core, t0, 1, error_response(429), 500);
    assert!(connects(&actions).is_empty());
    assert_eq!(states(&actions), vec![TranscriptionState::Reconnecting]);
    assert_eq!(core.next_deadline(), Some(at(t0, 1_500)));
    let _ = feed_at(&mut core, t0, 5, true, 1_400);
    assert!(connects(&core.on_tick(at(t0, 1_499))).is_empty());
    let actions = core.on_tick(at(t0, 1_500));
    assert_eq!(connects(&actions), vec![(2, 0, ConnectKind::Retry)]);
    let _ = connected(&mut core, t0, 2, 1_600);
    let actions = respond(&mut core, t0, 2, error_response(503), 1_700);
    assert!(connects(&actions).is_empty());
    // The second retry waits 5 s (no good response in between).
    assert_eq!(core.next_deadline(), Some(at(t0, 6_700)));
}

#[test]
fn too_long_reconnects_at_once() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = feed(&mut core, t0, 1, true);
    let actions = respond(&mut core, t0, 1, error_response(413), 150);
    assert!(actions.contains(&Action::Drop { conn: 1 }));
    assert_eq!(connects(&actions), vec![(2, 0, ConnectKind::Retry)]);
}

#[test]
fn more_than_twenty_connections_in_ten_minutes_stop() {
    let t0 = Instant::now();
    let mut core = keyed();
    let mut all = Vec::new();
    for k in 0..25_u32 {
        let ms = u64::from(k) * 1_000;
        all.extend(feed_at(&mut core, t0, k, true, ms));
        if finished(&all).is_some() {
            break;
        }
        all.extend(connected(&mut core, t0, k + 1, ms + 10));
        // Soniox ends the stream at once: reconnect, no back-off.
        all.extend(respond(&mut core, t0, k + 1, finished_response(), ms + 20));
    }
    assert_eq!(connects(&all).len(), 20);
    assert_eq!(finished(&all), Some(Some(OffReason::TooManyConnections)));
    assert_eq!(core.state(), TranscriptionState::Stopped);
}

#[test]
fn twenty_connections_spread_over_more_than_ten_minutes_go_on() {
    let t0 = Instant::now();
    let mut core = keyed();
    let mut all = Vec::new();
    for k in 0..30_u32 {
        let ms = u64::from(k) * 40_000;
        all.extend(feed_at(&mut core, t0, k, true, ms));
        all.extend(connected(&mut core, t0, k + 1, ms + 10));
        all.extend(respond(&mut core, t0, k + 1, finished_response(), ms + 20));
    }
    assert_eq!(finished(&all), None);
    assert!(connects(&all).len() >= 30);
}

/// Streams frame `i` every 100 ms from `from` to `to` (exclusive) with a
/// final token answering each second; returns every action.
fn stream_answered(
    core: &mut SonioxCore,
    t0: Instant,
    conn: u32,
    from: u32,
    to: u32,
) -> Vec<Action> {
    let mut all = Vec::new();
    for i in from..to {
        all.extend(feed(core, t0, i, true));
        if i % 10 == 9 {
            let end = u64::from(i) * 100;
            all.extend(respond(
                core,
                t0,
                conn,
                response(vec![token(" w", end - 100, end, true)]),
                end + 50,
            ));
        }
    }
    all
}

const MIN: u32 = 600; // frames per minute

#[test]
fn rollover_waits_for_a_gated_moment_after_280_minutes() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let all = stream_answered(&mut core, t0, 1, 1, 285 * MIN);
    // Still speaking at 285 min: no rollover before 295.
    assert!(connects(&all).is_empty());
    let gate = core.on_tick(at(t0, (u64::from(285 * MIN) - 1) * 100 + 10_000));
    assert_eq!(controls(&gate), vec![Control::Finalize]);
    assert!(connects(&gate).is_empty());
    let actions = respond(&mut core, t0, 1, response(vec![fin()]), 17_110_500);
    assert!(actions.contains(&Action::EndAudio { conn: 1 }));
    assert!(actions.contains(&Action::ConnectionEnded { conn: 1 }));
    let new = connects(&actions);
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].0, 2);
    assert_eq!(new[0].2, ConnectKind::Rollover);
    // Everything was confirmed by the `<fin>`: no resend.
    assert_eq!(new[0].1, u64::from(285 * MIN) * 100);
}

/// The first frame after 295 min of connection age (connected at 50 ms).
const ROLL: u32 = 295 * MIN + 1;

#[test]
fn rollover_is_forced_at_295_minutes_without_losing_a_frame() {
    let t0 = Instant::now();
    let mut core = keyed();
    let mut all = feed(&mut core, t0, 0, true);
    all.extend(connected(&mut core, t0, 1, 50));
    all.extend(stream_answered(&mut core, t0, 1, 1, ROLL));
    let roll = feed(&mut core, t0, ROLL, true);
    assert_eq!(controls(&roll), vec![Control::Finalize]);
    assert!(sent(&roll).is_empty());
    all.extend(roll);
    for i in ROLL + 1..ROLL + 10 {
        all.extend(feed(&mut core, t0, i, true));
    }
    let start = u64::from(ROLL) * 100;
    let actions = respond(&mut core, t0, 1, response(vec![fin()]), start + 950);
    assert!(actions.contains(&Action::EndAudio { conn: 1 }));
    assert_eq!(connects(&actions), vec![(2, start, ConnectKind::Rollover)]);
    all.extend(actions);
    all.extend(connected(&mut core, t0, 2, start + 1_000));
    all.extend(feed(&mut core, t0, ROLL + 10, true));
    let ids: Vec<u32> = sent(&all).iter().map(|(_, i)| *i).collect();
    let expected: Vec<u32> = (0..=ROLL + 10).collect();
    assert_eq!(ids, expected);
    let on_two: Vec<u32> = sent(&all)
        .iter()
        .filter(|(c, _)| *c == 2)
        .map(|(_, i)| *i)
        .collect();
    assert_eq!(on_two, (ROLL..=ROLL + 10).collect::<Vec<u32>>());
}

#[test]
fn rollover_waits_at_most_three_seconds_for_fin() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = stream_answered(&mut core, t0, 1, 1, ROLL);
    let start = u64::from(ROLL) * 100;
    let roll = feed(&mut core, t0, ROLL, true);
    assert_eq!(controls(&roll), vec![Control::Finalize]);
    assert_eq!(core.next_deadline(), Some(at(t0, start + 3_000)));
    assert!(connects(&core.on_tick(at(t0, start + 2_999))).is_empty());
    let actions = core.on_tick(at(t0, start + 3_000));
    assert!(actions.contains(&Action::EndAudio { conn: 1 }));
    // No `<fin>`: the audio after the last final (the last 200 ms) goes again.
    assert_eq!(
        connects(&actions),
        vec![(2, start - 200, ConnectKind::Rollover)]
    );
}

#[test]
fn stop_finalizes_ends_audio_and_finishes() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let actions = core.on_input(Input::Stop { at: at(t0, 1_000) });
    assert_eq!(
        actions,
        vec![
            Action::SendControl {
                conn: 1,
                control: Control::Finalize
            },
            Action::EndAudio { conn: 1 }
        ]
    );
    // Frames after the stop go nowhere.
    assert!(sent(&feed(&mut core, t0, 11, true)).is_empty());
    let words = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" last", 0, 900, true)]),
        1_200,
    );
    assert!(words.iter().any(|a| matches!(a, Action::Emit(_))));
    let actions = respond(&mut core, t0, 1, finished_response(), 1_300);
    assert!(actions.contains(&Action::ConnectionEnded { conn: 1 }));
    assert_eq!(actions.last(), Some(&Action::Finished(None)));
    assert!(core.on_tick(at(t0, 9_000)).is_empty());
}

#[test]
fn stop_without_finished_ends_after_three_seconds() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_input(Input::Stop { at: at(t0, 1_000) });
    assert!(finished(&core.on_tick(at(t0, 3_999))).is_none());
    let actions = core.on_tick(at(t0, 4_000));
    assert!(actions.contains(&Action::Drop { conn: 1 }));
    assert_eq!(finished(&actions), Some(None));
}

#[test]
fn stop_without_a_connection_finishes_at_once() {
    let t0 = Instant::now();
    let mut core = keyed();
    let actions = core.on_input(Input::Stop { at: t0 });
    assert_eq!(finished(&actions), Some(None));
    assert!(connects(&feed(&mut core, t0, 1, true)).is_empty());
}

#[test]
fn without_a_key_the_first_frame_finishes_with_no_key() {
    let t0 = Instant::now();
    let mut core = SonioxCore::new(MachineConfig { key_present: false });
    let actions = feed(&mut core, t0, 0, true);
    assert_eq!(actions, vec![Action::Finished(Some(OffReason::NoKey))]);
    assert!(feed(&mut core, t0, 1, true).is_empty());
}

#[test]
fn usage_counts_connected_and_audio_time() {
    let t0 = Instant::now();
    let mut core = keyed();
    let _ = feed(&mut core, t0, 0, true);
    let _ = connected(&mut core, t0, 1, 500);
    for i in 1..30 {
        let _ = feed_at(&mut core, t0, i, true, 500 + u64::from(i) * 100);
    }
    let tick = core.on_tick(at(t0, 10_500));
    assert!(tick.contains(&Action::Usage {
        audio_ms: 3_000,
        connected_ms: 10_000
    }));
    let _ = core.on_input(Input::Stop { at: at(t0, 11_000) });
    let actions = respond(&mut core, t0, 1, finished_response(), 11_500);
    assert!(actions.contains(&Action::Usage {
        audio_ms: 3_000,
        connected_ms: 11_000
    }));
    assert_eq!(core.stats().audio_ms, 3_000);
}

#[test]
fn onset_latency_is_measured_from_the_first_voiced_frame() {
    let t0 = Instant::now();
    let mut core = keyed();
    let _ = feed(&mut core, t0, 0, true);
    let _ = connected(&mut core, t0, 1, 200);
    // A marker is not a word.
    let marker = respond(&mut core, t0, 1, response(vec![fin()]), 500);
    assert!(!marker.iter().any(|a| matches!(a, Action::Onset { .. })));
    let actions = respond(
        &mut core,
        t0,
        1,
        response(vec![token("Hello", 0, 300, false)]),
        900,
    );
    assert!(actions.contains(&Action::Onset { latency_ms: 700 }));
    let again = respond(
        &mut core,
        t0,
        1,
        response(vec![token("Hello", 0, 300, true)]),
        1_100,
    );
    assert!(!again.iter().any(|a| matches!(a, Action::Onset { .. })));
    // After a gate the resume is the next onset.
    let _ = core.on_tick(at(t0, 10_000));
    let _ = feed_at(&mut core, t0, 1, true, 20_000);
    let actions = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" again", 400, 600, false)]),
        20_450,
    );
    assert!(actions.contains(&Action::Onset { latency_ms: 450 }));
}

#[test]
fn tokens_come_out_with_the_connection_and_its_offset() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let actions = respond(
        &mut core,
        t0,
        1,
        response(vec![token("Hi", 0, 80, true)]),
        300,
    );
    let batch = actions.iter().find_map(|a| match a {
        Action::Emit(batch) => Some(batch.clone()),
        _ => None,
    });
    let batch = batch.unwrap_or_else(|| panic!("no batch in {actions:?}"));
    assert_eq!(batch.connection, 1);
    assert_eq!(batch.offset_ms, 0);
    assert_eq!(batch.tokens.len(), 1);
    assert_eq!(core.stats().tokens_final, 1);
}

#[test]
fn a_parked_connection_is_ignored_and_dropped_when_it_ends() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    let _ = core.on_tick(at(t0, 310_000));
    let late = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" late", 0, 50, true)]),
        310_500,
    );
    assert!(late.is_empty(), "{late:?}");
    let end = respond(&mut core, t0, 1, finished_response(), 310_600);
    assert_eq!(end, vec![Action::Drop { conn: 1 }]);
}

#[test]
fn a_draining_connection_is_dropped_after_three_seconds() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    let _ = core.on_tick(at(t0, 310_000));
    assert_eq!(core.next_deadline(), Some(at(t0, 313_000)));
    let actions = core.on_tick(at(t0, 313_000));
    assert_eq!(actions, vec![Action::Drop { conn: 1 }]);
    assert_eq!(core.next_deadline(), None);
}

#[test]
fn a_finished_stream_while_nobody_speaks_waits_for_the_voice() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    let actions = respond(&mut core, t0, 1, finished_response(), 12_000);
    assert!(connects(&actions).is_empty());
    assert_eq!(states(&actions), vec![TranscriptionState::Parked]);
    let actions = feed_at(&mut core, t0, 300, true, 30_000);
    assert_eq!(connects(&actions).len(), 1);
}

#[test]
fn a_backoff_that_ends_in_silence_waits_for_the_voice() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = respond(&mut core, t0, 1, error_response(500), 9_000);
    let _ = core.on_tick(at(t0, 9_500));
    let actions = core.on_tick(at(t0, 10_000));
    assert!(connects(&actions).is_empty());
    assert_eq!(states(&actions), vec![TranscriptionState::Parked]);
    assert_eq!(
        connects(&feed_at(&mut core, t0, 200, true, 20_000)).len(),
        1
    );
}

#[test]
fn reports_of_an_unknown_connection_change_nothing() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    assert!(connected(&mut core, t0, 7, 100).is_empty());
    assert!(respond(&mut core, t0, 7, error_response(401), 100).is_empty());
    let closed = core.on_report(Report::Closed {
        conn: 7,
        code: Some(1000),
        reason: String::new(),
        at: at(t0, 100),
    });
    assert!(closed.is_empty());
    assert_eq!(core.state(), TranscriptionState::Streaming);
}

#[test]
fn a_close_without_finished_backs_off_and_a_good_answer_resets_the_steps() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let closed = |core: &mut SonioxCore, conn: u32, ms: u64| {
        core.on_report(Report::Closed {
            conn,
            code: Some(1000),
            reason: String::new(),
            at: at(t0, ms),
        })
    };
    let _ = feed_at(&mut core, t0, 1, true, 900);
    let _ = closed(&mut core, 1, 1_000);
    assert_eq!(core.next_deadline(), Some(at(t0, 2_000)));
    let _ = feed_at(&mut core, t0, 2, true, 2_000);
    let _ = connected(&mut core, t0, 2, 2_100);
    let _ = respond(&mut core, t0, 2, response(Vec::new()), 2_200);
    let _ = closed(&mut core, 2, 2_300);
    // The answer reset the steps: 1 s again, not 5 s.
    assert_eq!(core.next_deadline(), Some(at(t0, 3_300)));
}

#[test]
fn an_onset_during_a_back_off_keeps_its_pre_roll() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = respond(&mut core, t0, 1, error_response(503), 500);
    let mut conn = 1;
    for ms in [1_500, 6_500] {
        let actions = core.on_tick(at(t0, ms));
        conn = connects(&actions)[0].0;
        let _ = core.on_report(Report::ConnectFailed {
            conn,
            status: None,
            error: "dns".to_string(),
            at: at(t0, ms),
        });
    }
    // 20 s back-off: the guest pauses, then speaks before it ends.
    for i in 70..200 {
        let _ = feed(&mut core, t0, i, false);
    }
    let _ = feed(&mut core, t0, 200, true);
    let actions = core.on_tick(at(t0, 26_500));
    assert_eq!(connects(&actions)[0].0, conn + 1);
    let ids: Vec<u32> = sent(&connected(&mut core, t0, conn + 1, 26_600))
        .iter()
        .map(|(_, i)| *i)
        .collect();
    let tail: Vec<u32> = ids.iter().rev().take(10).rev().copied().collect();
    assert_eq!(tail, (191..=200).collect::<Vec<u32>>());
}

#[test]
fn a_stale_pre_roll_is_not_sent() {
    let t0 = Instant::now();
    let mut core = keyed();
    for i in 0..5 {
        assert!(feed(&mut core, t0, i, false).is_empty());
    }
    // The mic paused for a minute: the old quiet frames are no pre-roll.
    let _ = feed_at(&mut core, t0, 5, true, 60_000);
    let actions = connected(&mut core, t0, 1, 60_100);
    assert_eq!(sent(&actions), vec![(1, 5)]);
}

#[test]
fn each_fin_confirms_only_its_own_finalize() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let gate_a = core.on_tick(at(t0, 10_000));
    assert_eq!(controls(&gate_a), vec![Control::Finalize]);
    for i in 1..=10 {
        let _ = feed_at(&mut core, t0, i, true, 10_000 + u64::from(i) * 100);
    }
    let gate_b = core.on_tick(at(t0, 21_000));
    assert_eq!(controls(&gate_b), vec![Control::Finalize]);
    // The answer to the FIRST finalize arrives; the second is still open.
    let _ = respond(&mut core, t0, 1, response(vec![fin()]), 21_100);
    let _ = core.on_report(Report::Closed {
        conn: 1,
        code: Some(1006),
        reason: String::new(),
        at: at(t0, 21_200),
    });
    let _ = core.on_tick(at(t0, 22_200));
    let actions = feed_at(&mut core, t0, 11, true, 30_000);
    // Everything after the first finalize's 100 ms goes again.
    assert_eq!(connects(&actions), vec![(2, 100, ConnectKind::Lazy)]);
}

#[test]
fn late_finals_of_earlier_speech_do_not_answer_an_onset() {
    let t0 = Instant::now();
    let mut core = streaming(t0);
    let _ = core.on_tick(at(t0, 10_000));
    let _ = feed_at(&mut core, t0, 1, true, 20_000);
    // The gate's finalize answered late: a word of frame 0.
    let late = respond(
        &mut core,
        t0,
        1,
        response(vec![token("Old", 0, 90, true)]),
        20_100,
    );
    assert!(!late.iter().any(|a| matches!(a, Action::Onset { .. })));
    let actions = respond(
        &mut core,
        t0,
        1,
        response(vec![token(" new", 150, 190, false)]),
        20_400,
    );
    assert!(actions.contains(&Action::Onset { latency_ms: 400 }));
}
