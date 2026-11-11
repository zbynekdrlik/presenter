//! The real-time half of a Dante mic (#852): [`Tap`] is the `dlink::Process`
//! the device's media thread calls every block (~43 frames at 16 kHz on a
//! 48 kHz network, ~21 on a 96 kHz one — dlink's `Rated` converts on the
//! device clock). It must not lock, allocate, make syscalls or log (dstack
//! I7), so each RX channel goes into its own wait-free `rtrb` ring; a full
//! ring drops the samples that do not fit and counts them (an atomic). The
//! tokio side ([`TapOutputs::drain`]) takes whatever a ring holds and turns it
//! into one `mic_chain::Block` for the mic's chain.
//!
//! Lost audio: dstack's pump calls `discontinuity` whenever it skipped blocks
//! or started its schedule again (dlink's `Rated` passes it on), so that call
//! is the loss signal and is counted as a resync. Between two such calls the
//! media time runs on contiguously; a jump it shows anyway (a pump that did
//! not signal it) is handled by VBAN's counter rule (spec §5.3): up to 1 s is
//! written as silence, more is a resync.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dlink::{Block as DanteBlock, Process};
use rtrb::{Consumer, Producer, RingBuffer};

use super::DANTE_APP_RATE_HZ;

/// One second of 16 kHz per channel: the drain runs every 20 ms, so a ring
/// fills only when the tokio side stalls for a whole second.
pub(crate) const RING_SAMPLES: usize = DANTE_APP_RATE_HZ as usize;

/// What the media thread counted; the tokio side reads and logs it.
#[derive(Debug, Default)]
pub(crate) struct TapCounters {
    /// Samples a full ring could not take (every channel together).
    pub(crate) overflow_samples: AtomicU64,
    /// Skipped stretches ≤ 1 s, written as silence.
    pub(crate) gaps: AtomicU64,
    /// Jumps over 1 s (or backwards): nothing inserted.
    pub(crate) resyncs: AtomicU64,
}

/// What a block's media time says against the one expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Continuity {
    /// The first block, a block with no media time, or the expected one.
    InOrder,
    /// `n` frames were skipped: write `n` frames of silence first.
    Gap(u64),
    /// More than one second away, either way: start over from this block.
    Resync,
}

/// The block that starts at `start` after the one expected at `expected`, at
/// `rate` frames per second.
pub(crate) fn continuity(expected: Option<u64>, start: Option<u64>, rate: u32) -> Continuity {
    let (Some(expected), Some(start)) = (expected, start) else {
        return Continuity::InOrder;
    };
    match start.checked_sub(expected) {
        Some(0) => Continuity::InOrder,
        Some(skipped) if skipped <= u64::from(rate) => Continuity::Gap(skipped),
        _ => Continuity::Resync,
    }
}

/// The media thread's side: one producer per RX channel, in channel order.
pub(crate) struct Tap {
    producers: Vec<Producer<f32>>,
    counters: Arc<TapCounters>,
    next_start: Option<u64>,
}

/// The tokio side: one consumer per RX channel, in the same order.
pub(crate) struct TapOutputs {
    consumers: Vec<Consumer<f32>>,
    pub(crate) counters: Arc<TapCounters>,
}

/// A tap for `channels` RX channels and its outputs.
pub(crate) fn tap(channels: usize) -> (Tap, TapOutputs) {
    let counters = Arc::new(TapCounters::default());
    let (producers, consumers) = (0..channels)
        .map(|_| RingBuffer::<f32>::new(RING_SAMPLES))
        .unzip();
    (
        Tap {
            producers,
            counters: Arc::clone(&counters),
            next_start: None,
        },
        TapOutputs {
            consumers,
            counters,
        },
    )
}

impl Tap {
    /// Write `samples` (or `silence` zeros) into `ring`, as far as it has room;
    /// the samples that did not fit.
    fn write(ring: &mut Producer<f32>, samples: impl ExactSizeIterator<Item = f32>) -> u64 {
        let wanted = samples.len();
        let room = ring.slots().min(wanted);
        let mut written = 0;
        if let Ok(chunk) = ring.write_chunk_uninit(room) {
            written = chunk.fill_from_iter(samples);
        }
        (wanted - written) as u64
    }
}

impl Process for Tap {
    fn process(&mut self, block: &mut DanteBlock<'_>) {
        let frames = block.frames();
        let start = block.start();
        let silence = match continuity(self.next_start, start, DANTE_APP_RATE_HZ) {
            Continuity::InOrder => 0,
            Continuity::Gap(skipped) => {
                self.counters.gaps.fetch_add(1, Ordering::Relaxed);
                usize::try_from(skipped).unwrap_or(0)
            }
            Continuity::Resync => {
                self.counters.resyncs.fetch_add(1, Ordering::Relaxed);
                0
            }
        };
        self.next_start = start.map(|s| s + frames as u64);
        let mut lost = 0;
        for (channel, ring) in self.producers.iter_mut().enumerate() {
            if silence > 0 {
                lost += Self::write(ring, std::iter::repeat_n(0.0, silence));
            }
            let input = block.input(channel);
            lost += Self::write(ring, input.iter().map(|&x| x as f32));
        }
        if lost > 0 {
            self.counters
                .overflow_samples
                .fetch_add(lost, Ordering::Relaxed);
        }
    }

    fn discontinuity(&mut self) {
        self.next_start = None;
        self.counters.resyncs.fetch_add(1, Ordering::Relaxed);
    }
}

impl TapOutputs {
    /// Everything RX channel `channel`'s ring holds now (empty when none).
    pub(crate) fn drain(&mut self, channel: usize) -> Vec<f32> {
        let Some(ring) = self.consumers.get_mut(channel) else {
            return Vec::new();
        };
        let Ok(chunk) = ring.read_chunk(ring.slots()) else {
            return Vec::new();
        };
        let (first, second) = chunk.as_slices();
        let mut samples = Vec::with_capacity(first.len() + second.len());
        samples.extend_from_slice(first);
        samples.extend_from_slice(second);
        chunk.commit_all();
        samples
    }
}
