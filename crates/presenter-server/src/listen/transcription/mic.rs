//! One running guest session (#852 C3-7, plan Task 7.3): the mic's frames go
//! to the provider with their level through `try_send` (a full input counts
//! a drop, a lagging frame channel counts the lost frames — the chain never
//! waits for the transcript); the provider's events become hub sentences and
//! a tail (core `TranscriptAssembler`), the guest's state and the meter. A
//! stop sends `AsrInput::Stop` and drops the input sender (the provider
//! finalizes and ends); the task returns the provider's `Finished` reason.

use chrono::Utc;
use presenter_core::audio::level::rms_dbfs;
use presenter_core::listen::transcript::{AssemblyOutput, TranscriptAssembler};
use presenter_core::listen::transcription::{OffReason, TranscriptionState};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use super::ASR_TARGET;
use crate::listen::asr::{AsrEvent, AsrInput, AsrSession};
use crate::listen::{should_log_event, ListenService, MicFrame};

pub(super) struct MicTask {
    pub(super) mic_id: String,
    stop: Option<oneshot::Sender<()>>,
    pub(super) handle: JoinHandle<Option<OffReason>>,
}

impl MicTask {
    /// Ask the session to end (its last words still arrive).
    pub(super) fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

pub(super) fn spawn(
    mic_id: String,
    frames: broadcast::Receiver<MicFrame>,
    session: AsrSession,
    listen: ListenService,
) -> MicTask {
    let (stop, stopped) = oneshot::channel();
    let task = MicRun {
        mic_id: mic_id.clone(),
        listen,
        assembler: TranscriptAssembler::new(),
        totals: (0, 0),
        newest_connection: 0,
        last_state: TranscriptionState::Armed,
        dropped: 0,
        lag_events: 0,
        lost: 0,
    };
    let handle = tokio::spawn(task.run(frames, session, stopped));
    MicTask {
        mic_id,
        stop: Some(stop),
        handle,
    }
}

struct MicRun {
    mic_id: String,
    listen: ListenService,
    assembler: TranscriptAssembler,
    /// The session's last reported `(audio_ms, connected_ms)` totals.
    totals: (u64, u64),
    newest_connection: u32,
    /// The provider's last state: an outage is counted once, not per retry.
    last_state: TranscriptionState,
    dropped: u64,
    lag_events: u64,
    /// Frames lost because this task fell behind the mic.
    lost: u64,
}

impl MicRun {
    async fn run(
        mut self,
        mut frames: broadcast::Receiver<MicFrame>,
        session: AsrSession,
        mut stopped: oneshot::Receiver<()>,
    ) -> Option<OffReason> {
        let AsrSession { input, mut events } = session;
        let mut input = Some(input);
        let mut stop_open = true;
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped, if stop_open => {
                    stop_open = false;
                    // The provider finalizes, then ends with `Finished`. A
                    // full input cannot take the `Stop`: the sender is dropped
                    // either way, and a closed input is the same stop.
                    if let Some(input) = input.take() {
                        let _ = input.try_send(AsrInput::Stop);
                    }
                }
                event = events.recv() => match event {
                    Some(AsrEvent::Finished(reason)) => return self.finish(reason),
                    // The provider task is gone without a word.
                    None => return self.finish(None),
                    Some(event) => self.on_event(event),
                },
                frame = frames.recv(), if input.is_some() => match frame {
                    Ok(pcm) => self.forward(input.as_ref(), pcm),
                    Err(RecvError::Lagged(lost)) => self.lagged(lost),
                    // Frame channels live as long as the process.
                    Err(RecvError::Closed) => input = None,
                },
            }
        }
    }

    fn forward(&mut self, input: Option<&mpsc::Sender<AsrInput>>, pcm: MicFrame) {
        let Some(input) = input else { return };
        let dbfs = rms_dbfs(&pcm);
        if let Err(TrySendError::Full(_)) = input.try_send(AsrInput::Frame { pcm, dbfs }) {
            self.dropped += 1;
            if should_log_event(self.dropped) {
                tracing::warn!(target: ASR_TARGET, mic = self.mic_id.as_str(), dropped = self.dropped, "the transcription session is behind — frames dropped");
            }
        }
    }

    fn lagged(&mut self, lost: u64) {
        self.lag_events += 1;
        self.lost = self.lost.saturating_add(lost);
        if should_log_event(self.lag_events) {
            tracing::warn!(target: ASR_TARGET, mic = self.mic_id.as_str(), lost, lost_total = self.lost, events = self.lag_events, "the transcription task fell behind the mic — frames lost");
        }
    }

    fn on_event(&mut self, event: AsrEvent) {
        let transcription = self.listen.transcription();
        match event {
            AsrEvent::Tokens(batch) => {
                self.note_connection(batch.connection);
                let out = self.assembler.apply(batch);
                self.publish(out);
            }
            AsrEvent::ConnectionEnded(connection) => {
                self.note_connection(connection);
                let out = self.assembler.connection_ended(connection);
                self.publish(out);
            }
            AsrEvent::State { state, error } => {
                let mic = self.mic_id.as_str();
                let outage = state == TranscriptionState::Reconnecting
                    && self.last_state != TranscriptionState::Reconnecting;
                self.last_state = state;
                transcription.with_view(|view| {
                    if outage {
                        view.meter.add_reconnect(mic);
                    }
                    if let Some(guest) = view.guest.as_mut().filter(|g| g.mic_id == mic) {
                        guest.state = state;
                        guest.error = error;
                    }
                });
                transcription.publish_guest(&self.listen.transcripts());
            }
            AsrEvent::Usage {
                audio_ms,
                connected_ms,
            } => {
                let (audio, connected) = self.totals;
                self.totals = (audio_ms, connected_ms);
                // Rolled here too: time metered after midnight is the new day's.
                let today = self.listen.today();
                transcription.with_view(|view| {
                    view.meter.roll(today);
                    view.meter.add_usage(
                        &self.mic_id,
                        audio_ms.saturating_sub(audio),
                        connected_ms.saturating_sub(connected),
                    );
                });
            }
            AsrEvent::Onset { latency_ms } => {
                transcription.with_view(|view| view.meter.add_onset(&self.mic_id, latency_ms));
            }
            AsrEvent::Finished(_) => {}
        }
    }

    /// Connections are numbered from 1 per session: a new number = one more.
    fn note_connection(&mut self, connection: u32) {
        if connection > self.newest_connection {
            let added = connection - self.newest_connection;
            self.newest_connection = connection;
            let mic = self.mic_id.as_str();
            self.listen
                .transcription()
                .with_view(|view| view.meter.add_connections(mic, added));
        }
    }

    fn publish(&self, out: AssemblyOutput) {
        let hub = self.listen.transcripts();
        for sentence in out.closed {
            let _id = hub.push_sentence(&self.mic_id, sentence);
        }
        if let Some(tail) = out.tail {
            hub.set_tail(&self.mic_id, Some(tail), Utc::now());
        }
    }

    /// The session ended: the open sentence closes, the tail empties.
    fn finish(&mut self, reason: Option<OffReason>) -> Option<OffReason> {
        let out = self.assembler.flush(Utc::now());
        self.publish(out);
        if self.dropped > 0 || self.lost > 0 {
            tracing::info!(target: ASR_TARGET, mic = self.mic_id.as_str(), dropped = self.dropped, lost = self.lost, lag_events = self.lag_events, "guest transcription session lost frames");
        }
        reason
    }
}
