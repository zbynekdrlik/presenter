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
//! The four fixed Dante mics (#852, #933, `dante`) arrive through Presenter's
//! own in-process Dante device (dstack's `dlink`) and feed the same chain:
//! the mic list every consumer reads ([`ListenService::mics`]) is the saved
//! VBAN list followed by those four, whenever the box names a Dante interface.
//! The virtual mic `pulpit` ([`pulpit`], #852 28.11) forwards ONE of them at a
//! time (the voiced handheld) on its own chunk / frame channels; it is a
//! source ([`ListenService::has_source`]) but not in the mic list.
//! The guest's transcript (#852 C3-6) lives in the [`transcript_hub`] the
//! service owns; `GET /listen/ws` serves it. The guest's transcription
//! (#852 C3-7, [`transcription`]): one supervisor decides whether the guest
//! mic's Soniox session runs; its rows, meter and state live in the service.
//! The transcript's 7-day store (#852 C3-persist, [`transcript_store`]): one
//! writer fed by the hub, seeded into the hub at startup.

mod asr;
mod dante;
pub(crate) mod health;
mod mic_chain;
mod minute_summary;
mod pulpit;
pub(crate) mod recorder;
mod recording_file;
mod resample;
mod soniox;
#[cfg(test)]
mod tests;
pub(crate) mod transcript_hub;
pub(crate) mod transcript_store;
pub(crate) mod transcription;
pub(crate) mod translate;
mod vban_rx;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::NaiveDate;
use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
use presenter_core::listen::mics::{fixed_dante_mics, MicConfig, PULPIT_MIC_ID};
use presenter_core::listen::recorder::RecorderSetting;
use presenter_core::listen::status::ListenStatus;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use self::health::ListenHealth;
use self::recorder::RecorderEnv;
use self::soniox::config::SonioxEnv;
use self::soniox::session::SonioxAsr;
use self::transcript_hub::TranscriptHub;
use self::transcription::{Transcription, TranscriptionDeps, TranscriptionStore};
use self::translate::GuestTranslate;

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
/// One 20 ms chunk of a mic: 320 samples of 16 kHz mono s16, the same stream
/// as its frames, sent the moment it is complete (#999 L6e: the guests'
/// translation sends these, so an onset never waits for a 100 ms frame).
pub(crate) type MicChunk = Arc<[i16]>;
/// The chunk channel's capacity: 512 chunks (10.24 s, a power of two).
pub(crate) const CHUNK_CHANNEL_CAPACITY: usize = 512;

/// The listen state every `AppState` clone shares.
#[derive(Clone)]
pub(crate) struct ListenService {
    inner: Arc<Inner>,
}

struct Inner {
    /// The published mic list: [`Mics::published`].
    config: watch::Sender<Arc<Vec<MicConfig>>>,
    /// Its two sources, changed together under this lock.
    sources: Mutex<Mics>,
    health: Mutex<ListenHealth>,
    started: AtomicBool,
    save_lock: tokio::sync::Mutex<()>,
    /// One frame channel per mic id, kept for the life of the process: every
    /// chain of a mic (a save that changes its stream starts a new one) sends
    /// into the same channel, so a consumer never re-subscribes (#852 C1b).
    frames: Mutex<HashMap<String, broadcast::Sender<MicFrame>>>,
    /// The same per mic for the 20 ms chunks (#999 L6e), kept the same way.
    chunks: Mutex<HashMap<String, broadcast::Sender<MicChunk>>>,
    /// Where the recorder writes and what time it is (#852 C1b).
    recorder_env: Mutex<RecorderEnv>,
    /// The day the recorder records (`listen.recorder`); `None` = off.
    recorder: watch::Sender<Option<NaiveDate>>,
    /// The day the supervisor switched the recorder off because it lay after
    /// today: the state layer clears the stored row (#852 C1b-5).
    recorder_cleared: watch::Sender<Option<NaiveDate>>,
    /// The guest's transcript `GET /listen/ws` serves (#852 C3-6).
    transcripts: TranscriptHub,
    /// The Soniox env settings, read once (#852 C3-3).
    soniox: SonioxEnv,
    /// The guest's transcription rows, meter and state (#852 C3-7).
    transcription: Transcription,
    transcription_started: AtomicBool,
    /// The guests' live translation sessions (#999).
    guest: Mutex<GuestTranslate>,
}

/// Where the published mic list comes from (#933).
#[derive(Default)]
struct Mics {
    /// The saved list (`listen.mics`): VBAN mics only.
    saved: Vec<MicConfig>,
    /// The fixed Dante mics; empty on a box without a Dante interface.
    dante: Vec<MicConfig>,
}

impl Mics {
    /// The saved mics, then the fixed Dante mics.
    fn published(&self) -> Vec<MicConfig> {
        self.saved.iter().chain(&self.dante).cloned().collect()
    }
}

impl ListenService {
    pub(crate) fn new() -> Self {
        let (config, _) = watch::channel(Arc::new(Vec::new()));
        let (recorder, _) = watch::channel(None);
        let (recorder_cleared, _) = watch::channel(None);
        let soniox = SonioxEnv::from_env();
        let transcription = Transcription::new(&soniox);
        Self {
            inner: Arc::new(Inner {
                config,
                sources: Mutex::new(Mics::default()),
                health: Mutex::new(ListenHealth::default()),
                started: AtomicBool::new(false),
                save_lock: tokio::sync::Mutex::new(()),
                frames: Mutex::new(HashMap::new()),
                chunks: Mutex::new(HashMap::new()),
                recorder_env: Mutex::new(RecorderEnv::from_env()),
                recorder,
                recorder_cleared,
                transcripts: TranscriptHub::new(),
                soniox,
                transcription,
                transcription_started: AtomicBool::new(false),
                guest: Mutex::new(GuestTranslate::production()),
            }),
        }
    }

    /// The mics the receivers use now: the saved ones, then the fixed Dante
    /// mics (#933).
    pub(crate) fn mics(&self) -> Arc<Vec<MicConfig>> {
        Arc::clone(&self.inner.config.borrow())
    }

    /// Hand a validated (VBAN) list to the receivers; they update their mics
    /// at once. A Dante mic in it is never taken: the Dante mics are fixed.
    pub(crate) fn apply(&self, mics: Vec<MicConfig>) {
        self.change_mics(|sources| {
            sources.saved = mics.into_iter().filter(|mic| !mic.is_dante()).collect();
        });
    }

    /// Publish the four fixed Dante mics (`true`: the box names a Dante
    /// interface) or none (#933: independent of the saved list).
    pub(crate) fn set_dante_mics(&self, on: bool) {
        self.change_mics(|sources| {
            sources.dante = if on { fixed_dante_mics() } else { Vec::new() };
        });
    }

    /// Change a source and publish the list, both under the sources' lock, so
    /// two changes can never publish out of order.
    fn change_mics(&self, change: impl FnOnce(&mut Mics)) {
        let mut sources = self
            .inner
            .sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        change(&mut sources);
        let _previous = self
            .inner
            .config
            .send_replace(Arc::new(sources.published()));
    }

    /// Whether a consumer can read `mic_id`'s audio here: a published mic, or
    /// `pulpit` while a Dante mic is published (#852 28.11).
    pub(crate) fn has_source(&self, mic_id: &str) -> bool {
        let mics = self.mics();
        if mic_id == PULPIT_MIC_ID {
            return mics.iter().any(MicConfig::is_dante);
        }
        mics.iter().any(|mic| mic.id == mic_id)
    }

    /// The transcript hub (#852 C3-6): the same hub on every call.
    pub(crate) fn transcripts(&self) -> TranscriptHub {
        self.inner.transcripts.clone()
    }

    /// The guests' live translation (#999): the same registry on every call.
    pub(crate) fn guest(&self) -> GuestTranslate {
        self.inner
            .guest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Point the guest translation at a test fake.
    #[cfg(test)]
    pub(crate) fn set_guest(&self, guest: GuestTranslate) {
        *self
            .inner
            .guest
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = guest;
    }

    /// The guest's transcription rows, meter and state (#852 C3-7).
    pub(crate) fn transcription(&self) -> &Transcription {
        &self.inner.transcription
    }

    /// Start the transcription supervisor once with the real Soniox provider
    /// (never in validate mode); `false` when it was already started.
    pub(crate) fn start_transcription(
        &self,
        layout: Arc<tokio::sync::RwLock<String>>,
        store: Arc<dyn TranscriptionStore>,
    ) -> bool {
        let env = self.inner.soniox.clone();
        self.start_transcription_with(TranscriptionDeps {
            key: env.key.is_some(),
            model: env.model.clone(),
            asr: Arc::new(SonioxAsr::production(env)),
            layout,
            store,
        })
    }

    /// After a rows write: wait until the supervisor decided on it (no wait
    /// when no supervisor runs — validate mode, a test that never started it).
    pub(crate) async fn transcription_decided_on(&self, generation: u64) {
        if self.inner.transcription_started.load(Ordering::SeqCst) {
            self.inner.transcription.decided_on(generation).await;
        }
    }

    /// Start the supervisor once with `deps` (tests: a fake provider).
    pub(crate) fn start_transcription_with(&self, deps: TranscriptionDeps) -> bool {
        if self
            .inner
            .transcription_started
            .swap(true, Ordering::SeqCst)
        {
            return false;
        }
        tokio::spawn(transcription::supervise(self.clone(), deps));
        true
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
        status.transcription = self.inner.transcription.status(env.now().date());
        status
    }

    /// Start the socket task, the Dante device's supervisor (#933: the device
    /// with its four fixed channels whenever the box names a Dante interface),
    /// the `pulpit` selector and the recorder's supervisor once; `false` when they were already
    /// started.
    pub(crate) fn start(&self, bind: SocketAddr) -> bool {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.with_health(|health| health.socket.bind_addr = Some(bind));
        tokio::spawn(vban_rx::supervise(self.clone(), bind));
        tokio::spawn(dante::supervise(
            self.clone(),
            Arc::new(dante::DlinkBackend),
            dante::DanteEnv::from_env(),
        ));
        tokio::spawn(pulpit::supervise(self.clone()));
        tokio::spawn(recorder::supervise(self.clone()));
        tokio::spawn(minute_summary::supervise(self.clone()));
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
    pub(crate) fn frame_sender(&self, mic_id: &str) -> broadcast::Sender<MicFrame> {
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

    /// The mic's 20 ms chunk channel, created on first use (#999 L6e).
    pub(crate) fn chunk_sender(&self, mic_id: &str) -> broadcast::Sender<MicChunk> {
        let mut chunks = self
            .inner
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        chunks
            .entry(mic_id.to_string())
            .or_insert_with(|| broadcast::Sender::new(CHUNK_CHANNEL_CAPACITY))
            .clone()
    }

    /// A consumer's receiver of the mic's 20 ms chunks (the guests' translation).
    fn subscribe_chunks(&self, mic_id: &str) -> broadcast::Receiver<MicChunk> {
        self.chunk_sender(mic_id).subscribe()
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
