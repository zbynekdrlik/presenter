//! The audio bookkeeping: pre-roll, the connect queue, the replay ring and
//! the session ms of every sent frame (#852 C3-4).

use super::*;

impl SonioxCore {
    /// Sends a frame that never went out: it gets the next session ms.
    pub(super) fn send_new(&mut self, pcm: MicFrame, out: &mut Vec<Action>) {
        let ms = frame_ms(&pcm);
        self.replay.push_back(Sent {
            start_ms: self.sent_end_ms,
            pcm: pcm.clone(),
        });
        self.sent_end_ms += ms;
        self.trim_replay();
        self.send_audio(pcm, ms, out);
    }

    pub(super) fn send_audio(&mut self, pcm: MicFrame, ms: u64, out: &mut Vec<Action>) {
        let Some(live) = self.live_mut() else {
            return;
        };
        live.audio_since_response_ms += ms;
        let conn = live.id;
        self.stats.audio_ms += ms;
        self.unfinalized = true;
        out.push(Action::SendAudio { conn, pcm });
    }

    pub(super) fn send_finalize(&mut self, out: &mut Vec<Action>) {
        let Some(conn) = self.active_id() else {
            return;
        };
        out.push(Action::SendControl {
            conn,
            control: Control::Finalize,
        });
        self.finalize = Some(self.sent_end_ms);
        self.unfinalized = false;
    }

    pub(super) fn confirm(&mut self, ms: u64) {
        self.confirmed_ms = self.confirmed_ms.max(ms);
        self.trim_replay();
    }

    pub(super) fn trim_replay(&mut self) {
        let keep_from = self
            .confirmed_ms
            .max(self.sent_end_ms.saturating_sub(REPLAY_MAX_MS));
        while self.replay.front().is_some_and(|s| s.end_ms() <= keep_from) {
            self.replay.pop_front();
        }
    }

    /// Where the next connection's audio starts: the last confirmed ms,
    /// at most 5 s back.
    pub(super) fn resend_start(&self) -> u64 {
        let ring_start = self.replay.front().map_or(self.sent_end_ms, |s| s.start_ms);
        self.confirmed_ms.max(ring_start).min(self.sent_end_ms)
    }

    /// The sent audio from `from` on; the first frame cut at `from`.
    pub(super) fn resend_frames(&self, from: u64) -> Vec<MicFrame> {
        self.replay
            .iter()
            .filter(|s| s.end_ms() > from)
            .filter_map(|s| match from.checked_sub(s.start_ms) {
                Some(skip) if skip > 0 => {
                    let samples = usize::try_from(skip).ok()? * SAMPLES_PER_MS;
                    s.pcm.get(samples..).map(MicFrame::from)
                }
                _ => Some(s.pcm.clone()),
            })
            .collect()
    }

    pub(super) fn queue(&mut self, pcm: MicFrame) {
        self.pending_ms += frame_ms(&pcm);
        self.pending.push_back(pcm);
        while self.pending_ms > QUEUE_MAX_MS {
            let Some(old) = self.pending.pop_front() else {
                break;
            };
            self.pending_ms = self.pending_ms.saturating_sub(frame_ms(&old));
            self.stats.dropped_frames += 1;
        }
    }

    pub(super) fn push_preroll(&mut self, pcm: MicFrame) {
        self.preroll.push_back(pcm);
        while self.preroll.len() > PRE_ROLL_FRAMES {
            self.preroll.pop_front();
        }
    }
}
