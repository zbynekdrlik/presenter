//! What `GET /listen/status` reports (#852 C1a, spec §5.4, §14): the socket,
//! the per-mic counters (the counter rule's and the chain's included), the
//! streams seen but not used and the senders whose datagrams were refused.
//! Ages are computed at read time: "no audio" (C-S1) appears exactly 3 s
//! after the last packet. The #484 log gates read these counters (the counts
//! the `record_*` / `note_*` calls return) — there is no second set.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, SocketAddr};

use presenter_core::audio::level::is_voiced;
use presenter_core::audio::sequence::SequenceVerdict;
use presenter_core::audio::vban::{VbanAudioFormat, VbanReject};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::{
    ListenStatus, MicState, MicStatus, RejectedSender, SocketState, SocketStatus, UnmatchedStream,
};
use tokio::time::Instant;

use super::NO_AUDIO_AFTER;

/// Streams seen but not used (and their names) and the refused senders are
/// kept at most this many each (the oldest goes): a spoofed flood cannot grow
/// the status.
pub(crate) const RECENT_CAP: usize = 16;

#[derive(Debug)]
pub(crate) struct SocketHealth {
    pub(crate) state: SocketState,
    pub(crate) bind_addr: Option<SocketAddr>,
    pub(crate) local_addr: Option<SocketAddr>,
    pub(crate) error: Option<String>,
}

impl Default for SocketHealth {
    fn default() -> Self {
        Self {
            state: SocketState::NotStarted,
            bind_addr: None,
            local_addr: None,
            error: None,
        }
    }
}

/// One packet a mic took.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PacketSeen {
    pub(crate) from: SocketAddr,
    pub(crate) format: VbanAudioFormat,
    pub(crate) frames: u16,
    pub(crate) at: Instant,
}

/// The counts after one packet — the #484 log gate reads them, so `MicInput`
/// keeps no second set of event counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PacketTally {
    /// The count of this packet's verdict kind (gaps, late packets, resyncs); 0 in order.
    pub(crate) events: u64,
    /// `droppedBlocks` when this packet's block pushed out the oldest, else 0.
    pub(crate) dropped_blocks: u64,
}

#[derive(Debug, Default)]
struct MicCounters {
    sender: Option<SocketAddr>,
    format: Option<VbanAudioFormat>,
    frames_per_packet: Option<u16>,
    packets: u64,
    last_packet: Option<Instant>,
    packets_per_sec: f32,
    packets_at_tick: u64,
    tick_at: Option<Instant>,
    /// The mic list update that started these counters: a closed chain of an
    /// older update can no longer write them; a kept mic keeps its value.
    generation: u64,
    lost_packets: u64,
    late_packets: u64,
    resyncs: u64,
    /// Gaps (events, not packets): the #484 gate of the "lost packets" WARN.
    gap_events: u64,
    dropped_blocks: u64,
    other_sender_packets: u64,
    frames_out: u64,
    level_dbfs: Option<f32>,
    error: Option<String>,
    /// Formats the chain switched to (its first one included): the #484 gate
    /// of the converter lines.
    formats: u64,
}

#[derive(Debug, Default)]
struct UnmatchedEntry {
    format: Option<VbanAudioFormat>,
    packets: u64,
}

#[derive(Debug, Default)]
struct RejectCounts {
    not_vban: u64,
    non_audio: u64,
    unsupported_codec: u64,
    unsupported_type: u64,
    corrupt: u64,
}

impl RejectCounts {
    /// Count one reject; the count its log line reads: a non-audio packet's
    /// own count (DEBUG only), else the sender's total of every OTHER kind —
    /// the WARN gate, so a VB-Matrix host's frequent VBAN-TEXT / SERVICE
    /// packets never take the power-of-two slots of its rare corrupt or
    /// unsupported datagrams.
    fn count(&mut self, reject: VbanReject) -> u64 {
        let slot = match reject {
            VbanReject::NonAudio(_) => {
                self.non_audio += 1;
                return self.non_audio;
            }
            VbanReject::NotVban => &mut self.not_vban,
            VbanReject::UnsupportedCodec(_) => &mut self.unsupported_codec,
            VbanReject::UnsupportedType(_) => &mut self.unsupported_type,
            VbanReject::Corrupt => &mut self.corrupt,
        };
        *slot += 1;
        self.not_vban + self.unsupported_codec + self.unsupported_type + self.corrupt
    }
}

/// What [`ListenHealth::note_unmatched`] saw: the input of the #484 gates of
/// the unassigned-stream lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnmatchedCount {
    /// Packets of this stream NAME while it stayed in the bounded name map.
    pub(crate) packets: u64,
    /// The name entered the map with this packet (new, or back after an eviction).
    pub(crate) fresh: bool,
    /// Names that entered the map this socket session.
    pub(crate) names: u64,
    /// Unassigned datagrams this socket session, every name and sender.
    pub(crate) total: u64,
}

/// What [`ListenHealth::note_reject_seen`] saw: the input of the #484 gate of
/// the refused-datagram WARN. Like [`UnmatchedCount`], the per-sender count
/// lives in a bounded map and restarts when its sender is evicted, so the gate
/// also reads two session-wide counts a sender cycling through the map cannot
/// reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RejectSeen {
    /// This reject's count: a non-audio packet's own count (DEBUG only), else
    /// the sender's total of every other kind while it stayed in the map.
    pub(crate) count: u64,
    /// Senders whose first WARN-class reject was counted this socket session:
    /// new ones, and ones that came back after an eviction.
    pub(crate) senders: u64,
    /// WARN-class rejects this socket session, every sender, non-audio left out.
    pub(crate) total: u64,
    /// The reject was non-audio VBAN: DEBUG only, no part of the WARN gate.
    pub(crate) non_audio: bool,
}

/// A map that keeps the [`RECENT_CAP`] most recently seen keys.
#[derive(Debug)]
struct Recent<K, V> {
    entries: HashMap<K, (V, Instant)>,
}

impl<K, V> Default for Recent<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, V: Default> Recent<K, V> {
    /// The entry of `key`, seen `now` (a new key pushes out the oldest), and
    /// whether the key entered the map just now.
    fn touch(&mut self, key: K, now: Instant) -> (&mut V, bool) {
        let fresh = !self.entries.contains_key(&key);
        if fresh && self.entries.len() >= RECENT_CAP {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(oldest, _)| oldest.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| (V::default(), now));
        entry.1 = now;
        (&mut entry.0, fresh)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ListenHealth {
    pub(crate) socket: SocketHealth,
    /// The last mic list update. Never reset: a chain still draining from an
    /// earlier socket session can never match a later mic.
    generation: u64,
    mics: HashMap<String, MicCounters>,
    unmatched: Recent<(String, SocketAddr), UnmatchedEntry>,
    /// Packets per unassigned stream NAME: the #484 gate of its INFO line.
    unmatched_names: Recent<String, u64>,
    /// Names that entered `unmatched_names` this session, again after an
    /// eviction too: the gate of a new name's line, so a sender cycling more
    /// names than the map keeps cannot log every packet.
    new_names: u64,
    /// Unassigned datagrams this session: the second gate of a repeat line.
    unmatched_total: u64,
    /// Refused datagrams per sender IP; each total is that sender's WARN gate.
    rejected: Recent<IpAddr, RejectCounts>,
    /// Senders whose first WARN-class reject was counted this session, again
    /// after an eviction too: the gate of a returning sender's line, so more
    /// senders than the map keeps cannot log every datagram.
    reject_senders: u64,
    /// WARN-class rejects this session (non-audio left out): the second gate of
    /// a repeat line.
    reject_total: u64,
}

impl ListenHealth {
    pub(crate) fn set_socket(
        &mut self,
        state: SocketState,
        local_addr: Option<SocketAddr>,
        error: Option<String>,
    ) {
        self.socket.state = state;
        self.socket.local_addr = local_addr;
        self.socket.error = error;
    }

    /// A saved mic list: `kept` mics keep their counters and generation (same
    /// stream, sender and channel — a rename or a language change resets
    /// nothing), `fresh` ones start from zero with this update's generation,
    /// mics no longer listed go. Returns the generation.
    pub(crate) fn set_mics(&mut self, kept: &[&str], fresh: &[&str]) -> u64 {
        self.generation += 1;
        self.mics.retain(|id, _| kept.contains(&id.as_str()));
        for id in fresh {
            let counters = MicCounters {
                generation: self.generation,
                ..MicCounters::default()
            };
            self.mics.insert((*id).to_string(), counters);
        }
        self.generation
    }

    /// The socket closed: the session's counters go, so a later bind (or a
    /// failed rebind under a new stream name) never shows the last session's
    /// numbers, and the unassigned-stream and reject log gates (their
    /// session-wide counts included) start over.
    pub(crate) fn end_session(&mut self) {
        self.mics.clear();
        self.unmatched = Recent::default();
        self.unmatched_names = Recent::default();
        self.new_names = 0;
        self.unmatched_total = 0;
        self.rejected = Recent::default();
        self.reject_senders = 0;
        self.reject_total = 0;
    }

    /// Packets per second over the time since the last tick.
    pub(crate) fn on_tick(&mut self, now: Instant) {
        for mic in self.mics.values_mut() {
            if let Some(at) = mic.tick_at {
                let seconds = now.saturating_duration_since(at).as_secs_f32();
                if seconds > 0.0 {
                    let packets = mic.packets.saturating_sub(mic.packets_at_tick) as f32;
                    mic.packets_per_sec = (packets / seconds * 10.0).round() / 10.0;
                }
            }
            mic.packets_at_tick = mic.packets;
            mic.tick_at = Some(now);
        }
    }

    /// One packet a mic took with the counter rule's verdict on it; the counts
    /// the #484 gates read.
    pub(crate) fn record_packet(
        &mut self,
        mic_id: &str,
        seen: PacketSeen,
        verdict: SequenceVerdict,
        dropped_block: bool,
    ) -> PacketTally {
        let Some(mic) = self.mics.get_mut(mic_id) else {
            return PacketTally::default();
        };
        mic.packets += 1;
        mic.last_packet = Some(seen.at);
        mic.sender = Some(seen.from);
        mic.format = Some(seen.format);
        mic.frames_per_packet = Some(seen.frames);
        let events = match verdict {
            SequenceVerdict::Gap { missing, .. } => {
                mic.lost_packets += u64::from(missing);
                mic.gap_events += 1;
                mic.gap_events
            }
            SequenceVerdict::Late => {
                mic.late_packets += 1;
                mic.late_packets
            }
            SequenceVerdict::Resync => {
                mic.resyncs += 1;
                mic.resyncs
            }
            SequenceVerdict::InOrder => 0,
        };
        if dropped_block {
            mic.dropped_blocks += 1;
        }
        PacketTally {
            events,
            dropped_blocks: if dropped_block { mic.dropped_blocks } else { 0 },
        }
    }

    /// A packet of a second sender of the mic's stream; that count.
    pub(crate) fn record_other_sender(&mut self, mic_id: &str) -> u64 {
        self.mics.get_mut(mic_id).map_or(0, |mic| {
            mic.other_sender_packets += 1;
            mic.other_sender_packets
        })
    }

    /// A chain's output; ignored unless `generation` is the mic's current one.
    pub(crate) fn record_chain(
        &mut self,
        mic_id: &str,
        generation: u64,
        frames: u64,
        level_dbfs: Option<f32>,
        error: Option<&str>,
    ) {
        let Some(mic) = self
            .mics
            .get_mut(mic_id)
            .filter(|mic| mic.generation == generation)
        else {
            return;
        };
        mic.frames_out += frames;
        if level_dbfs.is_some() {
            mic.level_dbfs = level_dbfs;
        }
        mic.error = error.map(str::to_string);
    }

    /// A format the mic's chain switched to; the mic's formats this session
    /// (0 unless `generation` is the mic's current one).
    pub(crate) fn record_format(&mut self, mic_id: &str, generation: u64) -> u64 {
        self.mics
            .get_mut(mic_id)
            .filter(|mic| mic.generation == generation)
            .map_or(0, |mic| {
                mic.formats += 1;
                mic.formats
            })
    }

    /// A refused datagram of `from`, counted per sender and kind for the
    /// status and session-wide for the log gate. A non-audio packet leaves the
    /// session counts alone; any other reject adds to the session total, and
    /// to the session's senders when it is its sender's first one in the map.
    pub(crate) fn note_reject_seen(
        &mut self,
        from: IpAddr,
        reject: VbanReject,
        now: Instant,
    ) -> RejectSeen {
        let non_audio = matches!(reject, VbanReject::NonAudio(_));
        let count = self.rejected.touch(from, now).0.count(reject);
        if !non_audio {
            self.reject_total += 1;
            if count == 1 {
                self.reject_senders += 1;
            }
        }
        RejectSeen {
            count,
            senders: self.reject_senders,
            total: self.reject_total,
            non_audio,
        }
    }

    /// [`Self::note_reject_seen`]'s count only: non-audio, its own count; else
    /// the sender's WARN-gate total, non-audio left out.
    #[cfg(test)]
    pub(crate) fn note_reject(&mut self, from: IpAddr, reject: VbanReject, now: Instant) -> u64 {
        self.note_reject_seen(from, reject, now).count
    }

    /// A stream no mic takes: counted per (name, sender) for the status and
    /// per name for the log gates.
    pub(crate) fn note_unmatched(
        &mut self,
        stream_name: &str,
        from: SocketAddr,
        format: VbanAudioFormat,
        now: Instant,
    ) -> UnmatchedCount {
        let (entry, _) = self.unmatched.touch((stream_name.to_string(), from), now);
        entry.packets += 1;
        entry.format = Some(format);
        let (count, fresh) = self.unmatched_names.touch(stream_name.to_string(), now);
        *count += 1;
        let packets = *count;
        if fresh {
            self.new_names += 1;
        }
        self.unmatched_total += 1;
        UnmatchedCount {
            packets,
            fresh,
            names: self.new_names,
            total: self.unmatched_total,
        }
    }

    pub(crate) fn snapshot(&self, mics: &[MicConfig], now: Instant) -> ListenStatus {
        ListenStatus {
            socket: SocketStatus {
                state: self.socket.state,
                bind_addr: self.socket.bind_addr.map(|addr| addr.to_string()),
                local_addr: self.socket.local_addr.map(|addr| addr.to_string()),
                error: self.socket.error.clone(),
            },
            mics: mics
                .iter()
                .map(|config| mic_status(config, self.mics.get(&config.id), now))
                .collect(),
            unmatched: self.unmatched_streams(now),
            rejected: self.rejected_senders(now),
        }
    }

    /// The streams seen but not used, sorted by name, then sender.
    fn unmatched_streams(&self, now: Instant) -> Vec<UnmatchedStream> {
        let mut unmatched: Vec<UnmatchedStream> = self
            .unmatched
            .entries
            .iter()
            .filter_map(|((stream_name, from), (entry, seen))| {
                Some(UnmatchedStream {
                    stream_name: stream_name.clone(),
                    sender: from.to_string(),
                    format: entry.format?,
                    packets: entry.packets,
                    last_packet_age_ms: age_ms(*seen, now),
                })
            })
            .collect();
        unmatched.sort_by(|a, b| {
            a.stream_name
                .cmp(&b.stream_name)
                .then_with(|| a.sender.cmp(&b.sender))
        });
        unmatched
    }

    /// The refused senders, sorted by IP text.
    fn rejected_senders(&self, now: Instant) -> Vec<RejectedSender> {
        let mut rejected: Vec<RejectedSender> = self
            .rejected
            .entries
            .iter()
            .map(|(ip, (counts, seen))| RejectedSender {
                sender: ip.to_string(),
                not_vban: counts.not_vban,
                non_audio: counts.non_audio,
                unsupported_codec: counts.unsupported_codec,
                unsupported_type: counts.unsupported_type,
                corrupt: counts.corrupt,
                last_packet_age_ms: age_ms(*seen, now),
            })
            .collect();
        rejected.sort_by(|a, b| a.sender.cmp(&b.sender));
        rejected
    }
}

/// Waiting (no packet yet) → receiving → no audio 3 s after the last packet;
/// error while packets arrive in a format the converter refused.
fn mic_state(counters: &MicCounters, now: Instant) -> MicState {
    match counters.last_packet {
        None => MicState::Waiting,
        Some(at) if now.saturating_duration_since(at) >= NO_AUDIO_AFTER => MicState::NoAudio,
        Some(_) if counters.error.is_some() => MicState::Error,
        Some(_) => MicState::Receiving,
    }
}

fn mic_status(config: &MicConfig, counters: Option<&MicCounters>, now: Instant) -> MicStatus {
    let empty = MicCounters::default();
    let c = counters.unwrap_or(&empty);
    let state = mic_state(c, now);
    let receiving = state == MicState::Receiving;
    // packets arrive in the error state too (in a format the converter refused)
    let arriving = receiving || state == MicState::Error;
    MicStatus {
        config: config.clone(),
        state,
        sender: c.sender.map(|addr| addr.to_string()),
        format: c.format,
        frames_per_packet: c.frames_per_packet,
        packets: c.packets,
        packets_per_sec: if arriving { c.packets_per_sec } else { 0.0 },
        last_packet_age_ms: c.last_packet.map(|at| age_ms(at, now)),
        lost_packets: c.lost_packets,
        late_packets: c.late_packets,
        resyncs: c.resyncs,
        dropped_blocks: c.dropped_blocks,
        other_sender_packets: c.other_sender_packets,
        frames_out: c.frames_out,
        level_dbfs: if receiving { c.level_dbfs } else { None },
        voiced: receiving && c.level_dbfs.is_some_and(is_voiced),
        error: c.error.clone(),
    }
}

fn age_ms(at: Instant, now: Instant) -> u64 {
    u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX)
}
