//! The virtual mic `pulpit` (#852 / #999, DESIGN 28.11, owner ruling the same
//! day): every handheld arrives post-fader on a fixed Dante channel (a muted
//! one is digital silence), and `pulpit` forwards exactly ONE of them at a
//! time — never a sum: a guest preacher and his live interpreter speak on two
//! handhelds, and a mix would blend both voices. The guests' translation
//! (`PRESENTER_LISTEN_GUEST_MIC`, default `pulpit`) reads it like any mic.
//!
//! [`choose`] is the pure policy: stay on the current mic while it is voiced
//! or within [`HANGOVER`] of its last voiced chunk (no switch mid-sentence);
//! once it has been quiet past that and another mic is voiced, switch to the
//! best candidate — a non-Slovak mic before an unknown one before a Slovak one
//! ([`SpokenLanguage`]: the Slovak one is the interpreter), then the loudest
//! recent level, then the channel order. A mic's language is the one its own
//! transcript shows (#852 T2: the dominant token language of its last final
//! words, `ListenService::spoken_languages`), else its configured language.
//!
//! [`supervise`] (started by `ListenService::start`) subscribes the 20 ms
//! chunk channel of every published Dante mic, judges each chunk the moment it
//! arrives (`rms_dbfs` > `VOICED_DBFS`, the one voicing rule) and forwards the
//! chosen mic's chunks to `pulpit`'s chunk channel; every 5 forwarded chunks
//! are `pulpit`'s 100 ms frame. A mic not forwarded keeps its last 2 s, so on a
//! switch its onset (from 200 ms before its first voiced chunk) goes out first
//! — an interpreter who starts during the preacher's hangover loses nothing. Each chunk is handled on its own arrival, so a
//! mic whose chunks stop (unpatched, lagging) never holds up another: its last
//! voiced chunk just ages until the policy leaves it. `pulpit` is not in the
//! mic list (no status row, no recording: the handhelds themselves are
//! recorded); a box without Dante mics has no `pulpit`.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::select_all;
use presenter_core::audio::level::{is_voiced, rms_dbfs, SILENCE_DBFS};
use presenter_core::listen::language::SpokenLanguage;
use presenter_core::listen::mics::{MicConfig, MicLanguage, PULPIT_MIC_ID};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::translate::session::HANGOVER;
use super::{should_log_event, ListenService, MicChunk, MicFrame, MIC_TARGET};

/// A mic whose last voiced chunk is this recent is a candidate (a few chunks:
/// one stray voiced chunk of a quiet mic ages out at once).
pub(crate) const CANDIDATE_WINDOW: Duration = Duration::from_millis(100);
/// The chunks a mic's "recent level" is the loudest of (200 ms).
pub(crate) const LEVEL_WINDOW_CHUNKS: usize = 10;
/// Chunks per frame (5 × 20 ms = 100 ms, the resampler's own cut).
const CHUNKS_PER_FRAME: usize = 5;
/// Switch lines logged per minute; the rest are counted into one summary.
pub(crate) const SWITCH_LINES_PER_MINUTE: u32 = 20;
const SWITCH_LOG_WINDOW: Duration = Duration::from_secs(60);
/// The chunks a mic not forwarded keeps (2 s = the hangover: the longest a
/// new speaker can wait for the switch).
pub(crate) const ONSET_BUFFER_CHUNKS: usize = 100;
/// The held chunks before a new mic's first voiced one sent with its onset
/// (200 ms: the start of the word below the voicing level).
pub(crate) const SWITCH_PRE_ROLL_CHUNKS: usize = 10;

type ConfigRx = watch::Receiver<Arc<Vec<MicConfig>>>;
type SpokenRx = watch::Receiver<BTreeMap<String, SpokenLanguage>>;

/// What the policy knows about one mic now.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MicView {
    /// Since its last voiced chunk; `None` = never voiced.
    pub(crate) voiced_ago: Option<Duration>,
    /// The loudest of its recent chunks.
    pub(crate) recent_dbfs: f32,
    pub(crate) language: Option<SpokenLanguage>,
}

/// Why `pulpit` moved to another mic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchReason {
    /// Nothing was forwarded yet.
    First,
    /// The forwarded mic was quiet past the hangover.
    CurrentQuiet,
}

impl SwitchReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::First => "first voiced mic",
            Self::CurrentQuiet => "the forwarded mic went quiet",
        }
    }
}

/// The policy's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Stay,
    Switch { to: usize, reason: SwitchReason },
}

/// The one rule (module doc). `current` indexes `mics`.
pub(crate) fn choose(current: Option<usize>, mics: &[MicView]) -> Decision {
    let current_mic = current.and_then(|index| mics.get(index));
    if current_mic
        .and_then(|mic| mic.voiced_ago)
        .is_some_and(|ago| ago <= HANGOVER)
    {
        return Decision::Stay;
    }
    let best = mics
        .iter()
        .enumerate()
        .filter(|(index, mic)| {
            Some(*index) != current && mic.voiced_ago.is_some_and(|ago| ago <= CANDIDATE_WINDOW)
        })
        .min_by(|(a_index, a), (b_index, b)| {
            language_rank(a.language)
                .cmp(&language_rank(b.language))
                .then(b.recent_dbfs.total_cmp(&a.recent_dbfs))
                .then(a_index.cmp(b_index))
        });
    match best {
        None => Decision::Stay,
        Some((to, _)) => Decision::Switch {
            to,
            reason: if current_mic.is_some() {
                SwitchReason::CurrentQuiet
            } else {
                SwitchReason::First
            },
        },
    }
}

/// Lower ranks first: the guest (foreign) before an unknown mic before the
/// Slovak interpreter.
fn language_rank(language: Option<SpokenLanguage>) -> u8 {
    match language {
        Some(SpokenLanguage::Foreign) => 0,
        None => 1,
        Some(SpokenLanguage::Slovak) => 2,
    }
}

/// One input mic's state.
#[derive(Debug)]
struct Input {
    id: String,
    last_voiced: Option<Instant>,
    levels: VecDeque<f32>,
    /// From the mic's configured language (the Dante mics: `off` = `None`).
    configured: Option<SpokenLanguage>,
    /// From its transcript (#852 T2); wins over `configured`.
    detected: Option<SpokenLanguage>,
    /// Its last [`ONSET_BUFFER_CHUNKS`] chunks while it is not forwarded, and
    /// whether each was voiced: a mic that starts while the forwarded one is
    /// still in its hangover loses nothing — its onset goes out on the switch.
    held: VecDeque<(MicChunk, bool)>,
}

impl Input {
    fn new(mic: &MicConfig) -> Self {
        Self {
            id: mic.id.clone(),
            last_voiced: None,
            levels: VecDeque::with_capacity(LEVEL_WINDOW_CHUNKS),
            configured: spoken_language(mic.language),
            detected: None,
            held: VecDeque::with_capacity(ONSET_BUFFER_CHUNKS),
        }
    }

    fn view(&self, now: Instant) -> MicView {
        MicView {
            voiced_ago: self.last_voiced.map(|at| now.saturating_duration_since(at)),
            recent_dbfs: self.levels.iter().copied().fold(SILENCE_DBFS, f32::max),
            language: self.detected.or(self.configured),
        }
    }

    /// The held chunks from [`SWITCH_PRE_ROLL_CHUNKS`] before the first voiced
    /// one on (the held quiet before it stays behind); the hold empties.
    fn take_onset(&mut self) -> Vec<MicChunk> {
        let first_voiced = self
            .held
            .iter()
            .position(|(_, voiced)| *voiced)
            .unwrap_or(self.held.len());
        let start = first_voiced.saturating_sub(SWITCH_PRE_ROLL_CHUNKS);
        self.held
            .drain(..)
            .skip(start)
            .map(|(chunk, _)| chunk)
            .collect()
    }
}

/// The policy's language of a mic's configured language (the fixed Dante
/// mics are `off` = unknown until their transcript shows one).
fn spoken_language(language: MicLanguage) -> Option<SpokenLanguage> {
    match language {
        MicLanguage::Sk => Some(SpokenLanguage::Slovak),
        MicLanguage::En => Some(SpokenLanguage::Foreign),
        MicLanguage::Off => None,
    }
}

/// The selector: input states, the forwarded mic and the frame being cut.
#[derive(Debug)]
pub(crate) struct Selector {
    inputs: Vec<Input>,
    current: Option<usize>,
    frame: Vec<i16>,
    frame_chunks: usize,
}

/// What one chunk produced.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Forwarded {
    /// `pulpit`'s chunks, in order: the arriving one when it is the forwarded
    /// mic's; after a switch first the new mic's held onset.
    pub(crate) chunks: Vec<MicChunk>,
    /// The frames those chunks completed. A frame cut across a switch holds
    /// both mics' chunks: the stream stays continuous.
    pub(crate) frames: Vec<MicFrame>,
    /// A switch this chunk caused: (from, to, reason).
    pub(crate) switch: Option<(Option<String>, String, SwitchReason)>,
}

impl Selector {
    /// One input per mic, in order (their ids and configured languages).
    pub(crate) fn new(mics: &[MicConfig]) -> Self {
        Self {
            inputs: mics.iter().map(Input::new).collect(),
            current: None,
            frame: Vec::with_capacity(CHUNKS_PER_FRAME * 320),
            frame_chunks: 0,
        }
    }

    /// The forwarded mic's id.
    pub(crate) fn current(&self) -> Option<&str> {
        self.current
            .and_then(|index| self.inputs.get(index))
            .map(|input| input.id.as_str())
    }

    /// Judge the chunk of input `index` arriving at `now`, decide, and
    /// forward what the (new) current mic has.
    pub(crate) fn push(&mut self, index: usize, chunk: MicChunk, now: Instant) -> Forwarded {
        let forwarded_now = self.current == Some(index);
        let Some(input) = self.inputs.get_mut(index) else {
            return Forwarded::default();
        };
        let level = rms_dbfs(&chunk);
        if input.levels.len() == LEVEL_WINDOW_CHUNKS {
            input.levels.pop_front();
        }
        input.levels.push_back(level);
        let voiced = is_voiced(level);
        if voiced {
            input.last_voiced = Some(now);
        }
        let mut out = Forwarded::default();
        if forwarded_now {
            out.chunks.push(chunk);
        } else {
            if input.held.len() == ONSET_BUFFER_CHUNKS {
                input.held.pop_front();
            }
            input.held.push_back((chunk, voiced));
        }
        let views: Vec<MicView> = self.inputs.iter().map(|input| input.view(now)).collect();
        if let Decision::Switch { to, reason } = choose(self.current, &views) {
            let from = self.current().map(str::to_string);
            self.current = Some(to);
            if let Some(input) = self.inputs.get_mut(to) {
                out.chunks.extend(input.take_onset());
                out.switch = Some((from, input.id.clone(), reason));
            }
        }
        for chunk in &out.chunks {
            self.frame.extend_from_slice(chunk);
            self.frame_chunks += 1;
            if self.frame_chunks == CHUNKS_PER_FRAME {
                out.frames.push(Arc::from(std::mem::take(&mut self.frame)));
                self.frame_chunks = 0;
            }
        }
        out
    }

    /// Every input's language from its transcript (`None` = not known, the
    /// configured one counts); `true` when one changed.
    pub(crate) fn set_detected(&mut self, spoken: &BTreeMap<String, SpokenLanguage>) -> bool {
        let mut changed = false;
        for input in &mut self.inputs {
            let detected = spoken.get(&input.id).copied();
            if input.detected != detected {
                input.detected = detected;
                changed = true;
            }
        }
        changed
    }
}

/// The switch lines' minute limiter.
#[derive(Debug)]
pub(crate) struct SwitchLog {
    window_start: Instant,
    lines: u32,
    suppressed: u64,
}

impl SwitchLog {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            lines: 0,
            suppressed: 0,
        }
    }

    /// Once the minute is over: start the next one and return how many
    /// switches the last one did not log (`None` = none or still running).
    pub(crate) fn roll(&mut self, now: Instant) -> Option<u64> {
        if now.saturating_duration_since(self.window_start) < SWITCH_LOG_WINDOW {
            return None;
        }
        let suppressed = std::mem::take(&mut self.suppressed);
        self.window_start = now;
        self.lines = 0;
        (suppressed > 0).then_some(suppressed)
    }

    /// `true` = log this switch (the minute's summary first, from [`Self::roll`]).
    pub(crate) fn admit(&mut self, now: Instant) -> (bool, Option<u64>) {
        let summary = self.roll(now);
        if self.lines < SWITCH_LINES_PER_MINUTE {
            self.lines += 1;
            (true, summary)
        } else {
            self.suppressed += 1;
            (false, summary)
        }
    }
}

/// The Dante mics of a list, in list (= channel) order.
fn input_mics(mics: &[MicConfig]) -> Vec<MicConfig> {
    mics.iter().filter(|mic| mic.is_dante()).cloned().collect()
}

/// The `pulpit` task: see the module.
pub(crate) async fn supervise(service: ListenService) {
    let mut config = service.config_receiver();
    loop {
        let mics = input_mics(&config.borrow_and_update());
        if mics.is_empty() {
            if config.changed().await.is_err() {
                return;
            }
            continue;
        }
        let ids: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, inputs = ?ids, "pulpit selects one of the Dante mics");
        let Some(last) = select(&service, &mut config, &mics).await else {
            return;
        };
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, last = last.as_deref(), "the Dante mics changed — pulpit restarts its selection");
    }
}

/// Forward from `mics` until the Dante mics change (`Some(the last forwarded
/// mic)`) or the service is gone (`None`).
async fn select(
    service: &ListenService,
    config: &mut ConfigRx,
    mics: &[MicConfig],
) -> Option<Option<String>> {
    let chunks_out = service.chunk_sender(PULPIT_MIC_ID);
    let frames_out = service.frame_sender(PULPIT_MIC_ID);
    let mut receivers: Vec<broadcast::Receiver<MicChunk>> = mics
        .iter()
        .map(|mic| service.subscribe_chunks(&mic.id))
        .collect();
    let mut spoken = service.spoken_languages();
    let mut selector = Selector::new(mics);
    let _changed = selector.set_detected(&spoken.borrow_and_update());
    let mut log = SwitchLog::new(Instant::now());
    let mut lagged = vec![0u64; mics.len()];
    let mut minute = interval(SWITCH_LOG_WINDOW);
    minute.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let next = select_all(
            receivers
                .iter_mut()
                .enumerate()
                .map(|(index, rx)| Box::pin(async move { (index, rx.recv().await) })),
        );
        let (index, received) = tokio::select! {
            changed = config.changed() => {
                changed.ok()?;
                // a VBAN save leaves the Dante mics and the selection as they are
                if input_mics(&config.borrow_and_update()) == mics {
                    continue;
                }
                return Some(selector.current().map(str::to_string));
            }
            changed = spoken.changed() => {
                // the sender lives in the service: never closed
                if changed.is_ok() {
                    follow_languages(&mut selector, &mut spoken);
                }
                continue;
            }
            _ = minute.tick() => {
                log_suppressed(log.roll(Instant::now()));
                continue;
            }
            ((index, received), _, _) = next => (index, received),
        };
        let chunk = match received {
            Ok(chunk) => chunk,
            Err(RecvError::Lagged(missed)) => {
                if let (Some(count), Some(mic)) = (lagged.get_mut(index), mics.get(index)) {
                    *count += 1;
                    if should_log_event(*count) {
                        warn!(target: MIC_TARGET, mic = PULPIT_MIC_ID, input = mic.id.as_str(), missed, events = *count, "pulpit fell behind a Dante mic — chunks skipped");
                    }
                }
                continue;
            }
            // a mic's channel lives as long as the service: not reached
            Err(RecvError::Closed) => return None,
        };
        let now = Instant::now();
        let out = selector.push(index, chunk, now);
        if let Some((from, to, reason)) = out.switch {
            log_switch(&mut log, now, from.as_deref(), &to, reason);
        }
        send_all(&chunks_out, out.chunks);
        send_all(&frames_out, out.frames);
    }
}

/// Take the inputs' new transcript languages (logged: the rank changed).
fn follow_languages(selector: &mut Selector, spoken: &mut SpokenRx) {
    let languages = spoken.borrow_and_update().clone();
    if selector.set_detected(&languages) {
        let ranked: Vec<(String, &str)> = languages
            .iter()
            .map(|(mic, language)| (mic.clone(), language.as_str()))
            .collect();
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, languages = ?ranked, "pulpit ranks the Dante mics by their transcript language");
    }
}

/// To every reader of `pulpit`; never waits.
fn send_all(out: &broadcast::Sender<Arc<[i16]>>, pieces: Vec<Arc<[i16]>>) {
    if out.receiver_count() == 0 {
        return;
    }
    for piece in pieces {
        let _receivers = out.send(piece);
    }
}

fn log_suppressed(summary: Option<u64>) {
    if let Some(suppressed) = summary {
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, suppressed, "pulpit switches in the last minute not logged one by one");
    }
}

fn log_switch(
    log: &mut SwitchLog,
    now: Instant,
    from: Option<&str>,
    to: &str,
    reason: SwitchReason,
) {
    let (admit, summary) = log.admit(now);
    log_suppressed(summary);
    if admit {
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, from, to, reason = reason.as_str(), "pulpit switched mic");
    }
}
