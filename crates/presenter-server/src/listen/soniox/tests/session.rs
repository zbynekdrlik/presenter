//! The scenarios of plan Task 5.5: the real `conn` + `session` tasks, the
//! scripted server, the paused clock.

use std::time::Duration;

use presenter_core::listen::transcription::{OffReason, TranscriptionState};

use super::mock::Step;
use super::*;

const FIN: &str = r#"{"tokens":[{"text":"<fin>","is_final":true}]}"#;
const FINISHED: &str = r#"{"tokens":[],"finished":true}"#;

fn final_word(text: &str, start: u64, end: u64) -> String {
    format!(
        r#"{{"tokens":[{{"text":"{text}","start_ms":{start},"end_ms":{end},"confidence":0.9,"is_final":true}}]}}"#
    )
}

fn state(e: &AsrEvent, want: TranscriptionState) -> bool {
    matches!(e, AsrEvent::State { state, .. } if *state == want)
}

#[tokio::test(start_paused = true)]
async fn the_upgrade_carries_bearer_and_user_agent_and_the_first_frame_is_the_config() {
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ExpectAudio { ms: 1000 },
        Step::GoSilent,
    ]]);
    h.speak(15, VOICED).await;
    h.stop().await;
    h.finished().await;

    let headers = h.mock.headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[0]["authorization"].to_str().unwrap(),
        "Bearer test-key-not-real"
    );
    let agent = headers[0]["user-agent"].to_str().unwrap();
    assert!(agent.starts_with("presenter-listen/"), "{agent}");
    let first = h.mock.log().into_iter().next().unwrap();
    let mock::Frame::Text(config) = first.frame else {
        panic!("the first frame is not text: {first:?}");
    };
    let json: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(json["model"], "stt-rt-v6");
    assert_eq!(json["audio_format"], "pcm_s16le");
    assert_eq!(json["sample_rate"], 16_000);
    assert_eq!(json["context"]["terms"][0], "Romans");
    assert!(json.get("api_key").is_none());
    assert!(!config.contains(KEY), "the key is never in the config");
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn audio_goes_out_as_binary_frames_in_order() {
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ExpectAudio { ms: 2000 },
        Step::GoSilent,
    ]]);
    h.speak(20, VOICED).await;
    h.stop().await;
    h.finished().await;

    let audio = h.mock.audio(0);
    assert_eq!(audio.len(), 20);
    for (n, (_, bytes)) in audio.iter().enumerate() {
        assert_eq!(bytes.len(), 3200, "1600 s16 samples");
        assert_eq!(index(bytes), n as u32, "in order, none twice");
        assert_eq!(&bytes[4..6], &MARKER.to_le_bytes(), "little-endian");
    }
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn tokens_come_back_as_batches_with_the_connection_offset() {
    let reply = r#"{"tokens":[
        {"text":"Hello","start_ms":120,"end_ms":480,"confidence":0.97,"is_final":true},
        {"text":" world","start_ms":500,"end_ms":900,"confidence":0.6,"is_final":false}]}"#;
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ExpectAudio { ms: 1000 },
        Step::Reply(reply.replace('\n', "")),
        Step::GoSilent,
    ]]);
    h.speak(15, VOICED).await;
    h.wait_for(|e| matches!(e, AsrEvent::Tokens(_))).await;
    h.stop().await;
    h.finished().await;

    let batch = h
        .events()
        .into_iter()
        .find_map(|e| match e {
            AsrEvent::Tokens(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.connection, 1);
    assert_eq!(batch.offset_ms, 0);
    assert_eq!(batch.tokens.len(), 2);
    assert_eq!(batch.tokens[0].text, "Hello");
    assert_eq!(batch.tokens[0].start_ms, Some(120));
    assert!(batch.tokens[0].is_final);
    assert_eq!(batch.tokens[1].text, " world");
    assert!(!batch.tokens[1].is_final);
    assert!(h
        .events()
        .iter()
        .any(|e| matches!(e, AsrEvent::Onset { .. })));
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn finalize_after_ten_quiet_seconds_returns_fin_and_keepalives_follow() {
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ReplyOnFinalize(FIN.to_string()),
        Step::GoSilent,
    ]]);
    h.speak(10, VOICED).await;
    h.speak(200, QUIET).await;
    let stop_at = h.stop().await;
    h.finished().await;

    let texts: Vec<_> = h
        .mock
        .texts(0)
        .into_iter()
        .filter(|(at, _)| *at < stop_at)
        .collect();
    let finalizes: Vec<_> = texts
        .iter()
        .filter(|(_, t)| t.contains("finalize"))
        .collect();
    assert_eq!(finalizes.len(), 1, "one finalize per pause: {texts:?}");
    let keepalives: Vec<_> = texts
        .iter()
        .filter(|(_, t)| t.contains("keepalive"))
        .map(|(at, _)| *at)
        .collect();
    assert_eq!(keepalives.len(), 2, "{texts:?}");
    assert_eq!(keepalives[0] - finalizes[0].0, Duration::from_secs(5));
    assert_eq!(keepalives[1] - keepalives[0], Duration::from_secs(5));
    // The last voiced frame went out at 0.9 s: the gate is 10 s later.
    let gate_at = finalizes[0].0 - h.mock.audio(0)[0].0;
    assert!(gate_at >= Duration::from_millis(10_900), "{gate_at:?}");
    assert!(gate_at <= Duration::from_millis(11_000), "{gate_at:?}");
    let after_gate = h
        .mock
        .audio(0)
        .iter()
        .filter(|(at, _)| *at > finalizes[0].0)
        .count();
    assert_eq!(after_gate, 0, "no audio while gated");
    let fin = h
        .events()
        .into_iter()
        .any(|e| matches!(e, AsrEvent::Tokens(b) if b.tokens.iter().any(|t| t.text == "<fin>")));
    assert!(fin, "the <fin> answer reaches the assembler");
    assert!(h
        .events()
        .iter()
        .any(|e| state(e, TranscriptionState::Gated)));
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_connection_resends_unconfirmed_audio_once() {
    let mut h = Harness::start(vec![
        vec![
            Step::ExpectConfig,
            Step::ExpectAudio { ms: 3_500 },
            Step::Reply(final_word("one", 2_500, 3_000)),
            Step::Drop,
        ],
        vec![
            Step::ExpectConfig,
            Step::ExpectAudio { ms: 1_000 },
            Step::Reply(final_word("two", 0, 500)),
            Step::GoSilent,
        ],
    ]);
    h.speak(60, VOICED).await;
    h.stop().await;
    h.finished().await;

    let first: Vec<u32> = h.mock.audio(0).iter().map(|(_, b)| index(b)).collect();
    let second: Vec<u32> = h.mock.audio(1).iter().map(|(_, b)| index(b)).collect();
    assert_eq!(first, (0..first.len() as u32).collect::<Vec<_>>());
    assert!(first.len() >= 35, "{first:?}");
    assert_eq!(
        second.first(),
        Some(&30),
        "the resend starts at the last final's end"
    );
    assert_eq!(
        second,
        (30..30 + second.len() as u32).collect::<Vec<_>>(),
        "every unconfirmed frame exactly once, then live"
    );
    assert_eq!(second.last(), Some(&59));
    let events = h.events();
    assert!(events.contains(&AsrEvent::ConnectionEnded(1)));
    assert!(events
        .iter()
        .any(|e| state(e, TranscriptionState::Reconnecting)));
    let batches: Vec<(u32, u64)> = events
        .iter()
        .filter_map(|e| match e {
            AsrEvent::Tokens(b) => Some((b.connection, b.offset_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(
        batches,
        vec![(1, 0), (2, 3_000)],
        "the second connection's tokens carry the session ms of its first byte"
    );
    assert_eq!(h.mock.connects(), 2);
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_401_stops_without_a_second_connection() {
    // An error response, then the close.
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ExpectAudio { ms: 500 },
        Step::Error(401, "unauthenticated"),
    ]]);
    h.speak(100, VOICED).await;
    h.finished().await;
    assert!(h
        .events()
        .contains(&AsrEvent::Finished(Some(OffReason::Auth))));
    assert!(h
        .events()
        .iter()
        .any(|e| state(e, TranscriptionState::Stopped)));
    assert_eq!(h.mock.connects(), 1);

    // A refused upgrade.
    let mut h = Harness::start(vec![vec![Step::RefuseUpgrade(401)]]);
    h.speak(100, VOICED).await;
    h.finished().await;
    assert!(h
        .events()
        .contains(&AsrEvent::Finished(Some(OffReason::Auth))));
    assert_eq!(h.mock.connects(), 1);
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_silent_server_is_dropped_after_fifteen_seconds_of_audio() {
    let mut h = Harness::start(vec![
        vec![Step::ExpectConfig, Step::GoSilent],
        vec![Step::ExpectConfig, Step::GoSilent],
    ]);
    h.speak(200, VOICED).await;
    let reconnecting = h
        .wait_for(|e| state(e, TranscriptionState::Reconnecting))
        .await;
    h.stop().await;
    h.finished().await;

    let connected = h.mock.texts(0)[0].0;
    let silent_for = reconnecting - connected;
    assert!(silent_for >= Duration::from_secs(15), "{silent_for:?}");
    assert!(silent_for < Duration::from_millis(15_100), "{silent_for:?}");
    let error = h.events().into_iter().find_map(|e| match e {
        AsrEvent::State {
            state: TranscriptionState::Reconnecting,
            error,
        } => error,
        _ => None,
    });
    assert!(error.unwrap().contains("neodpovedá"));
    assert_eq!(h.mock.connects(), 2);
    let second = h.mock.texts(1)[0].0;
    assert_eq!(
        second - reconnecting,
        Duration::from_secs(1),
        "the first back-off"
    );
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_client_never_sends_a_ping() {
    // Speech, a long gated pause with keepalives, speech again, a lost
    // connection, a reconnect, a stop: no ping on any connection.
    let mut h = Harness::start(vec![
        vec![
            Step::ExpectConfig,
            Step::ReplyOnFinalize(FIN.to_string()),
            // 2 s of speech + 10 s before the gate + 4 s after the resume.
            Step::ExpectAudio { ms: 16_000 },
            Step::Drop,
        ],
        vec![Step::ExpectConfig, Step::GoSilent],
    ]);
    h.speak(20, VOICED).await;
    h.speak(600, QUIET).await;
    h.speak(50, VOICED).await;
    h.stop().await;
    h.finished().await;

    assert_eq!(h.mock.connects(), 2);
    let keepalives = h
        .mock
        .texts(0)
        .iter()
        .filter(|(_, t)| t.contains("keepalive"))
        .count();
    assert!(keepalives >= 8, "{keepalives}");
    assert!(!h.mock.audio(1).is_empty());
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn stop_ends_audio_with_an_empty_text_frame_and_waits_for_finished() {
    let mut h = Harness::start(vec![vec![
        Step::ExpectConfig,
        Step::ExpectEnd,
        Step::Wait(Duration::from_secs(2)),
        Step::Reply(FINISHED.to_string()),
        Step::Close(1000, ""),
    ]]);
    h.speak(10, VOICED).await;
    let stop_at = h.stop().await;
    let finished_at = h.finished().await;

    let waited = finished_at - stop_at;
    assert!(waited >= Duration::from_secs(2), "{waited:?}");
    assert!(
        waited < Duration::from_secs(3),
        "finished, not the 3 s timeout: {waited:?}"
    );
    let texts: Vec<String> = h.mock.texts(0).into_iter().map(|(_, t)| t).collect();
    let n = texts.len();
    assert!(texts[n - 2].contains("finalize"), "{texts:?}");
    assert_eq!(texts[n - 1], "", "the empty TEXT frame ends the stream");
    assert_eq!(h.events().last(), Some(&AsrEvent::Finished(None)));
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn without_a_key_a_session_finishes_at_once_and_never_connects() {
    let mut h = Harness::start_with(env(None), vec![vec![Step::ExpectConfig]]);
    let opened = Instant::now();
    let finished_at = h.finished().await;
    h.speak(20, VOICED).await;

    assert_eq!(finished_at, opened, "before any frame");
    assert_eq!(h.events(), vec![AsrEvent::Finished(Some(OffReason::NoKey))]);
    assert_eq!(h.mock.connects(), 0);
    assert!(h.mock.headers().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_connect_that_never_completes_fails_after_ten_seconds() {
    let mut h = Harness::start(vec![
        vec![Step::HangUpgrade],
        vec![Step::ExpectConfig, Step::GoSilent],
    ]);
    let opened = Instant::now();
    h.speak(150, VOICED).await;
    let reconnecting = h
        .wait_for(|e| state(e, TranscriptionState::Reconnecting))
        .await;
    h.stop().await;
    h.finished().await;

    assert_eq!(reconnecting - opened, Duration::from_secs(10));
    let error = h.events().into_iter().find_map(|e| match e {
        AsrEvent::State {
            state: TranscriptionState::Reconnecting,
            error,
        } => error,
        _ => None,
    });
    assert!(error.unwrap().contains("no connection within 10 s"));
    assert_eq!(h.mock.connects(), 2);
    assert_eq!(h.mock.texts(1)[0].0 - reconnecting, Duration::from_secs(1));
    assert!(!h.mock.audio(1).is_empty(), "the queued speech goes out");
}

#[tokio::test(start_paused = true)]
async fn a_url_that_makes_no_request_stops_without_retrying() {
    let mut bad = env(Some(KEY));
    bad.url = "not a url".to_string();
    let mut h = Harness::start_with(bad, vec![]);
    h.speak(100, VOICED).await;
    h.finished().await;

    assert!(h
        .events()
        .contains(&AsrEvent::Finished(Some(OffReason::Refused))));
    assert_eq!(h.mock.connects(), 0);
}

#[tokio::test(start_paused = true)]
async fn five_gated_minutes_end_the_connection_with_an_empty_text_frame() {
    let mut h = Harness::start(vec![vec![Step::ExpectConfig, Step::GoSilent]]);
    h.speak(10, VOICED).await;
    h.speak(3_200, QUIET).await;
    let parked = h.wait_for(|e| state(e, TranscriptionState::Parked)).await;
    h.stop().await;
    h.finished().await;

    let texts = h.mock.texts(0);
    let (end_at, end) = texts.last().unwrap();
    assert_eq!(end, "", "the park ends the stream: {texts:?}");
    assert_eq!(*end_at, parked);
    let gated_at = texts
        .iter()
        .find(|(_, t)| t.contains("finalize"))
        .unwrap()
        .0;
    assert_eq!(parked - gated_at, Duration::from_secs(300));
    assert!(h.events().contains(&AsrEvent::ConnectionEnded(1)));
    assert_eq!(
        h.mock.connects(),
        1,
        "nobody spoke again: no new connection"
    );
    assert_eq!(h.mock.pings(), 0);
}

#[tokio::test(start_paused = true)]
async fn reconnect_warnings_restart_per_outage_and_never_show_the_key() {
    let capture = crate::tracing_capture::Capture::install();
    // Outage 1 hits a gated connection: the back-off ends in silence, so the
    // session parks — never streaming between the outages. The park must
    // end the streak, or outage 2 (a connect that times out) would count on
    // from it (attempt 2, and a third outage would not WARN at all).
    let mut h = Harness::start(vec![
        vec![
            Step::ExpectConfig,
            Step::ReplyOnFinalize(FIN.to_string()),
            Step::Close(1011, "internal_error test-key-not-real"),
        ],
        vec![Step::HangUpgrade],
        vec![Step::ExpectConfig, Step::GoSilent],
    ]);
    h.speak(10, VOICED).await;
    h.speak(150, QUIET).await;
    h.wait_for(|e| state(e, TranscriptionState::Parked)).await;
    // 10 s connect timeout + 5 s back-off, then 5 s on the new connection.
    h.speak(200, VOICED).await;
    h.stop().await;
    h.finished().await;

    let warns = capture.events_with_message("Soniox connection lost, reconnecting");
    let attempts: Vec<&str> = warns.iter().filter_map(|l| l.field("attempt")).collect();
    assert_eq!(
        attempts,
        vec!["1", "1"],
        "each outage WARNs its first attempt"
    );
    let waits: Vec<&str> = warns.iter().filter_map(|l| l.field("wait_ms")).collect();
    assert_eq!(waits, vec!["1000", "5000"], "the back-off of each attempt");
    let ended = capture.events_with_message("Soniox reconnect attempts ended");
    assert_eq!(ended.len(), 2);
    let closes = capture.events_with_message("Soniox connection closed");
    let reason = closes[0].field("reason").unwrap();
    assert!(
        reason.contains("<redacted>") && !reason.contains(KEY),
        "{reason}"
    );
    let errors: Vec<String> = h
        .events()
        .into_iter()
        .filter_map(|e| match e {
            AsrEvent::State { error, .. } => error,
            _ => None,
        })
        .collect();
    assert!(
        errors.iter().any(|e| e.contains("<redacted>")),
        "{errors:?}"
    );
    assert!(errors.iter().all(|e| !e.contains(KEY)), "{errors:?}");
    let summaries = capture.events_with_message("transcription session finished");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].field("connections"), Some("3"));
}
