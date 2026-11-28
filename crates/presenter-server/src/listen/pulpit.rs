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
//! ([`SpokenLanguage`]: the Slovak one is the interpreter; no mic carries a
//! label yet, the rule waits for language detection), then the loudest
//! recent level, then the channel order.
//!
//! [`supervise`] (started by `ListenService::start`) subscribes the 20 ms
//! chunk channel of every published Dante mic, judges each chunk the moment it
//! arrives (`rms_dbfs` > `VOICED_DBFS`, the one voicing rule) and forwards the
//! chosen mic's chunks to `pulpit`'s chunk channel; every 5 forwarded chunks
//! are `pulpit`'s 100 ms frame. Each chunk is handled on its own arrival, so a
//! mic whose chunks stop (unpatched, lagging) never holds up another: its last
//! voiced chunk just ages until the policy leaves it. `pulpit` is not in the
//! mic list (no status row, no recording: the handhelds themselves are
//! recorded); a box without Dante mics has no `pulpit`.

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::select_all;
use presenter_core::audio::level::{is_voiced, rms_dbfs, SILENCE_DBFS};
use presenter_core::listen::mics::{MicConfig, PULPIT_MIC_ID};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;
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

/// What a mic's speaker speaks, once something tells (language detection is
/// a later step; every mic is `None` today).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpokenLanguage {
    Slovak,
    /// Any other language: the guest preacher.
    Foreign,
}

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
    language: Option<SpokenLanguage>,
}

impl Input {
    fn new(id: String) -> Self {
        Self {
            id,
            last_voiced: None,
            levels: VecDeque::with_capacity(LEVEL_WINDOW_CHUNKS),
            language: None,
        }
    }

    fn view(&self, now: Instant) -> MicView {
        MicView {
            voiced_ago: self.last_voiced.map(|at| now.saturating_duration_since(at)),
            recent_dbfs: self.levels.iter().copied().fold(SILENCE_DBFS, f32::max),
            language: self.language,
        }
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
    /// `pulpit`'s chunk (the arriving one, when it is the forwarded mic's).
    pub(crate) chunk: Option<MicChunk>,
    /// A frame completed by that chunk.
    pub(crate) frame: Option<MicFrame>,
    /// A switch this chunk caused: (from, to, reason).
    pub(crate) switch: Option<(Option<String>, String, SwitchReason)>,
}

impl Selector {
    pub(crate) fn new(ids: &[String]) -> Self {
        Self {
            inputs: ids.iter().cloned().map(Input::new).collect(),
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
    /// forward it when it is the (new) current mic's.
    pub(crate) fn push(&mut self, index: usize, chunk: MicChunk, now: Instant) -> Forwarded {
        let Some(input) = self.inputs.get_mut(index) else {
            return Forwarded::default();
        };
        let level = rms_dbfs(&chunk);
        if input.levels.len() == LEVEL_WINDOW_CHUNKS {
            input.levels.pop_front();
        }
        input.levels.push_back(level);
        if is_voiced(level) {
            input.last_voiced = Some(now);
        }
        let views: Vec<MicView> = self.inputs.iter().map(|input| input.view(now)).collect();
        let mut out = Forwarded::default();
        if let Decision::Switch { to, reason } = choose(self.current, &views) {
            let from = self.current().map(str::to_string);
            self.current = Some(to);
            if let Some(input) = self.inputs.get(to) {
                out.switch = Some((from, input.id.clone(), reason));
            }
        }
        if self.current == Some(index) {
            self.frame.extend_from_slice(&chunk);
            self.frame_chunks += 1;
            if self.frame_chunks == CHUNKS_PER_FRAME {
                out.frame = Some(Arc::from(std::mem::take(&mut self.frame)));
                self.frame_chunks = 0;
            }
            out.chunk = Some(chunk);
        }
        out
    }

    /// Label a mic's language (a later step; tests supply it today).
    #[cfg(test)]
    pub(crate) fn set_language(&mut self, index: usize, language: Option<SpokenLanguage>) {
        if let Some(input) = self.inputs.get_mut(index) {
            input.language = language;
        }
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

    /// `true` = log this switch. A new minute first reports what the last
    /// one suppressed (returned as `Some(n)` in the tuple).
    pub(crate) fn admit(&mut self, now: Instant) -> (bool, Option<u64>) {
        let mut summary = None;
        if now.saturating_duration_since(self.window_start) >= Duration::from_secs(60) {
            if self.suppressed > 0 {
                summary = Some(self.suppressed);
            }
            self.window_start = now;
            self.lines = 0;
            self.suppressed = 0;
        }
        if self.lines < SWITCH_LINES_PER_MINUTE {
            self.lines += 1;
            (true, summary)
        } else {
            self.suppressed += 1;
            (false, summary)
        }
    }
}

/// The Dante mics of a list, by id, in list (= channel) order.
fn input_ids(mics: &[MicConfig]) -> Vec<String> {
    mics.iter()
        .filter(|mic| mic.is_dante())
        .map(|mic| mic.id.clone())
        .collect()
}

/// The `pulpit` task: see the module.
pub(crate) async fn supervise(service: ListenService) {
    let mut config = service.config_receiver();
    let chunks_out = service.chunk_sender(PULPIT_MIC_ID);
    let frames_out = service.frame_sender(PULPIT_MIC_ID);
    loop {
        let ids = input_ids(&config.borrow_and_update());
        if ids.is_empty() {
            if config.changed().await.is_err() {
                return;
            }
            continue;
        }
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, inputs = ?ids, "pulpit selects one of the Dante mics");
        let mut receivers: Vec<broadcast::Receiver<MicChunk>> =
            ids.iter().map(|id| service.subscribe_chunks(id)).collect();
        let mut selector = Selector::new(&ids);
        let mut log = SwitchLog::new(Instant::now());
        let mut lagged: u64 = 0;
        loop {
            let next = select_all(
                receivers
                    .iter_mut()
                    .enumerate()
                    .map(|(index, rx)| Box::pin(async move { (index, rx.recv().await) })),
            );
            let (index, received) = tokio::select! {
                changed = config.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // a VBAN save leaves the Dante mics and the selection as they are
                    if input_ids(&config.borrow_and_update()) == ids {
                        continue;
                    }
                    break;
                }
                ((index, received), _, _) = next => (index, received),
            };
            let chunk = match received {
                Ok(chunk) => chunk,
                Err(RecvError::Lagged(missed)) => {
                    lagged += 1;
                    if should_log_event(lagged) {
                        warn!(target: MIC_TARGET, mic = PULPIT_MIC_ID, input = ids[index].as_str(), missed, events = lagged, "pulpit fell behind a Dante mic — chunks skipped");
                    }
                    continue;
                }
                // a mic's channel lives as long as the service: not reached
                Err(RecvError::Closed) => return,
            };
            let now = Instant::now();
            let out = selector.push(index, chunk, now);
            if let Some((from, to, reason)) = out.switch {
                log_switch(&mut log, now, from.as_deref(), &to, reason);
            }
            if let Some(chunk) = out.chunk {
                if chunks_out.receiver_count() > 0 {
                    let _receivers = chunks_out.send(chunk);
                }
            }
            if let Some(frame) = out.frame {
                if frames_out.receiver_count() > 0 {
                    let _receivers = frames_out.send(frame);
                }
            }
        }
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, last = selector.current(), "the mic list changed — pulpit restarts its selection");
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
    if let Some(suppressed) = summary {
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, suppressed, "pulpit switches in the last minute not logged one by one");
    }
    if admit {
        info!(target: MIC_TARGET, mic = PULPIT_MIC_ID, from, to, reason = reason.as_str(), "pulpit switched mic");
    }
}
