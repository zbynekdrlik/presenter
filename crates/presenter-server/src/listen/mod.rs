//! Sermon listening, the audio ingest (#852 C1a, spec §5): ONE UDP socket
//! receives the VBAN microphones (`vban_rx`); `GET /listen/status` reads
//! `health`. [`ListenService`] (in `AppState`) holds the current `listen.mics`
//! (a `watch`), the health (a std mutex, never held across an `.await`), the
//! start-once flag and the save lock. Started from `spawn_background_tasks`
//! only, so validate mode never binds. Each mic has a chain (counter rule →
//! 16 kHz → level, `mic_chain`); no frame channel yet: it arrives with its
//! first consumer (C1b recorder / C3 ASR). The recorder (`recorder`,
//! `recording_file`, #852 C1b) has its own supervisor task, started with the
//! socket task; it deletes day folders older than 30 days once per local date.

pub(crate) mod health;
mod mic_chain;
pub(crate) mod recorder;
mod recording_file;
mod resample;
#[cfg(test)]
mod tests;
mod vban_rx;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::ListenStatus;
use tokio::sync::watch;
use tokio::time::Instant;

use self::health::ListenHealth;
use self::recorder::RecorderEnv;

/// The target of every receiver log line (spec §14 `presenter::listen::mic`).
pub(crate) const MIC_TARGET: &str = "presenter::listen::mic";
/// Overrides where the VBAN socket binds (tests / E2E: `127.0.0.1:0`).
pub(crate) const VBAN_ADDR_ENV: &str = "PRESENTER_LISTEN_VBAN_ADDR";
/// Every interface, the VBAN default port (spec §5.1).
pub(crate) const DEFAULT_VBAN_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), VBAN_DEFAULT_PORT);
/// A mic whose last packet is this old has no audio ("žiadny zvuk" within 3 s, C-S1).
pub(crate) const NO_AUDIO_AFTER: Duration = Duration::from_secs(3);
const BIND_RETRY_CAP: Duration = Duration::from_secs(60);

/// The listen state every `AppState` clone shares.
#[derive(Clone)]
pub(crate) struct ListenService {
    inner: Arc<Inner>,
}

struct Inner {
    config: watch::Sender<Arc<Vec<MicConfig>>>,
    health: Mutex<ListenHealth>,
    started: AtomicBool,
    save_lock: tokio::sync::Mutex<()>,
    /// Where the recorder writes and what time it is (#852 C1b).
    recorder_env: Mutex<RecorderEnv>,
}

impl ListenService {
    pub(crate) fn new() -> Self {
        let (config, _) = watch::channel(Arc::new(Vec::new()));
        Self {
            inner: Arc::new(Inner {
                config,
                health: Mutex::new(ListenHealth::default()),
                started: AtomicBool::new(false),
                save_lock: tokio::sync::Mutex::new(()),
                recorder_env: Mutex::new(RecorderEnv::from_env()),
            }),
        }
    }

    /// The mics the receiver uses now.
    pub(crate) fn mics(&self) -> Arc<Vec<MicConfig>> {
        Arc::clone(&self.inner.config.borrow())
    }

    /// Hand a validated list to the receiver; it updates its mics at once.
    pub(crate) fn apply(&self, mics: Vec<MicConfig>) {
        let _previous = self.inner.config.send_replace(Arc::new(mics));
    }

    /// `GET /listen/status`, ages measured now.
    pub(crate) fn status(&self) -> ListenStatus {
        let mics = self.mics();
        self.with_health(|health| health.snapshot(&mics, Instant::now()))
    }

    /// Start the socket task and the recorder's supervisor once; `false` when
    /// they were already started.
    pub(crate) fn start(&self, bind: SocketAddr) -> bool {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.with_health(|health| health.socket.bind_addr = Some(bind));
        tokio::spawn(vban_rx::supervise(self.clone(), bind));
        tokio::spawn(recorder::supervise(self.clone()));
        true
    }

    /// Serializes `listen.mics` writers AND the startup read, so the stored and
    /// the applied list always agree.
    pub(crate) async fn save_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.save_lock.lock().await
    }

    pub(crate) fn with_health<R>(&self, f: impl FnOnce(&mut ListenHealth) -> R) -> R {
        let mut health = self
            .inner
            .health
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(&mut health)
    }

    fn config_receiver(&self) -> watch::Receiver<Arc<Vec<MicConfig>>> {
        self.inner.config.subscribe()
    }

    /// The recorder's folder and clock.
    fn recorder_env(&self) -> RecorderEnv {
        self.inner
            .recorder_env
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Point the recorder at a test folder and a test clock.
    #[cfg(test)]
    pub(crate) fn set_recorder_env(&self, env: RecorderEnv) {
        *self
            .inner
            .recorder_env
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = env;
    }
}

/// Where the VBAN socket binds: `PRESENTER_LISTEN_VBAN_ADDR`, else 0.0.0.0:6980.
pub(crate) fn vban_bind_addr() -> SocketAddr {
    parse_bind_addr(std::env::var(VBAN_ADDR_ENV).ok().as_deref())
}

pub(crate) fn parse_bind_addr(raw: Option<&str>) -> SocketAddr {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_VBAN_ADDR;
    };
    value.parse().unwrap_or_else(|err| {
        tracing::warn!(
            target: MIC_TARGET,
            value,
            %err,
            "invalid PRESENTER_LISTEN_VBAN_ADDR — binding 0.0.0.0:6980"
        );
        DEFAULT_VBAN_ADDR
    })
}

/// The #484 log gate: the 1st event and every power-of-two one.
pub(crate) fn should_log_event(count: u64) -> bool {
    count.is_power_of_two()
}

/// The pause before bind attempt `failures + 1`: 1, 2, 4 … s, at most 60 s
/// (the #484 `backoff_interval` shape).
pub(crate) fn bind_retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(32);
    Duration::from_secs(1u64.checked_shl(shift).unwrap_or(u64::MAX)).min(BIND_RETRY_CAP)
}
