//! The connection reports: connected, responses, closes, failed connects
//! and what each error class does (#852 C3-4).

use presenter_core::listen::transcript::AsrToken;

use super::super::wire::{classify, classify_close, to_tokens, ErrorClass, SonioxError};
use super::*;

impl SonioxCore {
    pub(super) fn on_connected(&mut self, conn: u32, at: Instant, out: &mut Vec<Action>) {
        let (offset_ms, kind) = match self.phase {
            Phase::Connecting {
                id,
                offset_ms,
                kind,
            } if id == conn => (offset_ms, kind),
            _ => return,
        };
        self.phase = Phase::Live(Live {
            id: conn,
            offset_ms,
            connected_at: at,
            last_response_at: at,
            audio_since_response_ms: 0,
            gate: None,
        });
        for frame in self.resend_frames(offset_ms) {
            let ms = frame_ms(&frame);
            self.send_audio(frame, ms, out);
        }
        let onset_ms = self.sent_end_ms;
        let pending: Vec<MicFrame> = self.pending.drain(..).collect();
        self.pending_ms = 0;
        for frame in pending {
            self.send_new(frame, out);
        }
        self.next_usage_at.get_or_insert(at + USAGE_EVERY);
        if self.speaking(at) {
            if kind == ConnectKind::Lazy {
                self.onset = Some((at, onset_ms));
            }
            self.set_state(TranscriptionState::Streaming, None, out);
        } else {
            self.gate(at, out);
        }
    }

    pub(super) fn on_response(
        &mut self,
        conn: u32,
        resp: SonioxResponse,
        at: Instant,
        received_at: DateTime<Utc>,
        out: &mut Vec<Action>,
    ) {
        if self.drained(conn, resp.finished, out) {
            return;
        }
        let Some(live) = self.live_mut().filter(|live| live.id == conn) else {
            return;
        };
        live.last_response_at = at;
        live.audio_since_response_ms = 0;
        let offset_ms = live.offset_ms;
        if let Some(error) = &resp.error {
            self.on_error(classify(error), describe(error), at, out);
            return;
        }
        self.backoff_step = 0;
        let tokens = to_tokens(&resp);
        let fin = self.take_tokens(conn, offset_ms, tokens, at, received_at, out);
        if fin {
            if let Some(covered) = self.finalizes.pop_front() {
                self.confirm(covered);
            }
        }
        if resp.finished {
            self.on_finished(at, out);
        } else if fin && matches!(self.phase, Phase::Rolling { .. }) {
            self.switch_rollover(at, true, out);
        }
    }

    /// Emits the batch; returns whether it holds `<fin>`.
    pub(super) fn take_tokens(
        &mut self,
        conn: u32,
        offset_ms: u64,
        tokens: Vec<AsrToken>,
        at: Instant,
        received_at: DateTime<Utc>,
        out: &mut Vec<Action>,
    ) -> bool {
        let mut fin = false;
        let mut onset_heard = false;
        for token in &tokens {
            if token.text == FIN_MARKER {
                fin = true;
            }
            if token.text == FIN_MARKER || token.text == END_MARKER || token.is_translation {
                continue;
            }
            // Only a word of the onset's own audio answers it (not the
            // late finals of speech before a gate, not a resend).
            if let (Some((_, onset_ms)), Some(start)) = (self.onset, token.start_ms) {
                onset_heard |= offset_ms.saturating_add(start) >= onset_ms;
            }
            if token.is_final {
                self.stats.tokens_final += 1;
                if let Some(end) = token.end_ms {
                    self.confirm(offset_ms.saturating_add(end));
                }
            }
        }
        if let Some((sent_at, _)) = self.onset.filter(|_| onset_heard) {
            self.onset = None;
            out.push(Action::Onset {
                latency_ms: ms_u32(at.saturating_duration_since(sent_at)),
            });
        }
        if !tokens.is_empty() {
            out.push(Action::Emit(TokenBatch {
                connection: conn,
                offset_ms,
                tokens,
                received_at,
            }));
        }
        fin
    }

    /// `finished: true` on the active connection.
    pub(super) fn on_finished(&mut self, at: Instant, out: &mut Vec<Action>) {
        match self.phase {
            Phase::Stopping { .. } => self.finish(None, None, at, out),
            Phase::Rolling { .. } => self.switch_rollover(at, false, out),
            _ => self.lose(at, Retry::Now, None, out),
        }
    }

    pub(super) fn on_closed(
        &mut self,
        conn: u32,
        code: Option<u16>,
        reason: &str,
        at: Instant,
        out: &mut Vec<Action>,
    ) {
        if self.drained(conn, true, out) || self.active_id() != Some(conn) {
            return;
        }
        if matches!(self.phase, Phase::Stopping { .. }) {
            self.finish(None, None, at, out);
            return;
        }
        let class = code.map_or(ErrorClass::Unknown, |code| classify_close(code, reason));
        let error = match code {
            Some(code) => format!("Soniox zatvoril spojenie ({code}) \"{reason}\""),
            None => format!("spojenie so Soniox sa prerušilo \"{reason}\""),
        };
        self.on_error(class, error, at, out);
    }

    pub(super) fn on_connect_failed(
        &mut self,
        conn: u32,
        status: Option<u16>,
        error: &str,
        at: Instant,
        out: &mut Vec<Action>,
    ) {
        if !matches!(self.phase, Phase::Connecting { id, .. } if id == conn) {
            return;
        }
        let class = status.map_or(ErrorClass::Unknown, |code| {
            classify(&SonioxError {
                code: Some(code),
                kind: None,
                message: String::new(),
            })
        });
        let error = format!("pripojenie k Soniox zlyhalo \"{error}\"");
        self.on_error(class, error, at, out);
    }

    pub(super) fn on_error(
        &mut self,
        class: ErrorClass,
        error: String,
        at: Instant,
        out: &mut Vec<Action>,
    ) {
        if matches!(self.phase, Phase::Stopping { .. }) {
            self.finish(None, None, at, out);
            return;
        }
        match class {
            ErrorClass::Auth => self.finish(Some(OffReason::Auth), Some(error), at, out),
            ErrorClass::Payment => self.finish(Some(OffReason::Payment), Some(error), at, out),
            ErrorClass::BadRequest => self.finish(Some(OffReason::Refused), Some(error), at, out),
            ErrorClass::TooLong => self.lose(at, Retry::Now, Some(error), out),
            ErrorClass::Timeout
            | ErrorClass::RateLimited
            | ErrorClass::Server
            | ErrorClass::Unknown => self.lose(at, Retry::Backoff, Some(error), out),
        }
    }

    /// A report of a draining connection: `true` = handled (ignored, or
    /// its end aborts the task).
    pub(super) fn drained(&mut self, conn: u32, ended: bool, out: &mut Vec<Action>) -> bool {
        let Some(index) = self.draining.iter().position(|(id, _)| *id == conn) else {
            return false;
        };
        if ended {
            self.draining.swap_remove(index);
            out.push(Action::Drop { conn });
        }
        true
    }
}

fn describe(error: &SonioxError) -> String {
    let code = error
        .code
        .map_or_else(|| "-".to_string(), |code| code.to_string());
    let kind = error.kind.as_deref().unwrap_or("-");
    format!("Soniox chyba {code} {kind} \"{}\"", error.message)
}
