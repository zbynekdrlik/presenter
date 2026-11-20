//! The transcript hub (#852 C3-6): what is kept, what is sent, and that a
//! subscriber's snapshot and stream join without a gap or a double.

use super::*;
use chrono::TimeZone;
use presenter_core::listen::transcription::TranscriptionState;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

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

/// One reader: subscribe `rounds` times, each time check that the snapshot's
/// sentences and the stream after it are one gapless, double-free run of ids.
/// Returns every break of that rule (empty = none).
async fn read_in_rounds(hub: TranscriptHub, total: u64, rounds: usize) -> Vec<String> {
    let mut faults = Vec::new();
    for round in 0..rounds {
        let (snapshot, mut rx) = hub.subscribe();
        let ids: Vec<u64> = snapshot.sentences.iter().map(|s| s.id).collect();
        if ids.windows(2).any(|pair| pair[1] != pair[0] + 1) {
            faults.push(format!("round {round}: snapshot ids not contiguous"));
        }
        let mut expected = ids.last().map_or(1, |last| last + 1);
        let mut seen = 0;
        while expected <= total && seen < 300 {
            match rx.recv().await {
                Ok(ListenWsMessage::Sentence { sentence }) => {
                    if sentence.id != expected {
                        faults.push(format!(
                            "round {round}: got {} after {}",
                            sentence.id,
                            expected - 1
                        ));
                    }
                    expected = sentence.id + 1;
                    seen += 1;
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => break,
                Err(RecvError::Closed) => {
                    faults.push("closed".into());
                    break;
                }
            }
        }
    }
    faults
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscribe_and_snapshot_never_miss_or_double_a_sentence() {
    const TOTAL: u64 = 3_000;
    let hub = TranscriptHub::new();
    let publisher = {
        let hub = hub.clone();
        tokio::spawn(async move {
            for n in 1..=TOTAL {
                let _id = hub.push_sentence("guest", closed(&format!("S{n}.")));
                if n % 50 == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        })
    };
    let readers: Vec<_> = (0..4u64)
        .map(|k| {
            let hub = hub.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(k * 7)).await;
                read_in_rounds(hub, TOTAL, 6).await
            })
        })
        .collect();
    publisher.await.unwrap();
    for reader in readers {
        let faults = reader.await.unwrap();
        assert!(faults.is_empty(), "{faults:?}");
    }
    assert_eq!(hub.snapshot().sentences.last().map(|s| s.id), Some(TOTAL));
}
