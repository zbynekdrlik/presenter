//! The transcript hub (#852 C3-6): what is kept, what is sent, and that a
//! subscriber's snapshot and stream join without a gap or a double.

use super::*;
use chrono::TimeZone;
use presenter_core::listen::transcription::TranscriptionState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use tokio::sync::broadcast::error::TryRecvError;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, 5)
        .single()
        .unwrap()
}

fn closed(text: &str) -> ClosedSentence {
    ClosedSentence {
        text: text.to_string(),
        start_ms: 1_000,
        end_ms: 2_000,
        closed_at: at(),
        language: Some("en".into()),
    }
}

fn tail(final_text: &str, partial_text: &str) -> TailText {
    TailText {
        final_text: final_text.to_string(),
        partial_text: partial_text.to_string(),
        start_ms: Some(1_000),
    }
}

fn session(state: TranscriptionState, audio_ms_today: u64) -> MicTranscription {
    MicTranscription {
        mic_id: "guest".into(),
        state,
        audio_ms_today,
        ..MicTranscription::default()
    }
}

/// Every message waiting in the receiver, in order.
fn drain(rx: &mut broadcast::Receiver<ListenWsMessage>) -> Vec<ListenWsMessage> {
    let mut messages = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(message) => messages.push(message),
            Err(TryRecvError::Empty) => return messages,
            Err(other) => panic!("the receiver failed: {other:?}"),
        }
    }
}

fn tail_texts(messages: &[ListenWsMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            ListenWsMessage::Tail { tail } => {
                Some(format!("{}|{}", tail.final_text, tail.partial_text))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn only_the_newest_500_sentences_are_kept() {
    let hub = TranscriptHub::new();
    let ids: Vec<u64> = (0..=SENTENCES_KEPT)
        .map(|n| hub.push_sentence("guest", closed(&format!("Sentence {n}."))))
        .collect();
    assert_eq!(ids.first(), Some(&1));
    assert_eq!(ids.last(), Some(&501), "ids increase by one, never reused");
    let snapshot = hub.snapshot();
    assert_eq!(snapshot.sentences.len(), SENTENCES_KEPT);
    assert_eq!(snapshot.sentences[0].id, 2, "the oldest one went");
    assert_eq!(snapshot.sentences[0].text, "Sentence 1.");
    assert_eq!(snapshot.sentences[499].id, 501);
    assert_eq!(snapshot.sentences[499].mic_id, "guest");
    assert_eq!(snapshot.sentences[499].language.as_deref(), Some("en"));
}

#[test]
fn session_messages_only_on_a_visible_change() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    hub.set_session(session(TranscriptionState::Armed, 0));
    hub.set_session(session(TranscriptionState::Armed, 5_000));
    let mut streaming = session(TranscriptionState::Streaming, 6_000);
    hub.set_session(streaming.clone());
    streaming.audio_ms_today = 7_000;
    streaming.connected_ms_today = 9_000;
    hub.set_session(streaming.clone());
    streaming.error = Some("Služba prepisu nedostupná".into());
    hub.set_session(streaming.clone());
    let states: Vec<(TranscriptionState, u64, Option<String>)> = drain(&mut rx)
        .into_iter()
        .filter_map(|message| match message {
            ListenWsMessage::Session { session } => {
                Some((session.state, session.audio_ms_today, session.error))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        vec![
            (TranscriptionState::Armed, 0, None),
            (TranscriptionState::Streaming, 6_000, None),
            (
                TranscriptionState::Streaming,
                7_000,
                Some("Služba prepisu nedostupná".into())
            ),
        ],
        "counters alone never send a message"
    );
    assert_eq!(
        hub.snapshot().sessions,
        vec![streaming],
        "the snapshot carries the newest counters"
    );
}

#[tokio::test(start_paused = true)]
async fn tails_are_coalesced_to_ten_per_second() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    let start = Instant::now();
    let mut sent_at = Vec::new();
    let mut texts = Vec::new();
    // Drained right after every set and every 20 ms step, so a message's
    // receipt time is its send time.
    let mut take = |rx: &mut broadcast::Receiver<ListenWsMessage>| {
        for text in tail_texts(&drain(rx)) {
            sent_at.push(Instant::now() - start);
            texts.push(text);
        }
    };
    for n in 0..50 {
        hub.set_tail("guest", Some(tail("For God", &format!(" word {n}"))), at());
        take(&mut rx);
        tokio::time::advance(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
        take(&mut rx);
    }
    tokio::time::advance(Duration::from_millis(200)).await;
    tokio::task::yield_now().await;
    take(&mut rx);
    assert!(
        (10..=11).contains(&texts.len()),
        "50 tails in one second → about ten messages: {texts:?}"
    );
    for pair in sent_at.windows(2) {
        assert!(
            pair[1] - pair[0] >= TAIL_MIN_INTERVAL,
            "two tails less than 100 ms apart: {sent_at:?}"
        );
    }
    assert_eq!(
        texts.last().map(String::as_str),
        Some("For God| word 49"),
        "the last tail is never swallowed"
    );
    assert_eq!(hub.snapshot().tails[0].partial_text, " word 49");
}

#[tokio::test(start_paused = true)]
async fn a_tail_after_a_sentence_goes_out_at_once_and_an_ended_tail_clears() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    hub.set_tail("guest", Some(tail("Turn with me", " to")), at());
    tokio::time::advance(Duration::from_millis(10)).await;
    hub.push_sentence("guest", closed("Turn with me to John 3:16."));
    hub.set_tail("guest", None, at());
    let messages = drain(&mut rx);
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert!(matches!(&messages[1], ListenWsMessage::Sentence { sentence } if sentence.id == 1));
    assert_eq!(
        tail_texts(&messages),
        vec!["Turn with me| to".to_string(), "|".to_string()],
        "the cleared tail follows its sentence without waiting for the window"
    );
    let snapshot = hub.snapshot();
    assert!(snapshot.tails.is_empty(), "{snapshot:?}");
    assert_eq!(snapshot.sentences.len(), 1);
}

#[test]
fn outside_a_runtime_a_coalesced_tail_is_sent_at_once() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    hub.set_tail("guest", Some(tail("a", "")), at());
    hub.set_tail("guest", Some(tail("a b", "")), at());
    assert_eq!(
        tail_texts(&drain(&mut rx)),
        vec!["a|".to_string(), "a b|".to_string()]
    );
}

/// Advance the paused clock `steps` × 10 ms, draining after each step;
/// every tail received with the paused-clock ms it went out at.
async fn tails_over(
    rx: &mut broadcast::Receiver<ListenWsMessage>,
    start: Instant,
    steps: u32,
) -> Vec<(u128, String)> {
    let mut received = Vec::new();
    for _ in 0..steps {
        tokio::time::advance(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
        for text in tail_texts(&drain(rx)) {
            received.push(((Instant::now() - start).as_millis(), text));
        }
    }
    received
}

#[tokio::test(start_paused = true)]
async fn a_sentence_clears_a_pending_tail() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    let start = Instant::now();
    hub.set_tail("guest", Some(tail("Turn with me", "")), at());
    tokio::time::advance(Duration::from_millis(20)).await;
    hub.set_tail("guest", Some(tail("Turn with me to John 3:16.", "")), at());
    hub.push_sentence("guest", closed("Turn with me to John 3:16."));
    let sent = drain(&mut rx);
    assert_eq!(sent.len(), 2, "the first tail and the sentence: {sent:?}");
    let later = tails_over(&mut rx, start, 20).await;
    assert!(
        later.is_empty(),
        "the pending tail held the closed words and must never go out: {later:?}"
    );
    assert!(hub.snapshot().tails.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_tail_after_a_forced_send_waits_for_the_new_window() {
    let hub = TranscriptHub::new();
    let (_, mut rx) = hub.subscribe();
    let start = Instant::now();
    hub.set_tail("guest", Some(tail("a", "")), at());
    tokio::time::advance(Duration::from_millis(20)).await;
    hub.set_tail("guest", Some(tail("a b", "")), at());
    tokio::time::advance(Duration::from_millis(30)).await;
    hub.push_sentence("guest", closed("A b."));
    hub.set_tail("guest", Some(tail("c", "")), at());
    tokio::time::advance(Duration::from_millis(10)).await;
    hub.set_tail("guest", Some(tail("c d", "")), at());
    let early = tail_texts(&drain(&mut rx));
    assert_eq!(early, vec!["a|".to_string(), "c|".to_string()]);
    let later = tails_over(&mut rx, start, 15).await;
    assert_eq!(
        later,
        vec![(150, "c d|".to_string())],
        "the tail waits 100 ms after the forced send at 50 ms, not the old 100 ms deadline"
    );
}

/// What one reader thread saw: rounds in which it checked at least one
/// message, and every break of the gapless / double-free rule.
#[derive(Default)]
struct ReaderReport {
    checked_rounds: u32,
    faults: Vec<String>,
}

/// Subscribe again and again while the publisher runs; each time the
/// snapshot's ids and the next ≤ 5 streamed ids must be one gapless run.
fn read_while_publishing(hub: &TranscriptHub, done: &AtomicBool) -> ReaderReport {
    let mut report = ReaderReport::default();
    while !done.load(Ordering::SeqCst) {
        let (snapshot, mut rx) = hub.subscribe();
        let ids: Vec<u64> = snapshot.sentences.iter().map(|s| s.id).collect();
        if ids.windows(2).any(|pair| pair[1] != pair[0] + 1) {
            report.faults.push("snapshot ids not contiguous".into());
        }
        let mut expected = ids.last().map_or(1, |last| last + 1);
        let mut seen = 0;
        while seen < 5 {
            match rx.try_recv() {
                Ok(ListenWsMessage::Sentence { sentence }) => {
                    if sentence.id != expected {
                        report
                            .faults
                            .push(format!("got {} after {}", sentence.id, expected - 1));
                    }
                    expected = sentence.id + 1;
                    seen += 1;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) if done.load(Ordering::SeqCst) => break,
                Err(TryRecvError::Empty) => std::thread::yield_now(),
                Err(TryRecvError::Lagged(_)) => break,
                Err(TryRecvError::Closed) => {
                    report.faults.push("closed".into());
                    break;
                }
            }
        }
        if seen > 0 {
            report.checked_rounds += 1;
        }
    }
    report
}

#[test]
fn subscribe_and_snapshot_never_miss_or_double_a_sentence() {
    const TOTAL: u64 = 50_000;
    let hub = TranscriptHub::new();
    let done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(5));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (hub, done, start) = (hub.clone(), Arc::clone(&done), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                read_while_publishing(&hub, &done)
            })
        })
        .collect();
    start.wait();
    for n in 1..=TOTAL {
        let _id = hub.push_sentence("guest", closed(&format!("S{n}.")));
        if n % 10 == 0 {
            std::thread::yield_now();
        }
    }
    done.store(true, Ordering::SeqCst);
    let reports: Vec<ReaderReport> = readers.into_iter().map(|r| r.join().unwrap()).collect();
    let faults: Vec<&String> = reports.iter().flat_map(|r| &r.faults).collect();
    assert!(faults.is_empty(), "{faults:?}");
    let checked: u32 = reports.iter().map(|r| r.checked_rounds).sum();
    assert!(
        checked >= 100,
        "the readers must subscribe while sentences flow, not after: {checked} rounds"
    );
    assert_eq!(hub.snapshot().sentences.last().map(|s| s.id), Some(TOTAL));
}
