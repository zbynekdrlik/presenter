//! The guest transcript's 7-day store (#852 C3-persist, spec §12, §22.14).
//! The hub hands every closed sentence to ONE writer task over a bounded
//! channel ([`StoreFeed::offer`], `try_send`): a full channel drops the row,
//! counted, with a #484-gated WARN per streak — the hub (and so the audio
//! path) never waits on the database. The writer stamps each batch with the
//! local service date ([`ListenService::today`], the recorder's clock) and
//! inserts it in one statement; a failed write loses that batch (counted,
//! gated WARN), never retried in a loop that would back the channel up.
//! The same task deletes rows older than [`TRANSCRIPT_RETENTION_DAYS`] once per
//! local date (at start, then the first tick after each midnight — the
//! `state/interpreter_tips` shape). At startup [`start`] seeds the hub with
//! today's newest [`SENTENCES_KEPT`] rows and makes its ids continue after the
//! highest stored one (any date), so a restart mid-sermon keeps the
//! interpreter's EN column and no id is ever reused. Started from
//! `AppState::start_listen` (never in validate mode), before the supervisor.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use presenter_core::listen::ws::TranscriptSentence;
use presenter_persistence::{NewTranscriptSentence, Repository};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::time::{interval, MissedTickBehavior};

use super::transcript_hub::SENTENCES_KEPT;
use super::{should_log_event, ListenService};

/// The target of every store line.
pub(crate) const STORE_TARGET: &str = "presenter::listen::transcript";
/// Sentences the writer may fall behind before rows are dropped (a sentence
/// every few seconds: minutes of slack).
pub(crate) const STORE_CAPACITY: usize = 256;
/// Days a sentence is kept: its service date more than this many days before
/// today is deleted (spec §12).
pub(crate) const TRANSCRIPT_RETENTION_DAYS: i64 = 7;
/// At most this many sentences per insert.
const WRITE_BATCH: usize = 64;
/// How often the writer checks whether the local date changed.
const RETENTION_TICK: Duration = Duration::from_secs(60);

/// What the store reads and writes (the `Repository`; tests: a fake).
// `#[async_trait]` marks the methods `#[must_use]` (clippy::double_must_use).
#[allow(clippy::double_must_use)]
#[async_trait]
pub(crate) trait TranscriptRows: Send + Sync {
    async fn insert(&self, rows: &[NewTranscriptSentence]) -> anyhow::Result<u64>;
    async fn newest_on(
        &self,
        day: NaiveDate,
        limit: u64,
    ) -> anyhow::Result<Vec<TranscriptSentence>>;
    async fn max_sentence_id(&self) -> anyhow::Result<Option<u64>>;
    async fn prune_before(&self, cutoff: NaiveDate) -> anyhow::Result<u64>;
}

#[async_trait]
impl TranscriptRows for Repository {
    async fn insert(&self, rows: &[NewTranscriptSentence]) -> anyhow::Result<u64> {
        self.insert_listen_transcript(rows).await
    }

    async fn newest_on(
        &self,
        day: NaiveDate,
        limit: u64,
    ) -> anyhow::Result<Vec<TranscriptSentence>> {
        self.newest_listen_transcript_on(day, limit).await
    }

    async fn max_sentence_id(&self) -> anyhow::Result<Option<u64>> {
        self.max_listen_transcript_sentence_id().await
    }

    async fn prune_before(&self, cutoff: NaiveDate) -> anyhow::Result<u64> {
        self.prune_listen_transcript_before(cutoff).await
    }
}

/// The hub's end of the writer's channel; lives in the hub's state, called
/// under its lock (never waits).
pub(crate) struct StoreFeed {
    tx: mpsc::Sender<TranscriptSentence>,
    /// Rows dropped in a row (a full or closed channel); reset by a send.
    streak: u64,
    /// Rows dropped since the start.
    dropped: u64,
}

impl StoreFeed {
    pub(crate) fn new(tx: mpsc::Sender<TranscriptSentence>) -> Self {
        Self {
            tx,
            streak: 0,
            dropped: 0,
        }
    }

    /// Hand a sentence to the writer, or drop and count it.
    pub(crate) fn offer(&mut self, sentence: &TranscriptSentence) {
        let why = match self.tx.try_send(sentence.clone()) {
            Ok(()) => {
                if self.streak > 0 {
                    tracing::info!(
                        target: STORE_TARGET,
                        dropped = self.streak,
                        "the transcript store takes sentences again"
                    );
                    self.streak = 0;
                }
                return;
            }
            Err(TrySendError::Full(_)) => "the writer is behind",
            Err(TrySendError::Closed(_)) => "the writer ended",
        };
        self.streak = self.streak.saturating_add(1);
        self.dropped = self.dropped.saturating_add(1);
        if should_log_event(self.streak) {
            tracing::warn!(
                target: STORE_TARGET,
                why,
                streak = self.streak,
                dropped_total = self.dropped,
                sentence = sentence.id,
                "a transcript sentence was not stored"
            );
        }
    }

    /// Rows dropped since the start.
    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Seed the hub from the store, attach the writer's channel and spawn the
/// writer (once, from `start_listen`, before the transcription supervisor).
pub(crate) async fn start(listen: &ListenService, store: Arc<dyn TranscriptRows>) {
    let hub = listen.transcripts();
    let today = listen.today();
    let (stored, next_id) = read_seed(store.as_ref(), today).await;
    tracing::info!(
        target: STORE_TARGET,
        sentences = stored.len(),
        next_id,
        %today,
        "transcript seeded from the store"
    );
    hub.seed(stored, next_id);
    let (tx, rx) = mpsc::channel(STORE_CAPACITY);
    hub.attach_store(StoreFeed::new(tx));
    drop(tokio::spawn(run_writer(store, rx, listen.clone())));
}

/// Today's newest sentences and the next id. A failed read WARNs and seeds
/// nothing (ids then start at 1: a stored id may repeat, nothing breaks).
async fn read_seed(store: &dyn TranscriptRows, today: NaiveDate) -> (Vec<TranscriptSentence>, u64) {
    let next_id = match store.max_sentence_id().await {
        Ok(max) => max.map_or(1, |max| max.saturating_add(1)),
        Err(err) => {
            tracing::warn!(
                target: STORE_TARGET,
                error = %err,
                "the highest stored sentence id cannot be read — ids start at 1"
            );
            1
        }
    };
    let limit = u64::try_from(SENTENCES_KEPT).unwrap_or(u64::MAX);
    let stored = match store.newest_on(today, limit).await {
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(
                target: STORE_TARGET,
                error = %err,
                "today's stored transcript cannot be read — the transcript starts empty"
            );
            Vec::new()
        }
    };
    (stored, next_id)
}

/// The writer: batches of received sentences, and retention once per date.
/// Ends when the hub (the only sender) is gone.
pub(crate) async fn run_writer(
    store: Arc<dyn TranscriptRows>,
    mut rx: mpsc::Receiver<TranscriptSentence>,
    listen: ListenService,
) {
    let mut ticker = interval(RETENTION_TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_prune: Option<NaiveDate> = None;
    let mut failures: u64 = 0;
    loop {
        tokio::select! {
            biased;
            received = rx.recv() => {
                let Some(first) = received else {
                    tracing::info!(target: STORE_TARGET, "transcript writer ended");
                    return;
                };
                let batch = collect_batch(first, &mut rx);
                write_batch(store.as_ref(), batch, listen.today(), &mut failures).await;
            }
            _ = ticker.tick() => {
                prune_once_per_date(store.as_ref(), listen.today(), &mut last_prune).await;
            }
        }
    }
}

/// `first` and whatever else is already waiting, up to [`WRITE_BATCH`].
fn collect_batch(
    first: TranscriptSentence,
    rx: &mut mpsc::Receiver<TranscriptSentence>,
) -> Vec<TranscriptSentence> {
    let mut batch = vec![first];
    while batch.len() < WRITE_BATCH {
        match rx.try_recv() {
            Ok(sentence) => batch.push(sentence),
            Err(_) => break,
        }
    }
    batch
}

/// Insert one batch under `today`; a failure loses the batch (WARN on
/// power-of-two failures in a row, INFO when writes work again).
async fn write_batch(
    store: &dyn TranscriptRows,
    batch: Vec<TranscriptSentence>,
    today: NaiveDate,
    failures: &mut u64,
) {
    let rows: Vec<NewTranscriptSentence> = batch
        .into_iter()
        .map(|sentence| NewTranscriptSentence {
            service_date: today,
            sentence,
        })
        .collect();
    match store.insert(&rows).await {
        Ok(_) => {
            if *failures > 0 {
                tracing::info!(
                    target: STORE_TARGET,
                    failures = *failures,
                    "the transcript store writes again"
                );
                *failures = 0;
            }
        }
        Err(err) => {
            *failures = failures.saturating_add(1);
            if should_log_event(*failures) {
                tracing::warn!(
                    target: STORE_TARGET,
                    error = %err,
                    lost = rows.len(),
                    failures = *failures,
                    "transcript sentences could not be stored"
                );
            }
        }
    }
}

/// Delete rows older than the retention once per local date: one attempt,
/// failed or not (a broken DB must not WARN every minute).
async fn prune_once_per_date(
    store: &dyn TranscriptRows,
    today: NaiveDate,
    last_prune: &mut Option<NaiveDate>,
) {
    if *last_prune == Some(today) {
        return;
    }
    *last_prune = Some(today);
    let Some(cutoff) = today.checked_sub_signed(chrono::Duration::days(TRANSCRIPT_RETENTION_DAYS))
    else {
        return;
    };
    match store.prune_before(cutoff).await {
        Ok(0) => {}
        Ok(pruned) => tracing::info!(
            target: STORE_TARGET,
            pruned,
            retention_days = TRANSCRIPT_RETENTION_DAYS,
            "old transcript sentences deleted"
        ),
        Err(err) => tracing::warn!(
            target: STORE_TARGET,
            error = %err,
            "transcript retention failed — next try tomorrow"
        ),
    }
}

#[cfg(test)]
mod tests;
