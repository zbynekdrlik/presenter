//! The VBAN receiver (#852 C1a, spec §5.1 + §5.3): ONE UDP socket, bound only
//! while a mic is configured (the list ships empty, C-S8). The receive loop
//! never waits on anything downstream (spec rev. 13 p. 11): decode, route and
//! count, synchronously. [`supervise`] binds (retrying 1, 2, 4 … 60 s) and
//! closes the socket when the list is emptied; [`MicRouter`] keeps a mic whose
//! stream name and sender stay across a save. Repeated log lines go through
//! the #484 power-of-two gate.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use presenter_core::audio::vban::{decode_packet, VbanPacket};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::SocketState;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{interval, sleep, Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use super::health::{ListenHealth, PacketSeen, UnmatchedCount, RECENT_CAP};
use super::{bind_retry_delay, should_log_event, ListenService, MIC_TARGET, NO_AUDIO_AFTER};

/// Big enough for any datagram (a conformant VBAN packet is ≤ 1464 bytes).
const RECV_BUFFER_LEN: usize = 65_536;
/// The "lost audio" line is evaluated this often.
const TICK: Duration = Duration::from_secs(1);
/// A socket is healthy once it received a datagram or stayed bound this long;
/// only then does a later failure start the retry pause at 1 s again.
const HEALTHY_AFTER: Duration = Duration::from_secs(30);
/// The new-name count at which the bounded name map evicts for the first time.
const FIRST_EVICTION: u64 = RECENT_CAP as u64 + 1;

pub(super) type ConfigRx = watch::Receiver<Arc<Vec<MicConfig>>>;

/// Why a bound socket was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SocketEnd {
    /// The list was emptied (or the service is gone).
    NoMic,
    /// `recv_from` failed: rebind after a pause. `received`: a datagram
    /// arrived before it failed.
    Failed { received: bool, error: String },
}

/// The supervisor's failure streak (#484): failed attempts in a row drive the
/// retry pause and its WARN gate; `early_ends`, the bound sessions that failed
/// before they were healthy, gate the "bound" line. A socket that fails right
/// after every bind therefore neither rebinds every second nor logs each time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Streak {
    pub(super) failures: u32,
    early_ends: u32,
}

impl Streak {
    pub(super) fn bind_failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    /// A bound session failed after `bound_for`; a healthy one resets the
    /// streak first, so its failure retries after 1 s.
    pub(super) fn session_failed(&mut self, received: bool, bound_for: Duration) {
        if received || bound_for >= HEALTHY_AFTER {
            *self = Self::default();
        } else {
            self.early_ends = self.early_ends.saturating_add(1);
        }
        self.failures = self.failures.saturating_add(1);
    }

    /// Log "VBAN socket bound": every bind of a healthy run (a port freed after
    /// bind retries included), then on power-of-two counts of early ends.
    pub(super) fn log_bound(&self) -> bool {
        should_log_event(u64::from(self.early_ends) + 1)
    }
}

/// The socket task: idle while the list is empty, bound while it is not,
/// retrying a port it cannot open (or a socket that failed) 1, 2, 4 … 60 s
/// later, or at once when the list changes.
pub(super) async fn supervise(service: ListenService, bind: SocketAddr) {
    let mut config = service.config_receiver();
    let mut streak = Streak::default();
    loop {
        if config.borrow_and_update().is_empty() {
            streak = Streak::default();
            service.with_health(|health| health.set_socket(SocketState::Idle, None, None));
            if config.changed().await.is_err() {
                return;
            }
            continue;
        }
        let error = match UdpSocket::bind(bind).await {
            Ok(socket) => {
                match bound_session(&service, &socket, &mut config, bind, &mut streak).await {
                    None => continue,
                    Some(error) => error,
                }
            }
            Err(err) => {
                streak.bind_failed();
                format!("port {} sa nedá otvoriť: {err}", bind.port())
            }
        };
        let delay = bind_retry_delay(streak.failures);
        if should_log_event(u64::from(streak.failures)) {
            warn!(
                target: MIC_TARGET,
                %bind,
                error = error.as_str(),
                attempt = streak.failures,
                retry_in_s = delay.as_secs(),
                "the VBAN socket is not receiving — retrying"
            );
        }
        service.with_health(|health| health.set_socket(SocketState::Retrying, None, Some(error)));
        tokio::select! {
            () = sleep(delay) => {}
            changed = config.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// One bound socket: receive until the list is emptied (`None`) or receiving
/// fails (`Some`, the Slovak status error). The session's counters go with
/// the socket.
async fn bound_session(
    service: &ListenService,
    socket: &UdpSocket,
    config: &mut ConfigRx,
    bind: SocketAddr,
    streak: &mut Streak,
) -> Option<String> {
    let local = socket.local_addr().ok();
    if streak.log_bound() {
        info!(target: MIC_TARGET, %bind, local = ?local, "VBAN socket bound");
    }
    service.with_health(|health| health.set_socket(SocketState::Bound, local, None));
    let bound_at = Instant::now();
    let end = receive(service, socket, config).await;
    service.with_health(ListenHealth::end_session);
    match end {
        SocketEnd::NoMic => {
            info!(target: MIC_TARGET, %bind, "VBAN socket closed — no microphone is configured");
            None
        }
        SocketEnd::Failed { received, error } => {
            streak.session_failed(received, bound_at.elapsed());
            Some(format!("príjem zo socketu VBAN zlyhal: {error}"))
        }
    }
}

/// Receive until the list is emptied or the socket fails.
pub(super) async fn receive(
    service: &ListenService,
    socket: &UdpSocket,
    config: &mut ConfigRx,
) -> SocketEnd {
    let mics = config.borrow_and_update().clone();
    if mics.is_empty() {
        // emptied between `supervise`'s idle check and the bind (review 1b)
        return SocketEnd::NoMic;
    }
    let mut router = MicRouter::new(service, &mics);
    let mut buf = vec![0u8; RECV_BUFFER_LEN];
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut received = false;
    loop {
        tokio::select! {
            result = socket.recv_from(&mut buf) => match result {
                Ok((len, from)) => {
                    received = true;
                    router.handle_datagram(&buf[..len], from, Instant::now());
                }
                Err(err) => return SocketEnd::Failed { received, error: err.to_string() },
            },
            changed = config.changed() => {
                if changed.is_err() {
                    return SocketEnd::NoMic;
                }
                let mics = config.borrow_and_update().clone();
                if mics.is_empty() {
                    return SocketEnd::NoMic;
                }
                router.update(&mics);
            }
            _ = tick.tick() => router.on_tick(Instant::now()),
        }
    }
}

/// The receive side of every configured mic.
pub(super) struct MicRouter {
    service: ListenService,
    mics: Vec<MicInput>,
}

struct MicInput {
    config: MicConfig,
    /// Parsed once per saved list, never per packet.
    sender_ip: Option<IpAddr>,
    last_packet: Option<Instant>,
    /// "receives" was logged and "lost audio" not yet.
    receiving: bool,
}

impl MicRouter {
    pub(super) fn new(service: &ListenService, configs: &[MicConfig]) -> Self {
        let mut router = Self {
            service: service.clone(),
            mics: Vec::new(),
        };
        router.update(configs);
        router
    }

    /// A saved list: a mic whose stream name and sender stay keeps its state
    /// and counters (a rename or a language change resets nothing); every
    /// other mic starts fresh; an unlisted one goes.
    pub(super) fn update(&mut self, configs: &[MicConfig]) {
        let mut old: HashMap<String, MicInput> = self
            .mics
            .drain(..)
            .map(|input| (input.config.id.clone(), input))
            .collect();
        let kept: Vec<&str> = configs
            .iter()
            .filter(|config| {
                old.get(&config.id)
                    .is_some_and(|input| input.receives_like(config))
            })
            .map(|config| config.id.as_str())
            .collect();
        let fresh: Vec<&str> = configs
            .iter()
            .map(|config| config.id.as_str())
            .filter(|id| !kept.contains(id))
            .collect();
        self.service
            .with_health(|health| health.set_mics(&kept, &fresh));
        self.mics = configs
            .iter()
            .map(|config| match old.remove(&config.id) {
                Some(mut input) if input.receives_like(config) => {
                    input.config = config.clone();
                    input
                }
                _ => MicInput::new(config),
            })
            .collect();
    }

    pub(super) fn handle_datagram(&mut self, datagram: &[u8], from: SocketAddr, now: Instant) {
        let packet = match decode_packet(datagram) {
            Ok(packet) => packet,
            Err(reject) => {
                debug!(
                    target: MIC_TARGET,
                    %from,
                    %reject,
                    "dropped a datagram that is not usable VBAN audio"
                );
                return;
            }
        };
        // C1a-1 review 1: `validate_mics` stores the sender IP canonical
        // (`::ffff:192.0.2.10` → `192.0.2.10`), so the peer is canonicalised too.
        let sender = from.ip().to_canonical();
        let index = self
            .mics
            .iter()
            .position(|mic| mic.matches(&packet.stream_name, sender));
        match index {
            Some(index) => self.mics[index].accept(&self.service, &packet, from, now),
            None => self.unmatched(&packet, from, now),
        }
    }

    pub(super) fn on_tick(&mut self, now: Instant) {
        for mic in &mut self.mics {
            let stale = mic
                .last_packet
                .is_some_and(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
            if mic.receiving && stale {
                mic.receiving = false;
                info!(
                    target: MIC_TARGET,
                    mic = mic.config.id.as_str(),
                    stream = mic.config.stream_name.as_str(),
                    "mic lost audio — no VBAN packet for 3 s"
                );
            }
        }
    }

    /// Counted per (name, sender) for the status; logged through
    /// [`unmatched_lines`] (bounded maps, review 1f).
    fn unmatched(&self, packet: &VbanPacket, from: SocketAddr, now: Instant) {
        let seen = self.service.with_health(|health| {
            health.note_unmatched(&packet.stream_name, from, packet.format, now)
        });
        let (line, summary) = unmatched_lines(seen);
        if line {
            info!(
                target: MIC_TARGET,
                stream = packet.stream_name.as_str(),
                %from,
                rate = packet.format.rate_hz,
                channels = packet.format.channels,
                packets = seen.packets,
                names = seen.names,
                "VBAN stream not assigned to any mic"
            );
        }
        if summary {
            info!(
                target: MIC_TARGET,
                %from,
                names = seen.names,
                kept = RECENT_CAP,
                "unassigned VBAN stream names rotate through more than the names kept — \
                 a new name is logged on power-of-two counts of new names only"
            );
        }
    }
}

/// The #484 gates of the unassigned-stream lines, `(line, summary)`. A name's
/// later packets log on its own power-of-two count; a name that enters the
/// bounded map (new, or back after an eviction) logs on the power-of-two
/// count of such names this session, so a sender cycling more names than the
/// map keeps logs log2(N) lines for N packets, not N. The summary marks the
/// first eviction, once per session.
fn unmatched_lines(seen: UnmatchedCount) -> (bool, bool) {
    if seen.fresh {
        (should_log_event(seen.names), seen.names == FIRST_EVICTION)
    } else {
        (should_log_event(seen.packets), false)
    }
}

impl MicInput {
    fn new(config: &MicConfig) -> Self {
        Self {
            sender_ip: config.sender_ip_addr(),
            config: config.clone(),
            last_packet: None,
            receiving: false,
        }
    }

    /// The same stream name and sender: the mic keeps its state on a save.
    fn receives_like(&self, config: &MicConfig) -> bool {
        self.config.stream_name == config.stream_name && self.config.sender_ip == config.sender_ip
    }

    /// Exact stream name, and the configured sender if there is one.
    fn matches(&self, stream_name: &str, from: IpAddr) -> bool {
        self.config.stream_name == stream_name && self.sender_ip.is_none_or(|ip| ip == from)
    }

    fn accept(
        &mut self,
        service: &ListenService,
        packet: &VbanPacket,
        from: SocketAddr,
        now: Instant,
    ) {
        let silent = self
            .last_packet
            .is_none_or(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
        if silent {
            self.log_receiving(packet, from);
        }
        self.last_packet = Some(now);
        let seen = PacketSeen {
            from,
            format: packet.format,
            frames: packet.frames,
            at: now,
        };
        service.with_health(|health| health.record_packet(&self.config.id, seen));
    }

    fn log_receiving(&mut self, packet: &VbanPacket, from: SocketAddr) {
        self.receiving = true;
        info!(
            target: MIC_TARGET,
            mic = self.config.id.as_str(),
            stream = self.config.stream_name.as_str(),
            %from,
            rate = packet.format.rate_hz,
            channels = packet.format.channels,
            sample_type = packet.format.sample_type.as_str(),
            frames = packet.frames,
            "mic receives VBAN"
        );
    }
}
