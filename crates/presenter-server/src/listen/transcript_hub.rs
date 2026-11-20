//! The guest's transcript as `GET /listen/ws` serves it (#852 C3-6, plan
//! Decision 8): the newest [`SENTENCES_KEPT`] closed sentences, every mic's
//! open tail and every mic's session state, plus ONE `broadcast` of
//! [`ListenWsMessage`]s. Every message is sent while the hub's lock is held and
//! [`TranscriptHub::subscribe`] takes the snapshot and the receiver under the
//! same lock, so a reader never misses or doubles a message between the two.
//! A reader [`HUB_CAPACITY`] messages behind gets `Lagged` and subscribes
//! again (a fresh snapshot). Tails are coalesced to one message per
//! [`TAIL_MIN_INTERVAL`] per mic; the hub sends the last one itself when the
//! window ends. `session` messages go out only when state / reason / error
//! change. Owned by `ListenService`; no `LiveEvent` (the stage and operator
//! sockets never see transcript traffic). The lock is a std mutex, never held
//! across an `.await`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use presenter_core::listen::transcript::{ClosedSentence, TailText};
use presenter_core::listen::transcription::MicTranscription;
use presenter_core::listen::ws::{
    ListenWsMessage, TranscriptSentence, TranscriptSnapshot, TranscriptTail,
};
use tokio::sync::broadcast;
use tokio::time::Instant;

/// Closed sentences a snapshot carries (the newest; older ones are dropped).
pub(crate) const SENTENCES_KEPT: usize = 500;
/// Messages a reader may fall behind before it is resynced with a snapshot.
pub(crate) const HUB_CAPACITY: usize = 512;
/// At most one `tail` message per mic per this interval (≤ 10 per second).
pub(crate) const TAIL_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// The transcript every `/listen/ws` reader shares; a clone is the same hub.
#[derive(Clone)]
pub(crate) struct TranscriptHub {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<HubState>,
    tx: broadcast::Sender<ListenWsMessage>,
}

#[derive(Default)]
struct HubState {
    /// The id of the next sentence: increasing, never reused.
    next_id: u64,
    sentences: VecDeque<TranscriptSentence>,
    /// Each mic's latest tail, empty ones included (the snapshot skips them).
    tails: BTreeMap<String, TranscriptTail>,
    gates: BTreeMap<String, TailGate>,
    sessions: BTreeMap<String, MicTranscription>,
}

/// One mic's tail coalescing.
#[derive(Default)]
struct TailGate {
    last_sent: Option<Instant>,
    /// The stored tail has not been sent yet.
    pending: bool,
    /// A flush task waits for the end of the window.
    flush_scheduled: bool,
    /// A sentence of this mic was just sent: its next tail goes out at once,
    /// so a reader never shows the closed text twice.
    after_sentence: bool,
}

impl TranscriptHub {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(HubState {
                    next_id: 1,
                    ..HubState::default()
                }),
                tx: broadcast::Sender::new(HUB_CAPACITY),
            }),
        }
    }

    // #852 C3-6: the publishers are C3-7's session task — C3-7 removes this.
    #[allow(dead_code)]
    /// Keep and send a closed sentence; its id.
    pub(crate) fn push_sentence(&self, mic_id: &str, sentence: ClosedSentence) -> u64 {
        let mut state = self.shared.lock();
        let id = state.next_id;
        state.next_id += 1;
        let sentence = TranscriptSentence {
            id,
            mic_id: mic_id.to_string(),
            text: sentence.text,
            start_ms: sentence.start_ms,
            end_ms: sentence.end_ms,
            closed_at: sentence.closed_at,
            language: sentence.language,
        };
        if state.sentences.len() == SENTENCES_KEPT {
            let _oldest = state.sentences.pop_front();
        }
        state.sentences.push_back(sentence.clone());
        // The stored tail held the words just closed: clear it, so neither a
        // snapshot nor a pending flush shows them twice. The publisher's next
        // `set_tail` goes out at once.
        if let Some(tail) = state.tails.get_mut(mic_id) {
            tail.final_text.clear();
            tail.partial_text.clear();
        }
        let gate = state.gates.entry(mic_id.to_string()).or_default();
        gate.pending = false;
        gate.after_sentence = true;
        self.shared.send(ListenWsMessage::Sentence { sentence });
        id
    }

    // #852 C3-6: the publishers are C3-7's session task — C3-7 removes this.
    #[allow(dead_code)]
    /// Set a mic's open tail (`None` or no text = nothing open: readers get an
    /// empty tail and the snapshot leaves it out). Coalesced per mic.
    pub(crate) fn set_tail(&self, mic_id: &str, tail: Option<TailText>, at: DateTime<Utc>) {
        let tail = tail.unwrap_or_default();
        let now = Instant::now();
        let mut state = self.shared.lock();
        let _previous = state.tails.insert(
            mic_id.to_string(),
            TranscriptTail {
                mic_id: mic_id.to_string(),
                final_text: tail.final_text,
                partial_text: tail.partial_text,
                updated_at: at,
            },
        );
        let gate = state.gates.entry(mic_id.to_string()).or_default();
        let due = gate
            .last_sent
            .is_none_or(|last| now >= last + TAIL_MIN_INTERVAL);
        if due || gate.after_sentence {
            send_tail(&self.shared, &mut state, mic_id, now);
            return;
        }
        gate.pending = true;
        if gate.flush_scheduled {
            return;
        }
        gate.flush_scheduled = true;
        let deadline = gate.last_sent.map_or(now, |last| last + TAIL_MIN_INTERVAL);
        drop(state);
        self.schedule_flush(mic_id, deadline);
    }

    // #852 C3-6: the publishers are C3-7's session task — C3-7 removes this.
    #[allow(dead_code)]
    /// Store a mic's session state; send it only when state / reason / error
    /// changed (the counters change every second and ride in the snapshot).
    pub(crate) fn set_session(&self, session: MicTranscription) {
        let mut state = self.shared.lock();
        let visible = state.sessions.get(&session.mic_id).is_none_or(|stored| {
            (stored.state, stored.reason, &stored.error)
                != (session.state, session.reason, &session.error)
        });
        let _previous = state
            .sessions
            .insert(session.mic_id.clone(), session.clone());
        if visible {
            self.shared.send(ListenWsMessage::Session { session });
        }
    }

    /// The snapshot and a receiver of every message after it, under one lock.
    pub(crate) fn subscribe(&self) -> (TranscriptSnapshot, broadcast::Receiver<ListenWsMessage>) {
        let state = self.shared.lock();
        (snapshot_of(&state), self.shared.tx.subscribe())
    }

    // #852 C3-6: the publishers are C3-7's session task — C3-7 removes this.
    #[allow(dead_code)]
    /// What a reader connecting now would get first.
    pub(crate) fn snapshot(&self) -> TranscriptSnapshot {
        snapshot_of(&self.shared.lock())
    }

    /// Send the mic's stored tail at `deadline` if it is still unsent. Outside
    /// a runtime (never in the server) it is sent at once instead.
    fn schedule_flush(&self, mic_id: &str, deadline: Instant) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let mut state = self.shared.lock();
            state
                .gates
                .entry(mic_id.to_string())
                .or_default()
                .flush_scheduled = false;
            send_tail(&self.shared, &mut state, mic_id, Instant::now());
            return;
        };
        let shared = Arc::clone(&self.shared);
        let mic_id = mic_id.to_string();
        drop(runtime.spawn(async move {
            let mut deadline = deadline;
            loop {
                tokio::time::sleep_until(deadline).await;
                match flush_due(&shared, &mic_id) {
                    Some(later) => deadline = later,
                    None => break,
                }
            }
        }));
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Send under the caller's lock; no reader is not an error.
    fn send(&self, message: ListenWsMessage) {
        let _readers = self.tx.send(message);
    }
}

/// The flush task woke: send the pending tail, unless a forced send after a
/// sentence restarted the window meanwhile — then the time to wait for.
fn flush_due(shared: &Shared, mic_id: &str) -> Option<Instant> {
    let now = Instant::now();
    let mut state = shared.lock();
    let gate = state.gates.entry(mic_id.to_string()).or_default();
    let window_end = gate.last_sent.map(|last| last + TAIL_MIN_INTERVAL);
    if let Some(later) = window_end.filter(|end| gate.pending && now < *end) {
        return Some(later);
    }
    gate.flush_scheduled = false;
    if gate.pending {
        send_tail(shared, &mut state, mic_id, now);
    }
    None
}

/// Send the mic's stored tail now and restart its window.
fn send_tail(shared: &Shared, state: &mut HubState, mic_id: &str, now: Instant) {
    let Some(tail) = state.tails.get(mic_id).cloned() else {
        return;
    };
    let gate = state.gates.entry(mic_id.to_string()).or_default();
    gate.last_sent = Some(now);
    gate.pending = false;
    gate.after_sentence = false;
    shared.send(ListenWsMessage::Tail { tail });
}

fn snapshot_of(state: &HubState) -> TranscriptSnapshot {
    TranscriptSnapshot {
        sentences: state.sentences.iter().cloned().collect(),
        tails: state
            .tails
            .values()
            .filter(|tail| !tail.final_text.is_empty() || !tail.partial_text.is_empty())
            .cloned()
            .collect(),
        sessions: state.sessions.values().cloned().collect(),
    }
}

#[cfg(test)]
mod tests;
