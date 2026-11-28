//! The guest translation over the real session task and registry, against the
//! scripted fake Gemini Live server (`test_fake`, #999 L1). On the paused clock.

use std::time::Duration;

use super::stats::{DayMeter, DAILY_CAP_MINUTES};
use super::test_fake::{guest, today, FakeGemini, Script, LATE_MARK};
use super::*;
use crate::listen::{ListenService, MicChunk};

fn listen_with_source() -> ListenService {
    let listen = ListenService::new();
    listen.set_dante_mics(true);
    listen
}

/// A 20 ms chunk (320 samples, what the mic chain hands the session, #999
/// L6e): a loud square wave (voiced) or digital silence.
fn chunk(voiced: bool) -> MicChunk {
    let level = if voiced { 8_000 } else { 0 };
    (0..320)
        .map(|i| if i % 20 < 10 { level } else { -level })
        .collect::<Vec<i16>>()
        .into()
}

/// Sends `count` × 100 ms of audio into the source mic's chunk channel, as
/// the chain does: one 20 ms chunk every 20 ms.
async fn feed(listen: &ListenService, voiced: bool, count: usize) {
    let sender = listen.chunk_sender("dante-mic-1");
    for _ in 0..count * 5 {
        let _ = sender.send(chunk(voiced));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The samples of `n` × 100 ms of 16 kHz input (what [`FakeGemini::audio_samples`] counts).
fn ms100(n: usize) -> usize {
    n * 1_600
}

/// Every packet waiting for the listener now.
fn drain(listener: &mut Listener) -> Vec<GuestPacket> {
    let mut packets = Vec::new();
    while let Ok(packet) = listener.packets.try_recv() {
        packets.push(packet);
    }
    packets
}

fn audio_count(packets: &[GuestPacket]) -> usize {
    packets
        .iter()
        .filter(|p| matches!(p, GuestPacket::Audio(..)))
        .count()
}

#[tokio::test(start_paused = true)]
async fn a_guest_hears_the_translation_and_silence_is_never_sent() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut listener = service.join(&listen, "de").expect("joined");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(*listener.state.borrow(), SessionState::Waiting);
    let setup = &fake.setups()[0]["setup"];
    assert_eq!(setup["model"], "models/gemini-test-translate");
    assert_eq!(
        setup["generationConfig"]["translationConfig"]["targetLanguageCode"],
        "de"
    );
    let headers = fake.headers.lock().unwrap()[0].clone();
    assert_eq!(
        headers.get("x-goog-api-key").map(|v| v.as_bytes()),
        Some(b"test-key-not-real".as_slice())
    );
    assert!(!format!("{:?}", setup).contains("test-key-not-real"));
    assert_eq!(
        setup["systemInstruction"]["parts"][0]["text"],
        super::config::DEFAULT_CONTEXT,
        "every session's setup carries the church context (#999 L6c)"
    );

    feed(&listen, false, 30).await;
    assert_eq!(fake.audio_samples(), vec![0], "a silent mic sends nothing");

    feed(&listen, true, 5).await;
    assert_eq!(*listener.state.borrow(), SessionState::Live);
    // 10 pre-roll frames of the quiet before + the 5 voiced ones.
    assert_eq!(fake.audio_samples(), vec![ms100(15)]);
    // 2 s of hangover is still sent, then the gate closes.
    feed(&listen, false, 40).await;
    assert_eq!(fake.audio_samples(), vec![ms100(15 + 20)]);
    assert_eq!(*listener.state.borrow(), SessionState::Waiting);

    let packets = drain(&mut listener);
    assert_eq!(audio_count(&packets), 35);
    assert!(packets.contains(&GuestPacket::Text("w1 ".to_string())));
    let GuestPacket::Audio(first, _) = &packets[0] else {
        panic!("audio first: {packets:?}");
    };
    assert_eq!(first.len(), 4_800, "24 kHz s16 100 ms");
    assert_eq!(fake.connects(), 1);
}

#[tokio::test(start_paused = true)]
async fn listeners_share_one_session_and_the_last_one_ends_it_after_the_linger() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut first = service.join(&listen, "en").expect("first");
    let mut second = service.join(&listen, "en").expect("second");
    feed(&listen, true, 3).await;
    assert_eq!(fake.connects(), 1, "one session for both");
    assert_eq!(audio_count(&drain(&mut first)), 3);
    assert_eq!(audio_count(&drain(&mut second)), 3);
    assert_eq!(service.status(&listen).active, vec!["en"]);

    drop(first);
    drop(second);
    tokio::time::sleep(LINGER / 2).await;
    let again = service
        .join(&listen, "en")
        .expect("rejoin within the linger");
    tokio::time::sleep(LINGER * 2).await;
    assert_eq!(
        service.status(&listen).active,
        vec!["en"],
        "a rejoin keeps it"
    );
    assert_eq!(fake.connects(), 1);
    drop(again);
    tokio::time::sleep(LINGER + Duration::from_millis(100)).await;
    assert!(service.status(&listen).active.is_empty());
    // A frame now reaches no session: nothing more is sent.
    feed(&listen, true, 3).await;
    assert_eq!(fake.audio_samples(), vec![ms100(3)]);
}

#[tokio::test(start_paused = true)]
async fn refusals_unknown_language_no_key_no_source_full_and_the_daily_cap() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    assert_eq!(
        service.join(&listen, "sk").err(),
        Some(Refusal::UnknownLanguage)
    );
    let held: Vec<Listener> = ["en", "de", "hu", "uk"]
        .iter()
        .map(|code| service.join(&listen, code).expect("joined"))
        .collect();
    assert_eq!(service.join(&listen, "pl").err(), Some(Refusal::Full));
    assert!(
        service.join(&listen, "de").is_ok(),
        "a running language takes more guests"
    );
    assert_eq!(service.status(&listen).active, vec!["de", "en", "hu", "uk"]);
    drop(held);

    let no_key = guest(&fake, None, DayMeter::default());
    assert_eq!(no_key.join(&listen, "en").err(), Some(Refusal::NoKey));
    assert_eq!(no_key.status(&listen).refusal, Some(Refusal::NoKey));

    let no_source = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let bare = ListenService::new();
    assert_eq!(no_source.join(&bare, "en").err(), Some(Refusal::NoSource));

    let spent = DayMeter::default();
    assert!(spent.spend(today(), DAILY_CAP_MINUTES * 60_000));
    let capped = guest(&fake, Some("test-key-not-real"), spent);
    assert_eq!(capped.join(&listen, "en").err(), Some(Refusal::DailyCap));
    assert!(!capped.status(&listen).available);
}

#[tokio::test(start_paused = true)]
async fn the_cap_reached_mid_session_stops_the_language() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let meter = DayMeter::default();
    assert!(meter.spend(today(), DAILY_CAP_MINUTES * 60_000 - 300));
    let service = guest(&fake, Some("test-key-not-real"), meter);
    let listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 10).await;
    assert_eq!(
        *listener.state.borrow(),
        SessionState::Stopped {
            reason: StopReason::DailyCap
        }
    );
    assert_eq!(
        fake.audio_samples(),
        vec![15 * 320],
        "exactly the 300 ms left: 15 chunks of 20 ms, the 16th refused"
    );
}

#[tokio::test(start_paused = true)]
async fn a_go_away_moves_the_audio_to_a_resumed_connection_without_a_gap() {
    let fake = FakeGemini::new(vec![Script {
        go_away_after: Some(12),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 30).await;
    assert_eq!(fake.connects(), 2);
    let setups = fake.setups();
    assert_eq!(
        setups[0]["setup"]["sessionResumption"],
        serde_json::json!({})
    );
    assert_eq!(setups[1]["setup"]["sessionResumption"]["handle"], "h-0-12");
    let samples = fake.audio_samples();
    assert_eq!(
        samples.iter().sum::<usize>(),
        30 * 1_600,
        "every chunk sent once: {samples:?}"
    );
    assert!(samples[0] >= ms100(12) && samples[1] > 0, "{samples:?}");
    assert_eq!(audio_count(&drain(&mut listener)), 30);
    assert_eq!(*listener.state.borrow(), SessionState::Live);
}

#[tokio::test(start_paused = true)]
async fn an_idle_cut_reconnects_at_once_with_the_handle() {
    let fake = FakeGemini::new(vec![Script {
        idle_cut_after: Some(Duration::from_secs(120)),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 5).await;
    // the preacher is quiet for 2 min: Gemini cuts the idle connection
    tokio::time::sleep(Duration::from_secs(120)).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(fake.connects(), 2, "reopened without a back-off");
    assert_eq!(
        fake.setups()[1]["setup"]["sessionResumption"]["handle"],
        "h-0-5"
    );
    feed(&listen, true, 3).await;
    assert_eq!(fake.audio_samples(), vec![ms100(5), ms100(3)]);
    assert_eq!(*listener.state.borrow(), SessionState::Live);
}

#[tokio::test(start_paused = true)]
async fn an_idle_cut_of_a_young_connection_backs_off() {
    let fake = FakeGemini::new(vec![
        Script {
            idle_cut_after: Some(Duration::from_secs(5)),
            ..Script::default()
        },
        Script::default(),
    ]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let listener = service.join(&listen, "en").expect("joined");
    tokio::time::sleep(Duration::from_millis(5_100)).await;
    assert_eq!(*listener.state.borrow(), SessionState::Reconnecting);
    assert_eq!(
        fake.connects(),
        1,
        "a cut after 5 s is no idle cut: back-off"
    );
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(fake.connects(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_answers_nothing_is_replaced() {
    let fake = FakeGemini::new(vec![Script {
        deaf_after: Some(3),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 95).await;
    assert_eq!(
        fake.connects(),
        1,
        "9.5 s without an answer is still patience"
    );
    // the 1 s tick sees 10 s without an answer at t = 11 s, the retry follows 1 s later
    feed(&listen, true, 30).await;
    assert_eq!(
        fake.connects(),
        2,
        "dropped after 10 s and reopened after 1 s"
    );
    assert_eq!(*listener.state.borrow(), SessionState::Live);
    assert!(
        audio_count(&drain(&mut listener)) > 3,
        "the new connection speaks"
    );
}

/// #999 L6a review: the model streams silence too, so the new connection's
/// first SILENT chunks are no speech — the old one's backlog still plays,
/// in one piece (the new one's silence is not mixed into it).
#[tokio::test(start_paused = true)]
async fn the_old_backlog_plays_while_the_new_connection_is_only_silent() {
    let fake = FakeGemini::new(vec![
        Script {
            go_away_after: Some(5),
            late_speech_after_go_away: Some(Duration::from_millis(1_500)),
            ..Script::default()
        },
        Script {
            quiet: std::iter::once(1..1_000).collect(),
            ..Script::default()
        },
    ]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 30).await;
    assert_eq!(fake.connects(), 2);
    let audio: Vec<bool> = drain(&mut listener)
        .iter()
        .filter_map(|p| match p {
            GuestPacket::Audio(pcm, _) => Some(i16::from_le_bytes([pcm[0], pcm[1]]) == LATE_MARK),
            _ => None,
        })
        .collect();
    let late: Vec<usize> = (0..audio.len()).filter(|&i| audio[i]).collect();
    assert_eq!(
        late.len(),
        5,
        "the old connection's backlog reaches the guest"
    );
    assert_eq!(late[4] - late[0], 4, "in one piece, no silence mixed in");
}

#[tokio::test(start_paused = true)]
async fn a_replaced_connection_never_speaks_over_the_new_one() {
    let fake = FakeGemini::new(vec![Script {
        go_away_after: Some(5),
        late_speech_after_go_away: Some(Duration::from_millis(1_500)),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let mut listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 30).await;
    assert_eq!(fake.connects(), 2);
    let late = drain(&mut listener)
        .iter()
        .filter(|p| match p {
            GuestPacket::Audio(pcm, _) => i16::from_le_bytes([pcm[0], pcm[1]]) == LATE_MARK,
            _ => false,
        })
        .count();
    assert_eq!(
        late, 0,
        "the old connection's backlog is dropped once the new one spoke"
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_key_stops_for_good_and_a_server_error_backs_off() {
    let refused = FakeGemini::new(vec![Script {
        close_on_setup: Some((1007, "API key not valid. Please pass a valid API key.")),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&refused, Some("test-key-not-real"), DayMeter::default());
    let listener = service.join(&listen, "en").expect("joined");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *listener.state.borrow(),
        SessionState::Stopped {
            reason: StopReason::Key
        }
    );
    feed(&listen, true, 50).await;
    assert_eq!(refused.connects(), 1, "never retried");

    let flaky = FakeGemini::new(vec![Script {
        close_after: Some((2, 1011, "Internal error")),
        ..Script::default()
    }]);
    let other = guest(&flaky, Some("test-key-not-real"), DayMeter::default());
    let listener = other.join(&listen, "en").expect("joined");
    feed(&listen, true, 2).await;
    assert_eq!(*listener.state.borrow(), SessionState::Reconnecting);
    // The preacher goes on during the outage: held, never a connect before the back-off.
    feed(&listen, true, 5).await;
    assert_eq!(flaky.connects(), 1);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(flaky.connects(), 2, "retried after 1 s");
    feed(&listen, true, 2).await;
    assert_eq!(*listener.state.borrow(), SessionState::Live);
    // The audio of the outage was held and sent to the new connection.
    assert_eq!(flaky.audio_samples(), vec![ms100(2), ms100(5 + 2)]);
}

#[test]
fn the_retry_delay_doubles_up_to_30_s() {
    use super::session::retry_delay;
    let delays: Vec<u64> = (1..=7).map(|f| retry_delay(f).as_secs()).collect();
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30]);
}

#[tokio::test(start_paused = true)]
async fn a_rejoin_restarts_the_linger() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    drop(service.join(&listen, "en").expect("first"));
    tokio::time::sleep(LINGER / 2).await;
    // rejoin and leave again: the first linger must not end the session
    drop(service.join(&listen, "en").expect("again"));
    tokio::time::sleep(LINGER / 2 + Duration::from_millis(100)).await;
    assert_eq!(
        service.status(&listen).active,
        vec!["en"],
        "60 s after the LAST leave, not the first"
    );
    tokio::time::sleep(LINGER / 2).await;
    assert!(service.status(&listen).active.is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_listener_cap_refuses_the_next_guest() {
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let held: Vec<Listener> = (0..MAX_LISTENERS)
        .map(|i| service.join(&listen, ["en", "de"][i % 2]).expect("joined"))
        .collect();
    assert_eq!(service.join(&listen, "en").err(), Some(Refusal::Busy));
    drop(held);
    assert!(service.join(&listen, "en").is_ok());
}

#[test]
fn speech_follows_a_pause_of_750_ms_audio_or_wall_time() {
    use super::session::{after_pause, output_voiced, OUTPUT_GAP};
    use super::test_fake::pcm;
    let now = tokio::time::Instant::now();
    assert!(output_voiced(&pcm(0, true)), "the fake's speech");
    assert!(!output_voiced(&pcm(120, false)), "the model's near-silence");
    assert!(after_pause(0, None, now), "nothing spoken yet");
    assert!(after_pause(750, Some(now), now), "750 ms of quiet audio");
    assert!(!after_pause(
        500,
        Some(now - Duration::from_millis(100)),
        now
    ));
    assert!(
        after_pause(0, Some(now - OUTPUT_GAP), now),
        "no output at all for 750 ms"
    );
}

/// The per-onset DEBUG lines' values (`… after an onset`).
fn timed_onsets(capture: &crate::tracing_capture::Capture) -> Vec<String> {
    capture
        .events_with_message("after an onset")
        .iter()
        .filter_map(|line| line.field("first_voiced_ms").map(str::to_string))
        .collect()
}

/// #999 L6a: the model streams audio all the time, silence included, so an
/// onset is timed to the first SPEECH-level output after the model's own
/// pause — every onset, never the first chunk of any level.
#[tokio::test(start_paused = true)]
async fn every_onset_is_timed_to_the_first_voiced_translation() {
    let capture = crate::tracing_capture::Capture::install();
    // Audio messages 1–30 and 41–80 are answered with the model's
    // near-silence: its latency to speech.
    let fake = FakeGemini::new(vec![Script {
        quiet: vec![1..31, 41..81],
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let _listener = service.join(&listen, "en").expect("joined");
    feed(&listen, false, 30).await;
    // Onset 1: pre-roll 10 + 1 = messages 1–11 at once, then one per 100 ms:
    // message 31 (the first voiced answer) goes out 2.0 s after the onset.
    feed(&listen, true, 25).await;
    // 2 s hangover (messages 36–55), the gate closes, 1 s of quiet.
    feed(&listen, false, 30).await;
    // Onset 2: messages 56–66 at once, message 81 goes out 1.5 s after it.
    feed(&listen, true, 20).await;
    assert_eq!(timed_onsets(&capture), vec!["2000", "1500"]);

    tokio::time::sleep(Duration::from_secs(60)).await;
    let summary = capture.events_with_message("guest translation minute summary");
    assert_eq!(summary.len(), 1, "one minute summary");
    assert_eq!(summary[0].field("onsets"), Some("2"));
    assert_eq!(summary[0].field("first_voiced_timed"), Some("2"));
    assert_eq!(summary[0].field("first_voiced_ms_median"), Some("2000"));
    assert_eq!(summary[0].field("first_voiced_ms_max"), Some("2000"));
}

/// #999 L6a: an onset heard while the previous phrase is still being spoken
/// is never timed on that backlog (it would read as ~0 s).
#[tokio::test(start_paused = true)]
async fn an_onset_during_the_backlog_is_not_timed_on_it() {
    let capture = crate::tracing_capture::Capture::install();
    // After the first second the model answers speech to everything: no
    // pause to time the second onset from.
    let fake = FakeGemini::new(vec![Script {
        quiet: std::iter::once(1..21).collect(),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let _listener = service.join(&listen, "en").expect("joined");
    feed(&listen, false, 30).await;
    feed(&listen, true, 10).await;
    // A 1.6 s pause is a new onset, but the gate (2 s hangover) never
    // closed: the model keeps speaking the previous phrase.
    feed(&listen, false, 16).await;
    feed(&listen, true, 10).await;
    assert_eq!(
        timed_onsets(&capture),
        vec!["1000"],
        "only the first onset is timed"
    );
}

/// #999 L6a review: a translation that streams only silence while the
/// preacher speaks is a stall (6 s from the onset), and its onset, never
/// answered, is counted as expired after 20 s — never silently lost.
#[tokio::test(start_paused = true)]
async fn only_silence_back_is_a_stall_and_the_onset_expires() {
    let capture = crate::tracing_capture::Capture::install();
    let fake = FakeGemini::new(vec![Script {
        quiet: std::iter::once(1..10_000).collect(),
        ..Script::default()
    }]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let _listener = service.join(&listen, "en").expect("joined");
    feed(&listen, true, 80).await;
    let stalls = capture.events_with_message("stalled while the preacher speaks");
    assert_eq!(stalls.len(), 1, "one stall, 6 s after the onset");
    feed(&listen, false, 150).await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    let summary = capture.events_with_message("guest translation minute summary");
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].field("onsets"), Some("1"));
    assert_eq!(summary[0].field("first_voiced_timed"), Some("0"));
    assert_eq!(summary[0].field("onsets_expired"), Some("1"));
    assert_eq!(summary[0].field("stalls"), Some("1"));
}

/// #999 L6a: our own delay to Gemini is timed per live frame (the pre-roll
/// is old on purpose and left out) and reported, then drained, by the
/// minute summary.
#[tokio::test(start_paused = true)]
async fn the_frames_to_gemini_are_timed_and_summarised() {
    let capture = crate::tracing_capture::Capture::install();
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let listener = service.join(&listen, "en").expect("joined");
    feed(&listen, false, 30).await;
    feed(&listen, true, 5).await;
    feed(&listen, false, 40).await;
    assert_eq!(
        fake.audio_samples(),
        vec![ms100(35)],
        "10 pre-roll + 5 voiced + 20 hangover"
    );
    let segments = listener.times.peek();
    assert_eq!(
        segments.to_gemini.count(),
        125,
        "the live chunks only (2.5 s of 20 ms)"
    );
    assert_eq!(segments.to_gemini.max(), Some(0), "the paused clock");
    assert_eq!(segments.to_guest.count(), 0, "no guest socket here");

    tokio::time::sleep(Duration::from_secs(60)).await;
    let summary = capture.events_with_message("guest translation minute summary");
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].field("to_gemini_chunks"), Some("125"));
    assert_eq!(summary[0].field("to_gemini_us_median"), Some("0"));
    assert_eq!(summary[0].field("to_guest_chunks"), Some("0"));
    assert_eq!(summary[0].field("to_guest_us_max"), None, "no sample");
    assert_eq!(listener.times.peek().to_gemini.count(), 0, "drained");
}

/// #999 L6e (owner 27.11: our side adds minimal latency): the audio goes to
/// Gemini in 20 ms chunks, and an onset's first chunk leaves the moment it
/// reaches the session — never held until 100 ms of audio exist. `sent_ms`
/// (and with it the day's cap) counts 20 ms per chunk.
#[tokio::test(start_paused = true)]
async fn the_audio_goes_out_in_20_ms_chunks_without_waiting_for_100_ms() {
    let capture = crate::tracing_capture::Capture::install();
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    let listener = service.join(&listen, "en").expect("joined");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(*listener.state.borrow(), SessionState::Waiting);

    let sent_at = tokio::time::Instant::now();
    let _ = listen.chunk_sender("dante-mic-1").send(chunk(true));
    // The paused clock moves while this waits: a session that held the chunk
    // for more audio (a 100 ms frame) fails here instead of hanging.
    tokio::time::timeout(Duration::from_millis(19), async {
        while fake.messages().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the first chunk left within 19 ms");
    let (samples, arrived) = fake.messages()[0];
    assert_eq!(samples, 320, "one 20 ms chunk (the pre-roll was empty)");
    assert!(
        arrived - sent_at < Duration::from_millis(19),
        "it left at once: {:?}",
        arrived - sent_at
    );

    feed(&listen, true, 10).await;
    let sizes: Vec<usize> = fake.messages().iter().map(|(n, _)| *n).collect();
    assert_eq!(sizes.len(), 51, "1 + 50 chunks");
    assert!(
        sizes.iter().all(|&n| n == 320),
        "every message 20 ms: {sizes:?}"
    );

    tokio::time::sleep(Duration::from_secs(60)).await;
    let summary = capture.events_with_message("guest translation minute summary");
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].field("sent_ms"), Some("1020"), "51 × 20 ms");
}

#[tokio::test(start_paused = true)]
async fn the_default_source_pulpit_translates_whichever_handheld_speaks() {
    // #852 28.11: the default source is `pulpit`, the one voiced Dante mic —
    // here the 2nd handheld; the 1st stays digital silence
    assert_eq!(super::config::DEFAULT_SOURCE_MIC, "pulpit");
    let fake = FakeGemini::new(vec![]);
    let listen = listen_with_source();
    tokio::spawn(crate::listen::pulpit::supervise(listen.clone()));
    let mut deps = guest(&fake, Some("test-key-not-real"), DayMeter::default())
        .inner
        .deps
        .clone();
    deps.env.source_mic = super::config::DEFAULT_SOURCE_MIC.to_string();
    let service = GuestTranslate::with_deps(deps);
    assert_eq!(service.status(&listen).refusal, None);
    assert_eq!(
        service.status(&ListenService::new()).refusal,
        Some(Refusal::NoSource),
        "no Dante mic, no pulpit"
    );
    let mut listener = service.join(&listen, "en").expect("joined");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let first = listen.chunk_sender("dante-mic-1");
    let second = listen.chunk_sender("dante-mic-2");
    for _ in 0..25 {
        let _ = first.send(chunk(false));
        let _ = second.send(chunk(true));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        fake.audio_samples(),
        vec![ms100(5)],
        "exactly the 2nd mic's 25 voiced chunks"
    );
    assert_eq!(*listener.state.borrow(), SessionState::Live);
    assert_eq!(audio_count(&drain(&mut listener)), 5);
}
