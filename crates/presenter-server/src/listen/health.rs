//! What `GET /listen/status` reports (#852 C1a, spec §5.4, §14): the socket,
//! the per-mic counters and the streams seen but not used. Ages are computed
//! at read time: "no audio" (C-S1) appears exactly 3 s after the last packet.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::SocketAddr;

use presenter_core::audio::vban::VbanAudioFormat;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::{
    ListenStatus, MicState, MicStatus, SocketState, SocketStatus, UnmatchedStream,
};
use tokio::time::Instant;

use super::NO_AUDIO_AFTER;

/// Streams seen but not used (and their names) are kept at most this many
/// (the oldest goes): a spoofed flood cannot grow the status.
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

#[derive(Debug, Default)]
struct MicCounters {
    sender: Option<SocketAddr>,
    format: Option<VbanAudioFormat>,
    frames_per_packet: Option<u16>,
    packets: u64,
    last_packet: Option<Instant>,
}

#[derive(Debug, Default)]
struct UnmatchedEntry {
    format: Option<VbanAudioFormat>,
    packets: u64,
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
    mics: HashMap<String, MicCounters>,
    unmatched: Recent<(String, SocketAddr), UnmatchedEntry>,
    /// Packets per unassigned stream NAME: the #484 gate of its INFO line.
    unmatched_names: Recent<String, u64>,
    /// Names that entered `unmatched_names` this session, again after an
    /// eviction too: the gate of a new name's line, so a sender cycling more
    /// names than the map keeps cannot log every packet.
    new_names: u64,
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

    /// A saved mic list: `kept` mics keep their counters (same stream and
    /// sender — a rename or a language change resets nothing), `fresh` ones
    /// start from zero, mics no longer listed go.
    pub(crate) fn set_mics(&mut self, kept: &[&str], fresh: &[&str]) {
        self.mics.retain(|id, _| kept.contains(&id.as_str()));
        for id in fresh {
            self.mics.insert((*id).to_string(), MicCounters::default());
        }
    }

    /// The socket closed: the session's counters go, so a later bind (or a
    /// failed rebind under a new stream name) never shows the last session's
    /// numbers, and the unassigned-stream log gates start over.
    pub(crate) fn end_session(&mut self) {
        self.mics.clear();
        self.unmatched = Recent::default();
        self.unmatched_names = Recent::default();
        self.new_names = 0;
    }

    pub(crate) fn record_packet(&mut self, mic_id: &str, seen: PacketSeen) {
        let Some(mic) = self.mics.get_mut(mic_id) else {
            return;
        };
        mic.packets += 1;
        mic.last_packet = Some(seen.at);
        mic.sender = Some(seen.from);
        mic.format = Some(seen.format);
        mic.frames_per_packet = Some(seen.frames);
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
        UnmatchedCount {
            packets,
            fresh,
            names: self.new_names,
        }
    }

    pub(crate) fn snapshot(&self, mics: &[MicConfig], now: Instant) -> ListenStatus {
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
            unmatched,
        }
    }
}

/// Waiting (no packet yet) → receiving → no audio 3 s after the last packet.
fn mic_state(counters: &MicCounters, now: Instant) -> MicState {
    match counters.last_packet {
        None => MicState::Waiting,
        Some(at) if now.saturating_duration_since(at) >= NO_AUDIO_AFTER => MicState::NoAudio,
        Some(_) => MicState::Receiving,
    }
}

fn mic_status(config: &MicConfig, counters: Option<&MicCounters>, now: Instant) -> MicStatus {
    let empty = MicCounters::default();
    let c = counters.unwrap_or(&empty);
    MicStatus {
        config: config.clone(),
        state: mic_state(c, now),
        sender: c.sender.map(|addr| addr.to_string()),
        format: c.format,
        frames_per_packet: c.frames_per_packet,
        packets: c.packets,
        last_packet_age_ms: c.last_packet.map(|at| age_ms(at, now)),
    }
}

fn age_ms(at: Instant, now: Instant) -> u64 {
    u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX)
}
