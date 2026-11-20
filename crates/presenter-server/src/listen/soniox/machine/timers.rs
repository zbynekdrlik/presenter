//! The clock rules and the transitions: gate, keepalive, park, stall,
//! back-off, rollover, the connection guard, stop (#852 C3-4).

use super::*;

impl SonioxCore {
    pub(super) fn check_timers(&mut self, now: Instant, out: &mut Vec<Action>) {
        let expired: Vec<u32> = self
            .draining
            .iter()
            .filter(|(_, until)| now >= *until)
            .map(|(id, _)| *id)
            .collect();
        self.draining.retain(|(_, until)| now < *until);
        out.extend(expired.into_iter().map(|conn| Action::Drop { conn }));
        match self.phase {
            Phase::Live(_) => self.live_timers(now, out),
            Phase::Backoff { until } if now >= until => {
                if self.speaking(now) {
                    self.connect(now, ConnectKind::Retry, out);
                } else {
                    self.phase = Phase::Idle;
                    self.set_state(TranscriptionState::Parked, None, out);
                }
            }
            Phase::Rolling { until, .. } if now >= until => self.switch_rollover(now, true, out),
            Phase::Stopping { until, .. } if now >= until => self.finish(None, None, now, out),
            _ => {}
        }
        if let Some(due) = self.next_usage_at.filter(|due| now >= *due) {
            out.push(self.usage(now));
            self.next_usage_at = self
                .live_mut()
                .is_some()
                .then(|| due.max(now) + USAGE_EVERY);
        }
    }

    pub(super) fn live_timers(&mut self, now: Instant, out: &mut Vec<Action>) {
        let speaking = self.speaking(now);
        let Some(live) = self.live_mut() else {
            return;
        };
        let conn = live.id;
        let age = now.saturating_duration_since(live.connected_at);
        match live.gate {
            None => {
                let silent_for = now.saturating_duration_since(live.last_response_at);
                if live.audio_since_response_ms >= STALL_MIN_AUDIO_MS && silent_for >= STALL_AFTER {
                    let error = "Soniox neodpovedá — pripájam znova".to_string();
                    self.lose(now, Retry::Backoff, Some(error), out);
                    return;
                }
                if !speaking {
                    self.gate(now, out);
                }
            }
            Some(gate) => {
                if now >= gate.since + PARK_AFTER {
                    self.park(now, out);
                    return;
                }
                if now >= gate.next_keepalive {
                    live.gate = Some(Gate {
                        next_keepalive: now + KEEPALIVE_EVERY,
                        ..gate
                    });
                    out.push(Action::SendControl {
                        conn,
                        control: Control::Keepalive,
                    });
                }
            }
        }
        let gated = self.live_mut().is_some_and(|live| live.gate.is_some());
        if age >= ROLLOVER_HARD || (gated && age >= ROLLOVER_FROM) {
            self.start_rollover(now, out);
        }
    }

    // ---- transitions ----

    pub(super) fn connect(&mut self, at: Instant, kind: ConnectKind, out: &mut Vec<Action>) {
        while self
            .connects
            .front()
            .is_some_and(|t| at.saturating_duration_since(*t) >= CONNECTION_WINDOW)
        {
            self.connects.pop_front();
        }
        if self.connects.len() >= MAX_CONNECTIONS {
            let error = format!("viac ako {MAX_CONNECTIONS} pripojení za 10 minút");
            self.finish(Some(OffReason::TooManyConnections), Some(error), at, out);
            return;
        }
        self.connects.push_back(at);
        let conn = self.next_conn;
        self.next_conn = self.next_conn.wrapping_add(1);
        let offset_ms = self.resend_start();
        self.stats.connections += 1;
        if kind == ConnectKind::Retry {
            self.stats.reconnects += 1;
        }
        out.push(Action::Connect {
            conn,
            offset_ms,
            kind,
        });
        self.phase = Phase::Connecting {
            id: conn,
            offset_ms,
            kind,
        };
        if kind == ConnectKind::Lazy {
            self.set_state(TranscriptionState::Connecting, None, out);
        }
    }

    pub(super) fn gate(&mut self, at: Instant, out: &mut Vec<Action>) {
        if self.unfinalized {
            self.send_finalize(out);
        }
        if let Some(live) = self.live_mut() {
            live.gate = Some(Gate {
                since: at,
                next_keepalive: at + KEEPALIVE_EVERY,
            });
        }
        self.onset_at = None;
        self.set_state(TranscriptionState::Gated, None, out);
    }

    pub(super) fn resume(&mut self, at: Instant, out: &mut Vec<Action>) {
        if let Some(live) = self.live_mut() {
            live.gate = None;
            live.last_response_at = at;
        }
        self.onset_at = Some(at);
        self.set_state(TranscriptionState::Streaming, None, out);
        let preroll: Vec<MicFrame> = self.preroll.drain(..).collect();
        for frame in preroll {
            self.send_new(frame, out);
        }
    }

    pub(super) fn park(&mut self, at: Instant, out: &mut Vec<Action>) {
        if let Some(live) = self.take_live() {
            self.end_connection(live, at, true, out);
        }
        self.phase = Phase::Idle;
        self.set_state(TranscriptionState::Parked, None, out);
    }

    pub(super) fn lose(
        &mut self,
        at: Instant,
        retry: Retry,
        error: Option<String>,
        out: &mut Vec<Action>,
    ) {
        if let Some(live) = self.take_live() {
            self.end_connection(live, at, false, out);
        } else if let Phase::Connecting { id, .. } = self.phase {
            out.push(Action::Drop { conn: id });
        }
        self.phase = Phase::Idle;
        match retry {
            Retry::Now if self.speaking(at) => {
                self.set_state(TranscriptionState::Reconnecting, error, out);
                self.connect(at, ConnectKind::Retry, out);
            }
            Retry::Now => self.set_state(TranscriptionState::Parked, error, out),
            Retry::Backoff => {
                let wait = BACKOFF
                    .get(self.backoff_step)
                    .copied()
                    .unwrap_or(BACKOFF[BACKOFF.len() - 1]);
                self.backoff_step = self.backoff_step.saturating_add(1);
                self.phase = Phase::Backoff { until: at + wait };
                self.set_state(TranscriptionState::Reconnecting, error, out);
            }
        }
    }

    pub(super) fn start_rollover(&mut self, at: Instant, out: &mut Vec<Action>) {
        let Some(live) = self.take_live() else {
            return;
        };
        self.phase = Phase::Rolling {
            live,
            until: at + FIN_WAIT,
        };
        if self.unfinalized {
            self.send_finalize(out);
        }
        if self.finalize.is_none() {
            self.switch_rollover(at, true, out);
        }
    }

    pub(super) fn switch_rollover(&mut self, at: Instant, graceful: bool, out: &mut Vec<Action>) {
        if let Some(live) = self.take_live() {
            self.end_connection(live, at, graceful, out);
        }
        self.connect(at, ConnectKind::Rollover, out);
    }

    /// Ends the session; `reason` = a stop a person must fix.
    pub(super) fn finish(
        &mut self,
        reason: Option<OffReason>,
        error: Option<String>,
        at: Instant,
        out: &mut Vec<Action>,
    ) {
        if let Some(live) = self.take_live() {
            self.end_connection(live, at, false, out);
        } else {
            if let Phase::Connecting { id, .. } = self.phase {
                out.push(Action::Drop { conn: id });
            }
            out.push(self.usage(at));
        }
        out.extend(
            self.draining
                .drain(..)
                .map(|(conn, _)| Action::Drop { conn }),
        );
        if reason.is_some() {
            self.set_state(TranscriptionState::Stopped, error, out);
        }
        self.phase = Phase::Done;
        self.next_usage_at = None;
        out.push(Action::Finished(reason));
    }

    /// `graceful` = the empty TEXT frame, then wait ≤ 3 s for `finished`;
    /// else abort at once.
    pub(super) fn end_connection(
        &mut self,
        live: Live,
        at: Instant,
        graceful: bool,
        out: &mut Vec<Action>,
    ) {
        let conn = live.id;
        if graceful {
            out.push(Action::EndAudio { conn });
            self.draining.push((conn, at + FIN_WAIT));
        } else {
            out.push(Action::Drop { conn });
        }
        out.push(Action::ConnectionEnded { conn });
        let connected = at.saturating_duration_since(live.connected_at);
        self.closed_connected_ms = self.closed_connected_ms.saturating_add(ms_u64(connected));
        self.unfinalized = false;
        self.finalize = None;
        self.next_usage_at = None;
        out.push(self.usage(at));
    }
}
