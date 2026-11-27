//! The guests' live translation (#999, DESIGN D1): the preacher's Slovak,
//! spoken into each guest's own language by Gemini Live Translate and played
//! on the guest's phone (the root of the guest hostname, `GET /listen/guest/ws`).
//!
//! [`GuestTranslate`] keeps ONE [`session`] per ACTIVE language, shared by
//! every listener of it: the first listener starts it, it stops [`LINGER`]
//! after the last one left (a page reload does not restart it). At most
//! [`MAX_LANGUAGES`] run at once and the day's audio minutes are capped
//! ([`stats::DAILY_CAP_MINUTES`]): the cost scales with languages, never with
//! listeners. `config` = the env settings, `wire` = the Gemini JSON, `conn` =
//! one WebSocket connection; the languages and the guest wire are core's
//! `listen::guest`.

pub(crate) mod config;
mod conn;
pub(crate) mod session;
pub(crate) mod stats;
#[cfg(test)]
pub(crate) mod test_fake;
#[cfg(test)]
mod tests;
pub(crate) mod wire;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{broadcast, watch};

use self::config::GeminiEnv;
use self::session::{SessionDeps, SessionIo};
use self::stats::DayMeter;
use super::soniox::conn::TlsConnector;
use super::ListenService;

/// The target of every guest translation line.
pub(crate) const TARGET: &str = "presenter::listen::guest";
/// Languages translated at the same time (D1 cap).
pub(crate) const MAX_LANGUAGES: usize = 4;
/// A language's session runs on this long after its last listener left.
#[cfg(not(test))]
pub(crate) const LINGER: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const LINGER: Duration = Duration::from_secs(2);
/// Packets a listener may fall behind before it skips (≈ 30 s of audio).
const PACKET_CAPACITY: usize = 256;

pub(crate) use presenter_core::listen::guest::{
    GuestRefusal as Refusal, GuestSessionState as SessionState, GuestStopReason as StopReason,
};

/// What a session sends its listeners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuestPacket {
    /// Translated speech, s16le mono at 24 kHz.
    Audio(Arc<[u8]>),
    /// The translated text as it is spoken (a fragment to append).
    Text(String),
    /// The Slovak original (a fragment to append).
    Original(String),
    TurnComplete,
}

/// An offered language (core's `GUEST_LANGUAGES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Language {
    pub(crate) code: &'static str,
    /// The name in the language itself.
    pub(crate) name: &'static str,
}

impl Language {
    pub(crate) fn find(code: &str) -> Option<Self> {
        presenter_core::listen::guest::guest_language(code).map(|(code, name)| Self { code, name })
    }
}

/// One guest's subscription; dropping it counts the guest out.
pub(crate) struct Listener {
    pub(crate) language: Language,
    pub(crate) packets: broadcast::Receiver<GuestPacket>,
    pub(crate) state: watch::Receiver<SessionState>,
    _guard: ListenerGuard,
}

struct ListenerGuard {
    service: GuestTranslate,
    code: &'static str,
    generation: u64,
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        self.service.leave(self.code, self.generation);
    }
}

struct Entry {
    generation: u64,
    out: broadcast::Sender<GuestPacket>,
    state: watch::Receiver<SessionState>,
    stop: watch::Sender<bool>,
    listeners: Arc<AtomicUsize>,
}

/// The registry of language sessions (a cheap clone of one `Arc`).
#[derive(Clone)]
pub(crate) struct GuestTranslate {
    inner: Arc<Inner>,
}

struct Inner {
    deps: SessionDeps,
    sessions: Mutex<HashMap<&'static str, Entry>>,
    generations: AtomicUsize,
}

/// The status `GET /listen/guest/languages` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestStatus {
    pub(crate) available: bool,
    pub(crate) refusal: Option<Refusal>,
    pub(crate) active: Vec<&'static str>,
    pub(crate) max_languages: usize,
}

impl GuestTranslate {
    pub(crate) fn production() -> Self {
        Self::with_deps(SessionDeps {
            env: GeminiEnv::from_env(),
            connector: Arc::new(TlsConnector),
            meter: DayMeter::default(),
            today: local_today,
        })
    }

    /// The guests' own hostname (`PRESENTER_GUEST_HOST`), when this box has one.
    pub(crate) fn guest_host(&self) -> Option<&str> {
        self.inner.deps.env.guest_host.as_deref()
    }

    pub(crate) fn with_deps(deps: SessionDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                sessions: Mutex::new(HashMap::new()),
                generations: AtomicUsize::new(0),
            }),
        }
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<&'static str, Entry>> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Why nobody can listen on this box right now (`None` = they can).
    fn box_refusal(&self, listen: &ListenService) -> Option<Refusal> {
        let deps = &self.inner.deps;
        if deps.env.key.is_none() {
            return Some(Refusal::NoKey);
        }
        if !listen
            .mics()
            .iter()
            .any(|mic| mic.id == deps.env.source_mic)
        {
            return Some(Refusal::NoSource);
        }
        if deps.meter.exhausted((deps.today)()) {
            return Some(Refusal::DailyCap);
        }
        None
    }

    pub(crate) fn status(&self, listen: &ListenService) -> GuestStatus {
        let refusal = self.box_refusal(listen);
        let mut active: Vec<&'static str> = self.sessions().keys().copied().collect();
        active.sort_unstable();
        GuestStatus {
            available: refusal.is_none(),
            refusal,
            active,
            max_languages: MAX_LANGUAGES,
        }
    }

    /// Count a guest in for `code`, starting its session when it is the
    /// first one.
    pub(crate) fn join(&self, listen: &ListenService, code: &str) -> Result<Listener, Refusal> {
        let language = Language::find(code).ok_or(Refusal::UnknownLanguage)?;
        let mut sessions = self.sessions();
        if let Some(entry) = sessions.get(language.code) {
            let count = entry.listeners.fetch_add(1, Ordering::SeqCst) + 1;
            tracing::info!(target: TARGET, language = language.code, listeners = count, "guest joined a running translation");
            return Ok(self.listener(language, entry));
        }
        if let Some(refusal) = self.box_refusal(listen) {
            tracing::info!(target: TARGET, language = language.code, refusal = ?refusal, "guest refused");
            return Err(refusal);
        }
        if sessions.len() >= MAX_LANGUAGES {
            tracing::info!(target: TARGET, language = language.code, active = sessions.len(), "guest refused — the language cap is reached");
            return Err(Refusal::Full);
        }
        let generation = self.inner.generations.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        let (out, _) = broadcast::channel(PACKET_CAPACITY);
        let (state_tx, state) = watch::channel(SessionState::Connecting);
        let (stop, stop_rx) = watch::channel(false);
        let listeners = Arc::new(AtomicUsize::new(1));
        let io = SessionIo {
            frames: listen.subscribe_frames(&self.inner.deps.env.source_mic),
            out: out.clone(),
            state: state_tx,
            stop: stop_rx,
            listeners: Arc::clone(&listeners),
        };
        tokio::spawn(session::run(language, self.inner.deps.clone(), io));
        let entry = Entry {
            generation,
            out,
            state,
            stop,
            listeners,
        };
        let listener = self.listener(language, &entry);
        sessions.insert(language.code, entry);
        tracing::info!(target: TARGET, language = language.code, active = sessions.len(), "first guest — translation session starting");
        Ok(listener)
    }

    fn listener(&self, language: Language, entry: &Entry) -> Listener {
        Listener {
            language,
            packets: entry.out.subscribe(),
            state: entry.state.clone(),
            _guard: ListenerGuard {
                service: self.clone(),
                code: language.code,
                generation: entry.generation,
            },
        }
    }

    /// Count a guest out; the last one leaves the session [`LINGER`] to live.
    fn leave(&self, code: &'static str, generation: u64) {
        let sessions = self.sessions();
        let Some(entry) = sessions.get(code).filter(|e| e.generation == generation) else {
            return;
        };
        let left = entry
            .listeners
            .fetch_sub(1, Ordering::SeqCst)
            .saturating_sub(1);
        tracing::info!(target: TARGET, language = code, listeners = left, "guest left");
        if left == 0 {
            let service = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(LINGER).await;
                service.end_if_empty(code, generation);
            });
        }
    }

    fn end_if_empty(&self, code: &'static str, generation: u64) {
        let mut sessions = self.sessions();
        let empty = sessions
            .get(code)
            .is_some_and(|e| e.generation == generation && e.listeners.load(Ordering::SeqCst) == 0);
        if empty {
            if let Some(entry) = sessions.remove(code) {
                let _ = entry.stop.send(true);
                tracing::info!(target: TARGET, language = code, active = sessions.len(), linger_s = LINGER.as_secs(), "no guest left — translation session stopping");
            }
        }
    }
}

fn local_today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}
