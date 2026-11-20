//! The streaming speech-recognition seam of the guest transcription (#852
//! C3-5, plan Decision 4): [`StreamingAsr::open`] starts one session in its
//! own task and hands back two channels — the mic frames in, the session's
//! events out. Channel-based because the session must read frames and the
//! provider's answers at the same time (spec §6.1's `&mut self` pair cannot
//! be awaited concurrently by one task). Soniox (`soniox::session`) is the
//! one provider; the supervisor (C3-7) tests against a fake.

use presenter_core::listen::context::ContextTerms;
use presenter_core::listen::transcript::TokenBatch;
use presenter_core::listen::transcription::{OffReason, TranscriptionState};
use tokio::sync::mpsc;

use super::MicFrame;

/// Frames a session buffers (6.4 s of audio). The mic task uses `try_send`
/// and counts a full channel as a drop — it never waits.
pub const INPUT_CAPACITY: usize = 64;
/// Events a session buffers before it waits for its reader.
pub const EVENT_CAPACITY: usize = 256;

/// What one session needs from its owner.
#[derive(Debug, Clone)]
pub struct AsrSessionConfig {
    pub mic_id: String,
    /// The provider's vocabulary hints (core `listen::context`).
    pub context: ContextTerms,
}

#[derive(Debug, Clone)]
pub enum AsrInput {
    /// One 100 ms frame and its level (dBFS).
    Frame { pcm: MicFrame, dbfs: f32 },
    /// End the session: the last words are finalized, then `Finished`.
    Stop,
}

/// What a session reports. Dropping the input sender also stops it.
#[derive(Debug, Clone, PartialEq)]
pub enum AsrEvent {
    Tokens(TokenBatch),
    /// The connection ended: its live (non-final) tail is void.
    ConnectionEnded(u32),
    State {
        state: TranscriptionState,
        error: Option<String>,
    },
    /// The session's running totals (not deltas).
    Usage {
        audio_ms: u64,
        connected_ms: u64,
    },
    /// First voiced frame sent → its first word back.
    Onset {
        latency_ms: u32,
    },
    /// The last event; `Some` = a stop a person must fix.
    Finished(Option<OffReason>),
}

pub struct AsrSession {
    pub input: mpsc::Sender<AsrInput>,
    pub events: mpsc::Receiver<AsrEvent>,
}

pub trait StreamingAsr: Send + Sync {
    /// Starts one session (needs a tokio runtime).
    fn open(&self, cfg: AsrSessionConfig) -> AsrSession;
}
