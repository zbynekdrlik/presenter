//! One microphone's chain (#852 C1a, spec §5.1 + §5.4). The receive loop hands
//! each decoded packet over as a [`Block`] through a bounded [`BlockQueue`]
//! and never waits: on a full queue the OLDEST block goes and is counted
//! (songplayer `vban-out.md` "The queue never blocks"). One task per mic
//! ([`run_chain`]) picks the channel, converts to 16 kHz mono
//! (`resample::Resampler16k`), cuts 100 ms frames and records how many and the
//! last one's level; it spends a unit of its coop budget per block, so a full
//! queue never pins a worker. A format change rebuilds the converter; its log
//! line goes through the #484 gate on the mic's format count (health). The
//! frames themselves reach no consumer yet: the channel arrives with C1b's
//! recorder / C3's ASR.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use presenter_core::audio::level::rms_dbfs;
use presenter_core::audio::vban::VbanAudioFormat;
use presenter_core::listen::mics::MicChannel;
use tokio::sync::Notify;
use tracing::{info, warn};

use super::resample::Resampler16k;
use super::{should_log_event, ListenService, MIC_TARGET};

/// ≈ 340 ms of 256-frame packets at 48 kHz — far above the ≥ 6 × 256
/// samples a Voicemeeter receiver keeps (songplayer `vban-out.md`).
pub(crate) const QUEUE_CAPACITY: usize = 64;

/// One packet's audio for the chain: `silence_frames` of inserted silence
/// (lost packets, spec §5.3) followed by the interleaved samples.
#[derive(Debug)]
pub(crate) struct Block {
    pub(crate) format: VbanAudioFormat,
    pub(crate) silence_frames: usize,
    pub(crate) samples: Vec<f32>,
}

/// A bounded single-consumer queue whose push never waits.
pub(crate) struct BlockQueue {
    state: Mutex<QueueState>,
    ready: Notify,
    capacity: usize,
}

#[derive(Default)]
struct QueueState {
    blocks: VecDeque<Block>,
    closed: bool,
}

impl BlockQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            ready: Notify::new(),
            capacity: capacity.max(1),
        }
    }

    /// Queue a block; `true` when the oldest one had to go.
    pub(crate) fn push(&self, block: Block) -> bool {
        let dropped = {
            let mut state = self.lock();
            let dropped = state.blocks.len() >= self.capacity && state.blocks.pop_front().is_some();
            state.blocks.push_back(block);
            dropped
        };
        self.ready.notify_one();
        dropped
    }

    /// The next block; `None` once the queue is closed and empty.
    pub(crate) async fn pop(&self) -> Option<Block> {
        loop {
            {
                let mut state = self.lock();
                if let Some(block) = state.blocks.pop_front() {
                    return Some(block);
                }
                if state.closed {
                    return None;
                }
            }
            // a push between the unlock and this wait left a permit: no lost wake-up
            self.ready.notified().await;
        }
    }

    /// No more blocks: the chain drains what is queued and ends.
    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_one();
    }

    /// `(silence_frames, samples)` of every queued block.
    #[cfg(test)]
    pub(crate) fn peek(&self) -> Vec<(usize, usize)> {
        self.lock()
            .blocks
            .iter()
            .map(|b| (b.silence_frames, b.samples.len()))
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The mic's channel of interleaved samples: `left` = channel 1, `right` =
/// channel 2 (channel 1 of a mono stream), `mix` = the mean of all channels.
pub(crate) fn pick_channel(interleaved: &[f32], channels: u16, pick: MicChannel) -> Vec<f32> {
    let width = usize::from(channels.max(1));
    interleaved
        .chunks_exact(width)
        .map(|frame| {
            let first = frame.first().copied().unwrap_or(0.0);
            match pick {
                MicChannel::Left => first,
                MicChannel::Right => frame.get(1).copied().unwrap_or(first),
                MicChannel::Mix => frame.iter().sum::<f32>() / width as f32,
            }
        })
        .collect()
}

/// Channel pick + converter of one mic; a new stream format starts a fresh
/// converter (spec §5.3 "reset the converter").
pub(crate) struct MicChain {
    channel: MicChannel,
    format: Option<VbanAudioFormat>,
    resampler: Option<Resampler16k>,
    error: Option<String>,
}

impl MicChain {
    pub(crate) fn new(channel: MicChannel) -> Self {
        Self {
            channel,
            format: None,
            resampler: None,
            error: None,
        }
    }

    /// One block → the 100 ms frames it completes (none while the format is refused).
    pub(crate) fn push(&mut self, block: &Block) -> Vec<Vec<i16>> {
        if self.format != Some(block.format) {
            self.reset(block.format);
        }
        let Some(resampler) = self.resampler.as_mut() else {
            return Vec::new();
        };
        let mut mono = vec![0.0; block.silence_frames];
        mono.extend(pick_channel(
            &block.samples,
            block.format.channels,
            self.channel,
        ));
        resampler.push(&mono)
    }

    pub(crate) fn format(&self) -> Option<VbanAudioFormat> {
        self.format
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn reset(&mut self, format: VbanAudioFormat) {
        self.format = Some(format);
        match Resampler16k::new(format.rate_hz) {
            Ok(resampler) => {
                self.resampler = Some(resampler);
                self.error = None;
            }
            Err(error) => {
                self.resampler = None;
                self.error = Some(error);
            }
        }
    }
}

/// The task of one mic: blocks in, frames counted with the last one's level
/// (only while `generation` is the mic's current one). Ends when the queue is
/// closed (the mic was dropped or changed, or the socket closed) and drained.
pub(crate) async fn run_chain(
    service: ListenService,
    mic_id: String,
    channel: MicChannel,
    generation: u64,
    queue: Arc<BlockQueue>,
) {
    let mut chain = MicChain::new(channel);
    while let Some(block) = queue.pop().await {
        let before = chain.format();
        let frames = chain.push(&block);
        if before != chain.format() {
            let formats = service.with_health(|health| health.record_format(&mic_id, generation));
            if should_log_event(formats) {
                log_format(&mic_id, before, block.format, chain.error(), formats);
            }
        }
        let level = frames.last().map(Vec::as_slice).map(rms_dbfs);
        let produced = frames.len() as u64;
        service.with_health(|health| {
            health.record_chain(&mic_id, generation, produced, level, chain.error());
        });
        // `pop` touches no tokio resource: while blocks are queued (a sender
        // flooding counter jumps queues a second of silence per packet) the
        // task must still give its worker back once its coop budget is spent.
        tokio::task::coop::consume_budget().await;
    }
}

/// The converter line of a format the chain switched to; `formats` (the
/// mic's formats this session, its first one included) passed the #484 gate.
fn log_format(
    mic_id: &str,
    before: Option<VbanAudioFormat>,
    after: VbanAudioFormat,
    error: Option<&str>,
    formats: u64,
) {
    if let Some(error) = error {
        warn!(
            target: MIC_TARGET,
            mic = mic_id,
            rate = after.rate_hz,
            error,
            formats,
            "mic converter refused the VBAN format — this mic gives no audio"
        );
    } else if let Some(before) = before {
        info!(
            target: MIC_TARGET,
            mic = mic_id,
            from = ?before,
            to = ?after,
            formats,
            "mic VBAN format changed — converter reset"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::health::PacketSeen;
    use crate::tracing_capture::Capture;
    use presenter_core::audio::sequence::SequenceVerdict;
    use presenter_core::audio::vban::VbanSampleType;
    use presenter_core::audio::vban_encode::tone;
    use presenter_core::listen::mics::{MicConfig, MicLanguage, MicRole};
    use std::time::Duration;
    use tokio::time::Instant;

    const MONO_48K: VbanAudioFormat = VbanAudioFormat {
        rate_hz: 48_000,
        channels: 1,
        sample_type: VbanSampleType::Int16,
    };
    /// Passed through without a converter: a cheap block of exactly one frame.
    const MONO_16K: VbanAudioFormat = VbanAudioFormat {
        rate_hz: 16_000,
        ..MONO_48K
    };
    const STEREO_48K: VbanAudioFormat = VbanAudioFormat {
        rate_hz: 48_000,
        channels: 2,
        sample_type: VbanSampleType::Int24,
    };

    fn block(format: VbanAudioFormat, silence_frames: usize, samples: Vec<f32>) -> Block {
        Block {
            format,
            silence_frames,
            samples,
        }
    }

    #[test]
    fn a_full_queue_drops_the_oldest_block() {
        let queue = BlockQueue::new(3);
        let dropped: Vec<bool> = (0..5)
            .map(|k| queue.push(block(MONO_48K, k, Vec::new())))
            .collect();
        assert_eq!(dropped, vec![false, false, false, true, true]);
        assert_eq!(queue.peek(), vec![(2, 0), (3, 0), (4, 0)]);
    }

    #[tokio::test]
    async fn pop_waits_for_a_push_and_close_ends_it() {
        let queue = Arc::new(BlockQueue::new(4));
        let popper = {
            let queue = Arc::clone(&queue);
            tokio::spawn(async move {
                let first = queue.pop().await.map(|b| b.silence_frames);
                let second = queue.pop().await.map(|b| b.silence_frames);
                (first, second)
            })
        };
        tokio::task::yield_now().await;
        queue.push(block(MONO_48K, 7, Vec::new()));
        queue.close();
        let popped = tokio::time::timeout(Duration::from_secs(5), popper)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            popped,
            (Some(7), None),
            "the last block first, then the end"
        );
    }

    #[test]
    fn the_channel_pick_follows_the_config() {
        let stereo = [0.25, 0.75, 0.5, 0.0];
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Left), vec![0.25, 0.5]);
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Right), vec![0.75, 0.0]);
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Mix), vec![0.5, 0.25]);
        for pick in [MicChannel::Left, MicChannel::Right, MicChannel::Mix] {
            assert_eq!(
                pick_channel(&[0.5, -0.5], 1, pick),
                vec![0.5, -0.5],
                "mono: {pick:?}"
            );
        }
        assert_eq!(
            pick_channel(&[0.25, 0.5, 0.75], 3, MicChannel::Mix),
            vec![0.5]
        );
    }

    #[test]
    fn the_chain_picks_the_configured_channel() {
        // left silent, right a 1 kHz tone at half scale; 2 × 100 ms (the
        // Python mirror of rubato's Fft: the second frame −9.03 dBFS)
        let stereo: Vec<f32> = tone(48_000, 1_000.0, 0.5, 0, 4_800, 1)
            .into_iter()
            .flat_map(|x| [0.0, x])
            .collect();
        for (pick, expected) in [(MicChannel::Right, -9.03), (MicChannel::Left, -120.0)] {
            let mut chain = MicChain::new(pick);
            let mut frames = chain.push(&block(STEREO_48K, 0, stereo.clone()));
            frames.extend(chain.push(&block(STEREO_48K, 0, stereo.clone())));
            assert_eq!(frames.len(), 2, "{pick:?}: 200 ms in, 2 frames out");
            let level = rms_dbfs(&frames[1]);
            assert!((level - expected).abs() <= 0.3, "{pick:?}: {level}");
        }
    }

    #[test]
    fn inserted_silence_is_part_of_the_output() {
        let mut chain = MicChain::new(MicChannel::Left);
        let mut frames = chain.push(&block(MONO_48K, 4_800, Vec::new()));
        for start in [0, 4_800] {
            let samples = tone(48_000, 1_000.0, 0.5, start, 4_800, 1);
            frames.extend(chain.push(&block(MONO_48K, 0, samples)));
        }
        assert_eq!(
            frames.len(),
            3,
            "100 ms of inserted silence + 200 ms of tone"
        );
        assert!(rms_dbfs(&frames[0]) < -60.0, "{}", rms_dbfs(&frames[0]));
        assert!(
            (rms_dbfs(&frames[2]) + 9.03).abs() <= 0.3,
            "{}",
            rms_dbfs(&frames[2])
        );
    }

    #[test]
    fn a_format_change_resets_the_converter_and_an_unsupported_rate_says_why() {
        let mut chain = MicChain::new(MicChannel::Left);
        assert_eq!(chain.push(&block(MONO_48K, 0, vec![0.0; 4_800])).len(), 1);
        let at_44k = VbanAudioFormat {
            rate_hz: 44_100,
            ..MONO_48K
        };
        assert_eq!(
            chain.push(&block(at_44k, 0, vec![0.0; 4_410])).len(),
            1,
            "a fresh converter"
        );
        assert_eq!((chain.format(), chain.error()), (Some(at_44k), None));
        let at_11k = VbanAudioFormat {
            rate_hz: 11_025,
            ..MONO_48K
        };
        assert!(chain.push(&block(at_11k, 0, vec![0.0; 1_102])).is_empty());
        assert_eq!(
            chain.error(),
            Some("nepodporovaná vzorkovacia frekvencia 11025 Hz")
        );
        assert_eq!(chain.push(&block(MONO_48K, 0, vec![0.0; 4_800])).len(), 1);
        assert_eq!(chain.error(), None, "a supported format clears the error");
    }

    #[tokio::test]
    async fn the_chain_task_counts_its_frames_and_keeps_the_last_level() {
        let service = ListenService::new();
        let generation = service.with_health(|health| health.set_mics(&[], &["guest"]));
        let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
        let task = tokio::spawn(run_chain(
            service.clone(),
            "guest".to_string(),
            MicChannel::Left,
            generation,
            Arc::clone(&queue),
        ));
        queue.push(block(MONO_48K, 0, tone(48_000, 1_000.0, 0.5, 0, 9_600, 1)));
        queue.close();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let now = Instant::now();
        let seen = PacketSeen {
            from: "192.0.2.10:6980".parse().unwrap(),
            format: MONO_48K,
            frames: 256,
            at: now,
        };
        service.with_health(|health| {
            health.record_packet("guest", seen, SequenceVerdict::InOrder, false);
        });
        let mic = &service
            .with_health(|health| health.snapshot(&[guest()], now))
            .mics[0];
        assert_eq!(mic.frames_out, 2, "200 ms in, 2 frames");
        let level = mic.level_dbfs.unwrap();
        assert!((level + 9.03).abs() <= 0.3, "{level}");
        assert!(mic.voiced);
    }

    #[tokio::test]
    async fn the_chain_task_gives_its_worker_back_while_blocks_are_queued() {
        // `pop` never waits while blocks are queued: without a yield point the
        // task would convert all of them before any other task on this
        // current-thread runtime got to run.
        const BLOCKS: u64 = 300;
        let service = ListenService::new();
        let generation = service.with_health(|health| health.set_mics(&[], &["guest"]));
        let queue = Arc::new(BlockQueue::new(512));
        for _ in 0..BLOCKS {
            queue.push(block(MONO_16K, 0, vec![0.0; 1_600]));
        }
        let chain = tokio::spawn(run_chain(
            service.clone(),
            "guest".to_string(),
            MicChannel::Left,
            generation,
            Arc::clone(&queue),
        ));
        let probe = {
            let service = service.clone();
            tokio::spawn(async move { frames_out(&service) })
        };
        let meanwhile = tokio::time::timeout(Duration::from_secs(5), probe)
            .await
            .unwrap()
            .unwrap();
        assert!(
            meanwhile < BLOCKS,
            "another task ran only after all {meanwhile} blocks"
        );
        queue.close();
        tokio::time::timeout(Duration::from_secs(5), chain)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            frames_out(&service),
            BLOCKS,
            "every block was still converted"
        );
    }

    #[tokio::test]
    async fn alternating_formats_log_the_converter_line_on_power_of_two_counts() {
        let capture = Capture::install();
        let service = ListenService::new();
        let generation = service.with_health(|health| health.set_mics(&[], &["guest"]));
        let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
        let at_44k = VbanAudioFormat {
            rate_hz: 44_100,
            ..MONO_48K
        };
        for k in 0..QUEUE_CAPACITY {
            let format = if k % 2 == 0 { MONO_48K } else { at_44k };
            queue.push(block(format, 0, Vec::new()));
        }
        queue.close();
        let chain = tokio::spawn(run_chain(
            service.clone(),
            "guest".to_string(),
            MicChannel::Left,
            generation,
            Arc::clone(&queue),
        ));
        tokio::time::timeout(Duration::from_secs(5), chain)
            .await
            .unwrap()
            .unwrap();
        let lines = capture.events_with_message("mic VBAN format changed");
        let formats: Vec<&str> = lines
            .iter()
            .filter_map(|line| line.field("formats"))
            .collect();
        assert_eq!(
            formats,
            vec!["2", "4", "8", "16", "32", "64"],
            "64 formats (63 changes): 6 lines, not 63"
        );
    }

    fn guest() -> MicConfig {
        MicConfig {
            id: "guest".into(),
            name: "Hosť".into(),
            stream_name: "GUEST_EN".into(),
            sender_ip: None,
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        }
    }

    fn frames_out(service: &ListenService) -> u64 {
        let status = service.with_health(|health| health.snapshot(&[guest()], Instant::now()));
        status.mics[0].frames_out
    }
}
