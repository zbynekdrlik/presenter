//! Sermon listening, the audio ingest (#852 C1a, spec §5): ONE UDP socket
//! receives the VBAN microphones (`vban_rx`); `GET /listen/status` reads
//! `health`. [`ListenService`] (in `AppState`) holds the current `listen.mics`
//! (a `watch`), the health (a std mutex, never held across an `.await`), the
//! start-once flag and the save lock. Started from `spawn_background_tasks`
//! only, so validate mode never binds. Each mic has a chain (counter rule →
//! 16 kHz → level, `mic_chain`) that hands its frames to the mic's frame
//! channel (#852 C1b): one `broadcast` per mic id that never makes the chain
//! wait, read by the recorder (C1b) and later the ASR (C3). The recorder
//! (`recorder`, `recording_file`) has its own supervisor task, started with the
//! socket task: it keeps one writer per configured mic while the per-day toggle
//! names today and deletes day folders older than 30 days once per local date.

pub(crate) mod health;
mod mic_chain;
pub(crate) mod recorder;
mod recording_file;
mod resample;
#[cfg(test)]
mod tests;
mod vban_rx;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::NaiveDate;
use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::recorder::RecorderSetting;
use presenter_core::listen::status::ListenStatus;
use tokio::sync::{broadcast, watch};
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
/// One 100 ms frame of a mic: 1600 samples of 16 kHz mono s16 — exactly what
/// the ASR gets (spec §5.4). Shared: every consumer reads the same buffer.
pub(crate) type MicFrame = Arc<[i16]>;
/// A consumer that falls this many frames (12.8 s) behind loses the oldest ones
/// and is told how many (`RecvError::Lagged`): no consumer can slow a chain.
/// tokio rounds a broadcast capacity up to a power of two, so this is one.
pub(crate) const FRAME_CHANNEL_CAPACITY: usize = 128;

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
    /// One frame channel per mic id, kept for the life of the process: every
    /// chain of a mic (a save that changes its stream starts a new one) sends
    /// into the same channel, so a consumer never re-subscribes (#852 C1b).
    frames: Mutex<HashMap<String, broadcast::Sender<MicFrame>>>,
    /// Where the recorder writes and what time it is (#852 C1b).
    recorder_env: Mutex<RecorderEnv>,
    /// The day the recorder records (`listen.recorder`); `None` = off.
    recorder: watch::Sender<Option<NaiveDate>>,
    /// The day the supervisor switched the recorder off because it lay after
    /// today: the state layer clears the stored row (#852 C1b-4).
    recorder_cleared: watch::Sender<Option<NaiveDate>>,
}

impl ListenService {
    pub(crate) fn new() -> Self {
        let (config, _) = watch::channel(Arc::new(Vec::new()));
        let (recorder, _) = watch::channel(None);
        let (recorder_cleared, _) = watch::channel(None);
        Self {
            inner: Arc::new(Inner {
                config,
                health: Mutex::new(ListenHealth::default()),
                started: AtomicBool::new(false),
                save_lock: tokio::sync::Mutex::new(()),
                frames: Mutex::new(HashMap::new()),
                recorder_env: Mutex::new(RecorderEnv::from_env()),
                recorder,
                recorder_cleared,
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

    /// Hand the stored recorder toggle to the recorder (`None` = off).
    pub(crate) fn apply_recorder(&self, enabled_for: Option<NaiveDate>) {
        let _previous = self.inner.recorder.send_replace(enabled_for);
    }

    /// Told each time the recorder was switched off for a day after today.
    pub(crate) fn recorder_cleared(&self) -> watch::Receiver<Option<NaiveDate>> {
        self.inner.recorder_cleared.subscribe()
    }

    /// Today's local date, as the recorder counts days.
    pub(crate) fn today(&self) -> NaiveDate {
        self.recorder_env().now().date()
    }

    /// `GET /listen/status`, ages measured now; the recorder records while its
    /// day is today.
    pub(crate) fn status(&self) -> ListenStatus {
        let mics = self.mics();
        let mut status = self.with_health(|health| health.snapshot(&mics, Instant::now()));
        let env = self.recorder_env();
        let enabled_for = *self.inner.recorder.borrow();
        status.recorder.enabled = RecorderSetting { enabled_for }.records_on(env.now().date());
        status.recorder.enabled_for = enabled_for;
        status.recorder.root = env.root.display().to_string();
        status
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

    /// The mic's frame channel, created on first use.
    fn frame_sender(&self, mic_id: &str) -> broadcast::Sender<MicFrame> {
        let mut frames = self
            .inner
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        frames
            .entry(mic_id.to_string())
            .or_insert_with(|| broadcast::Sender::new(FRAME_CHANNEL_CAPACITY))
            .clone()
    }

    /// A consumer's receiver of the mic's frames (the recorder; C3: the ASR).
    fn subscribe_frames(&self, mic_id: &str) -> broadcast::Receiver<MicFrame> {
        self.frame_sender(mic_id).subscribe()
    }

    fn recorder_receiver(&self) -> watch::Receiver<Option<NaiveDate>> {
        self.inner.recorder.subscribe()
    }

    /// The toggle names `day`, after today: switch it off in memory — unless a
    /// switch made since replaced it — and tell the state layer to clear the
    /// stored row. `true` when this call switched it off.
    fn switch_off_future_day(&self, day: NaiveDate) -> bool {
        let off = self.inner.recorder.send_if_modified(|stored| {
            let named = *stored == Some(day);
            if named {
                *stored = None;
            }
            named
        });
        if off {
            let _previous = self.inner.recorder_cleared.send_replace(Some(day));
        }
        off
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
