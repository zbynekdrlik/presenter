//! The receive side of the Dante mics (#852): one [`DanteMic`] per Dante mic,
//! in plan (= list) order, each with the `BlockQueue` of its chain. A mic whose
//! Dante channel stays keeps its chain, counters and generation across a save
//! (a rename or a language change resets nothing, VBAN's rule); another mic
//! starts fresh. A dropped mic closes its queue: its chain drains and ends.
//!
//! [`DanteInputs::drain`] never waits: it takes each ring's samples, and only
//! for a LIVE channel (its `written` count advanced within `LIVE_WINDOW`) hands
//! them to the chain as one block (a full queue drops its oldest, counted).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use presenter_core::listen::mics::{MicChannel, MicConfig, MicTransport};
use tokio::time::Instant;
use tracing::{info, warn};

use super::link::ChannelFlow;
use super::tap::TapOutputs;
use super::{DANTE_BLOCK_FORMAT, LIVE_WINDOW};
use crate::listen::mic_chain::{run_chain, Block, BlockQueue, QUEUE_CAPACITY};
use crate::listen::{should_log_event, ListenService, MIC_TARGET, NO_AUDIO_AFTER};

/// One Dante mic's receive side.
struct DanteMic {
    id: String,
    /// Its RX channel's name.
    channel: String,
    queue: Arc<BlockQueue>,
    /// The channel's `written` count at the last read; `None` until the
    /// current device's first read (a new device counts from zero).
    last_written: Option<u64>,
    /// When the count last advanced.
    last_advance: Option<Instant>,
    /// "receives" was logged and "lost audio" not yet.
    receiving: bool,
}

impl Drop for DanteMic {
    /// The chain drains its queue and ends.
    fn drop(&mut self) {
        self.queue.close();
    }
}

/// The device's loss counters already reported (the tap counts since start).
#[derive(Debug, Default)]
struct Reported {
    gaps: u64,
    resyncs: u64,
    overflow: u64,
    overflow_events: u64,
}

/// Every Dante mic's receive side and the current device's rings.
#[derive(Default)]
pub(super) struct DanteInputs {
    mics: Vec<DanteMic>,
    outputs: Option<TapOutputs>,
    reported: Reported,
}

impl DanteInputs {
    /// A saved list's Dante mics (in order): kept mics keep their chain,
    /// fresh ones start a chain counting with this update's generation.
    pub(super) fn update(&mut self, service: &ListenService, mics: &[MicConfig]) {
        let mut old: Vec<DanteMic> = std::mem::take(&mut self.mics);
        let channel_of = |mic: &MicConfig| mic.dante.as_ref().map(|d| d.channel.clone());
        let kept: Vec<&str> = mics
            .iter()
            .filter(|mic| {
                old.iter()
                    .any(|o| o.id == mic.id && Some(&o.channel) == channel_of(mic).as_ref())
            })
            .map(|mic| mic.id.as_str())
            .collect();
        let fresh: Vec<&str> = mics
            .iter()
            .map(|mic| mic.id.as_str())
            .filter(|id| !kept.contains(id))
            .collect();
        let generation = service
            .with_health(|health| health.set_transport_mics(MicTransport::Dante, &kept, &fresh));
        self.mics = mics
            .iter()
            .map(|mic| {
                let channel = channel_of(mic).unwrap_or_default();
                match old
                    .iter()
                    .position(|o| o.id == mic.id && o.channel == channel)
                {
                    Some(index) => old.swap_remove(index),
                    None => start_mic(service, &mic.id, channel, generation),
                }
            })
            .collect();
    }

    /// A new device's rings (in the same order as the mics).
    pub(super) fn attach(&mut self, outputs: TapOutputs) {
        self.outputs = Some(outputs);
        self.reported = Reported::default();
        for mic in &mut self.mics {
            mic.last_written = None;
            mic.last_advance = None;
            mic.receiving = false;
        }
    }

    /// Why the Dante mics give no audio (Slovak), or `None`.
    pub(super) fn set_error(&self, service: &ListenService, error: Option<&str>) {
        service.with_health(|health| {
            for mic in &self.mics {
                health.set_dante_error(&mic.id, error);
            }
        });
    }

    /// Every ring's samples: a live channel's go to its chain as one block.
    pub(super) fn drain(&mut self, service: &ListenService, flows: &[ChannelFlow], now: Instant) {
        let Some(outputs) = self.outputs.as_mut() else {
            return;
        };
        for (index, mic) in self.mics.iter_mut().enumerate() {
            let samples = outputs.drain(index);
            let flow = flows.get(index);
            if let Some(flow) = flow {
                if mic.last_written.is_some_and(|last| flow.written > last) {
                    mic.last_advance = Some(now);
                }
                mic.last_written = Some(flow.written);
            }
            let live = mic
                .last_advance
                .is_some_and(|at| now.saturating_duration_since(at) < LIVE_WINDOW);
            if !live || samples.is_empty() {
                continue;
            }
            if !mic.receiving {
                mic.receiving = true;
                info!(target: MIC_TARGET, mic = mic.id.as_str(), channel = mic.channel.as_str(), "mic receives Dante");
            }
            let frames = u16::try_from(samples.len()).unwrap_or(u16::MAX);
            let dropped = mic.queue.push(Block {
                format: DANTE_BLOCK_FORMAT,
                silence_frames: 0,
                samples,
            });
            let source = flow.and_then(|flow| flow.source.as_deref());
            service.with_health(|health| {
                health.record_dante_block(&mic.id, frames, source, dropped, now);
            });
        }
        self.report_losses(service);
    }

    /// Once a second: a receiving mic whose channel stopped advancing 3 s ago
    /// lost its audio.
    pub(super) fn on_tick(&mut self, now: Instant) {
        for mic in &mut self.mics {
            let stale = mic
                .last_advance
                .is_some_and(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
            if mic.receiving && stale {
                mic.receiving = false;
                info!(target: MIC_TARGET, mic = mic.id.as_str(), channel = mic.channel.as_str(), "mic lost audio — its Dante channel stopped for 3 s");
            }
        }
    }

    /// The tap's new gaps / resyncs / overflows: counted on every Dante mic,
    /// WARN through the #484 gate on the device's counts.
    fn report_losses(&mut self, service: &ListenService) {
        let Some(outputs) = self.outputs.as_ref() else {
            return;
        };
        let counters = &outputs.counters;
        let gaps = counters.gaps.load(Ordering::Relaxed);
        let resyncs = counters.resyncs.load(Ordering::Relaxed);
        let overflow = counters.overflow_samples.load(Ordering::Relaxed);
        for (now, before, gap) in [
            (gaps, &mut self.reported.gaps, true),
            (resyncs, &mut self.reported.resyncs, false),
        ] {
            while *before < now {
                *before += 1;
                service.with_health(|health| {
                    for mic in &self.mics {
                        health.record_dante_loss(&mic.id, gap);
                    }
                });
                if should_log_event(*before) {
                    warn!(target: MIC_TARGET, events = *before, gap, "the Dante device skipped audio (gap = silence inserted, else a resync)");
                }
            }
        }
        if overflow > self.reported.overflow {
            self.reported.overflow = overflow;
            self.reported.overflow_events += 1;
            if should_log_event(self.reported.overflow_events) {
                warn!(target: MIC_TARGET, samples = overflow, events = self.reported.overflow_events, "a Dante ring was full — samples dropped");
            }
        }
    }
}

/// A fresh Dante mic and its chain (one mono channel: `left`).
fn start_mic(service: &ListenService, id: &str, channel: String, generation: u64) -> DanteMic {
    let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
    tokio::spawn(run_chain(
        service.clone(),
        id.to_string(),
        MicChannel::Left,
        generation,
        Arc::clone(&queue),
    ));
    DanteMic {
        id: id.to_string(),
        channel,
        queue,
        last_written: None,
        last_advance: None,
        receiving: false,
    }
}
