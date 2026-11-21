//! The transcript store: the hub's never-waiting feed, the writer, the seed
//! and the once-per-date retention (#852 C3-persist).

use super::*;
use crate::listen::recorder::RecorderEnv;
use crate::listen::transcript_hub::TranscriptHub;
use crate::tracing_capture::{quoted, Capture};
use chrono::{NaiveDateTime, TimeZone, Utc};
use presenter_core::listen::transcript::ClosedSentence;
use std::sync::Mutex;

fn closed(text: &str) -> ClosedSentence {
    ClosedSentence {
        text: text.to_string(),
        start_ms: 1_000,
        end_ms: 1_900,
        closed_at: Utc
            .with_ymd_and_hms(2026, 11, 22, 9, 30, 5)
            .single()
            .expect("time"),
        language: Some("en".to_string()),
    }
}

fn sentence(id: u64, text: &str) -> TranscriptSentence {
    let closed = closed(text);
    TranscriptSentence {
        id,
        mic_id: "guest".to_string(),
        text: closed.text,
        start_ms: closed.start_ms,
        end_ms: closed.end_ms,
        closed_at: closed.closed_at,
        language: closed.language,
    }
}

fn day(text: &str) -> NaiveDate {
    text.parse().expect("date")
}

/// A listen service whose local clock the test moves.
fn service_at(now: &str) -> (ListenService, Arc<Mutex<NaiveDateTime>>) {
    let clock = Arc::new(Mutex::new(now.parse::<NaiveDateTime>().expect("time")));
    let read = Arc::clone(&clock);
    let listen = ListenService::new();
    listen.set_recorder_env(RecorderEnv {
        root: std::env::temp_dir().join("presenter-transcript-store-never-written"),
        clock: Arc::new(move || *read.lock().expect("clock")),
        first_pass_delay: Duration::ZERO,
    });
    (listen, clock)
}

/// Records the prune cutoffs and inserts; inserts fail while `fail` is set.
#[derive(Default)]
struct FakeRows {
    cutoffs: Mutex<Vec<NaiveDate>>,
    inserted: Mutex<Vec<NewTranscriptSentence>>,
    fail: Mutex<bool>,
}

#[async_trait]
impl TranscriptRows for FakeRows {
    async fn insert(&self, rows: &[NewTranscriptSentence]) -> anyhow::Result<u64> {
        if *self.fail.lock().expect("fail") {
            anyhow::bail!("disk full");
        }
        self.inserted
            .lock()
            .expect("inserted")
            .extend_from_slice(rows);
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }

    async fn newest_on(&self, _: NaiveDate, _: u64) -> anyhow::Result<Vec<TranscriptSentence>> {
        Ok(Vec::new())
    }

    async fn max_sentence_id(&self) -> anyhow::Result<Option<u64>> {
        Ok(None)
    }

    async fn prune_before(&self, cutoff: NaiveDate) -> anyhow::Result<u64> {
        self.cutoffs.lock().expect("cutoffs").push(cutoff);
        Ok(0)
    }
}

/// Let the writer task run on the paused clock.
async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_full_store_channel_drops_and_counts_but_never_blocks_the_hub() {
    let capture = Capture::install();
    let hub = TranscriptHub::new();
    let (tx, mut rx) = mpsc::channel(2);
    hub.attach_store(StoreFeed::new(tx));
    // no writer reads: every push must still return at once
    let ids: Vec<u64> = (1..=5)
        .map(|n| hub.push_sentence("guest", closed(&format!("s{n}"))))
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    assert_eq!(hub.store_dropped(), 3, "rows 3, 4, 5 dropped and counted");
    assert_eq!(rx.try_recv().map(|s| s.id).ok(), Some(1));
    assert_eq!(rx.try_recv().map(|s| s.id).ok(), Some(2));
    assert!(rx.try_recv().is_err(), "nothing else was queued");
    let warns = capture.events_with_message("a transcript sentence was not stored");
    assert_eq!(
        warns.len(),
        2,
        "WARN on the 1st and 2nd drop of the streak only"
    );
    let why = quoted("the writer is behind");
    assert_eq!(warns[0].field("why"), Some(why.as_str()));

    let _six = hub.push_sentence("guest", closed("s6"));
    assert_eq!(
        rx.try_recv().map(|s| s.id).ok(),
        Some(6),
        "room again: stored"
    );
    assert_eq!(
        capture
            .events_with_message("the transcript store takes sentences again")
            .len(),
        1
    );
    assert_eq!(
        hub.snapshot().sentences.len(),
        6,
        "the hub kept every sentence"
    );
}

#[tokio::test]
async fn a_closed_writer_counts_every_offered_row() {
    let hub = TranscriptHub::new();
    let (tx, rx) = mpsc::channel(4);
    hub.attach_store(StoreFeed::new(tx));
    drop(rx);
    let _id = hub.push_sentence("guest", closed("lost"));
    assert_eq!(hub.store_dropped(), 1);
}

#[tokio::test]
async fn the_writer_stores_batches_under_the_local_date_and_ends_with_the_hub() {
    let (listen, clock) = service_at("2026-11-22T10:00:00");
    let repo = Repository::connect_in_memory().await.expect("repo");
    let (tx, rx) = mpsc::channel(STORE_CAPACITY);
    let writer = tokio::spawn(run_writer(Arc::new(repo.clone()), rx, listen.clone()));
    for id in 1..=3 {
        tx.send(sentence(id, "today")).await.expect("send");
    }
    let mut stored = Vec::new();
    for _ in 0..200 {
        stored = repo
            .newest_listen_transcript_on(day("2026-11-22"), 10)
            .await
            .expect("read");
        if stored.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        stored,
        vec![
            sentence(1, "today"),
            sentence(2, "today"),
            sentence(3, "today")
        ]
    );
    *clock.lock().expect("clock") = "2026-11-23T00:00:01".parse().expect("time");
    tx.send(sentence(4, "after midnight")).await.expect("send");
    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .expect("the writer ends when its sender is gone")
        .expect("no panic");
    let next_day = repo
        .newest_listen_transcript_on(day("2026-11-23"), 10)
        .await
        .expect("read");
    assert_eq!(next_day, vec![sentence(4, "after midnight")]);
}

#[tokio::test(start_paused = true)]
async fn retention_runs_at_start_and_once_per_local_date() {
    let (listen, clock) = service_at("2026-11-22T10:00:00");
    let rows = Arc::new(FakeRows::default());
    let (_tx, rx) = mpsc::channel(STORE_CAPACITY);
    let writer = tokio::spawn(run_writer(
        Arc::clone(&rows) as Arc<dyn TranscriptRows>,
        rx,
        listen,
    ));
    settle().await;
    assert_eq!(
        *rows.cutoffs.lock().expect("cutoffs"),
        vec![day("2026-11-15")],
        "at start: rows of a date more than 7 days back go"
    );
    tokio::time::advance(RETENTION_TICK * 3).await;
    settle().await;
    assert_eq!(
        rows.cutoffs.lock().expect("cutoffs").len(),
        1,
        "same date: once"
    );
    *clock.lock().expect("clock") = "2026-11-23T00:00:30".parse().expect("time");
    tokio::time::advance(RETENTION_TICK).await;
    settle().await;
    assert_eq!(
        *rows.cutoffs.lock().expect("cutoffs"),
        vec![day("2026-11-15"), day("2026-11-16")],
        "the first tick of the new date prunes again"
    );
    writer.abort();
}

#[tokio::test(start_paused = true)]
async fn a_failed_write_loses_the_batch_and_warns_on_power_of_two_failures() {
    let capture = Capture::install();
    let (listen, _clock) = service_at("2026-11-22T10:00:00");
    let rows = Arc::new(FakeRows::default());
    *rows.fail.lock().expect("fail") = true;
    let (tx, rx) = mpsc::channel(STORE_CAPACITY);
    let writer = tokio::spawn(run_writer(
        Arc::clone(&rows) as Arc<dyn TranscriptRows>,
        rx,
        listen,
    ));
    for id in 1..=3 {
        tx.send(sentence(id, "x")).await.expect("send");
        settle().await;
    }
    assert_eq!(
        capture
            .events_with_message("transcript sentences could not be stored")
            .len(),
        2,
        "failures 1 and 2 WARN, the 3rd is quiet"
    );
    *rows.fail.lock().expect("fail") = false;
    tx.send(sentence(4, "y")).await.expect("send");
    settle().await;
    assert_eq!(
        capture
            .events_with_message("the transcript store writes again")
            .len(),
        1
    );
    let inserted = rows.inserted.lock().expect("inserted").clone();
    assert_eq!(inserted.len(), 1, "the failed batches are not retried");
    assert_eq!(inserted[0].sentence.id, 4);
    writer.abort();
}

#[tokio::test]
async fn a_restart_seeds_todays_sentences_and_ids_continue_after_the_highest() {
    let (listen, _clock) = service_at("2026-11-22T10:00:00");
    let repo = Repository::connect_in_memory().await.expect("repo");
    let stored = |date: &str, id: u64, text: &str| NewTranscriptSentence {
        service_date: day(date),
        sentence: sentence(id, text),
    };
    repo.insert_listen_transcript(&[
        stored("2026-11-21", 50, "yesterday"),
        stored("2026-11-22", 7, "one"),
        stored("2026-11-22", 8, "two"),
        stored("2026-11-22", 9, "three"),
    ])
    .await
    .expect("insert");

    start(&listen, Arc::new(repo.clone())).await;

    let hub = listen.transcripts();
    let seeded: Vec<u64> = hub.snapshot().sentences.iter().map(|s| s.id).collect();
    assert_eq!(
        seeded,
        vec![7, 8, 9],
        "today's sentences only, oldest first"
    );
    let id = hub.push_sentence("guest", closed("four"));
    assert_eq!(id, 51, "after the highest stored id of any date");
    let mut today = Vec::new();
    for _ in 0..200 {
        today = repo
            .newest_listen_transcript_on(day("2026-11-22"), 10)
            .await
            .expect("read");
        if today.len() == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        today.last().map(|s| (s.id, s.text.as_str())),
        Some((51, "four"))
    );
}

#[tokio::test]
async fn a_seed_keeps_the_newest_kept_sentences_and_never_lowers_the_next_id() {
    let hub = TranscriptHub::new();
    let _first = hub.push_sentence("guest", closed("live"));
    let stored: Vec<_> = (100..100 + SENTENCES_KEPT as u64)
        .map(|id| sentence(id, "stored"))
        .collect();
    hub.seed(stored, 1);
    let sentences = hub.snapshot().sentences;
    assert_eq!(sentences.len(), SENTENCES_KEPT);
    assert_eq!(
        sentences.first().map(|s| s.id),
        Some(101),
        "the oldest stored one went"
    );
    assert_eq!(sentences.last().map(|s| s.text.as_str()), Some("live"));
    assert_eq!(hub.push_sentence("guest", closed("next")), 2);
}
