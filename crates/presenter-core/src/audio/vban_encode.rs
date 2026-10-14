//! The VBAN packet ENCODER, test-only (#852 C1a): the packet generator for the
//! decoder's round trips and for presenter-server's real-UDP receiver tests
//! (its dev-dependency enables `test-helpers`). Never in a production binary.
//!
//! Ported from songplayer `crates/sp-server/src/playback/vban_packet.rs`
//! @ dd551b7d: `stream_name_bytes`, `f32_to_int24`, `int24_le` (verbatim),
//! `sr_index_of` (widened from songplayer's 5 rates to the spec's list),
//! `write_header_as` (widened to any 1–256 frames × 1–256 channels: [`header`])
//! and `VbanEncoder`'s counter (+1 per packet, wrapping at `u32::MAX`).
//! Songplayer's 30 fps grid geometry and packet schedule are sender pacing and
//! are not ported. Written panic-free: the no-panic gate counts this file.

use super::vban::{
    clean_f32, f32_to_int16, VbanSampleType, INT24_FULL_SCALE, VBAN_HEADER_LEN, VBAN_MAGIC,
    VBAN_SAMPLE_RATES, VBAN_STREAM_NAME_LEN,
};

/// The stream-name field for `name` (songplayer, verbatim): its first 16
/// characters as ASCII (any non-ASCII or control character becomes `_`),
/// zero padded.
pub fn stream_name_bytes(name: &str) -> [u8; VBAN_STREAM_NAME_LEN] {
    let mut out = [0u8; VBAN_STREAM_NAME_LEN];
    for (dst, c) in out.iter_mut().zip(name.chars()) {
        *dst = if c.is_ascii() && !c.is_ascii_control() {
            c as u8
        } else {
            b'_'
        };
    }
    out
}

/// One f32 sample as a 24-bit integer (songplayer, verbatim): clamped to
/// `[-1.0, 1.0]`, scaled by [`INT24_FULL_SCALE`], rounded half away from
/// zero. NaN is silence (0, via Rust's saturating float→int cast).
pub fn f32_to_int24(x: f32) -> i32 {
    let clamped = f64::from(x).clamp(-1.0, 1.0);
    (clamped * f64::from(INT24_FULL_SCALE)).round() as i32
}

/// The 3 little-endian bytes of a 24-bit two's-complement sample (songplayer).
pub fn int24_le(v: i32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// One f32 sample as INT32: clamped, scaled by `i32::MAX`, rounded.
pub fn f32_to_int32(x: f32) -> i32 {
    (f64::from(x).clamp(-1.0, 1.0) * f64::from(i32::MAX)).round() as i32
}

/// The SR index of a rate (songplayer `sr_index_of`, now the full list).
pub fn sr_index_of(rate_hz: u32) -> Option<u8> {
    VBAN_SAMPLE_RATES
        .iter()
        .position(|&rate| rate == rate_hz)
        .and_then(|index| u8::try_from(index).ok())
}

/// The 28-byte header (songplayer `write_header_as`, any geometry); `None`
/// when it does not fit a header (0 or more than 256 frames or channels, a
/// rate outside the SR list).
pub fn header(
    rate_hz: u32,
    frames: usize,
    channels: usize,
    sample_type: VbanSampleType,
    name: &str,
    counter: u32,
) -> Option<[u8; VBAN_HEADER_LEN]> {
    let sr_index = sr_index_of(rate_hz)?;
    let nbs = u8::try_from(frames.checked_sub(1)?).ok()?;
    let nbc = u8::try_from(channels.checked_sub(1)?).ok()?;
    let mut out = [0u8; VBAN_HEADER_LEN];
    out[0..4].copy_from_slice(&VBAN_MAGIC);
    out[4] = sr_index;
    out[5] = nbs;
    out[6] = nbc;
    out[7] = sample_type.data_type();
    out[8..24].copy_from_slice(&stream_name_bytes(name));
    out[24..28].copy_from_slice(&counter.to_le_bytes());
    Some(out)
}

/// Append one sample in `sample_type` (songplayer `write_sample` + INT32).
pub fn write_sample(sample_type: VbanSampleType, x: f32, out: &mut Vec<u8>) {
    match sample_type {
        VbanSampleType::Int16 => out.extend_from_slice(&f32_to_int16(x).to_le_bytes()),
        VbanSampleType::Int24 => out.extend_from_slice(&int24_le(f32_to_int24(x))),
        VbanSampleType::Int32 => out.extend_from_slice(&f32_to_int32(x).to_le_bytes()),
        VbanSampleType::Float32 => out.extend_from_slice(&clean_f32(x).to_le_bytes()),
    }
}

/// The encoder of ONE stream (songplayer `VbanEncoder`): it owns the frame
/// counter, which grows by exactly 1 per packet and wraps at `u32::MAX`.
#[derive(Debug, Clone)]
pub struct VbanEncoder {
    name: String,
    rate_hz: u32,
    channels: usize,
    sample_type: VbanSampleType,
    counter: u32,
}

impl VbanEncoder {
    pub fn new(name: &str, rate_hz: u32, channels: usize, sample_type: VbanSampleType) -> Self {
        Self {
            name: name.to_string(),
            rate_hz,
            channels,
            sample_type,
            counter: 0,
        }
    }

    /// Start the counter at `counter` (the wrap and second-sender tests).
    pub fn starting_at(mut self, counter: u32) -> Self {
        self.counter = counter;
        self
    }

    /// The `nuFrame` the next packet carries.
    pub fn next_counter(&self) -> u32 {
        self.counter
    }

    /// Leave `n` packets out (a lost-packet test): the counter moves, nothing
    /// is sent.
    pub fn skip(&mut self, n: u32) {
        self.counter = self.counter.wrapping_add(n);
    }

    /// One packet of interleaved `samples` (1–256 whole frames); `None`, and
    /// no counter step, otherwise.
    pub fn packet(&mut self, samples: &[f32]) -> Option<Vec<u8>> {
        if self.channels == 0 || samples.len() % self.channels != 0 {
            return None;
        }
        let frames = samples.len() / self.channels;
        let head = header(
            self.rate_hz,
            frames,
            self.channels,
            self.sample_type,
            &self.name,
            self.counter,
        )?;
        let mut out =
            Vec::with_capacity(VBAN_HEADER_LEN + samples.len() * self.sample_type.bytes());
        out.extend_from_slice(&head);
        for &x in samples {
            write_sample(self.sample_type, x, &mut out);
        }
        self.counter = self.counter.wrapping_add(1);
        Some(out)
    }

    /// A block split into packets of `frames_per_packet` frames (the last may
    /// be shorter).
    pub fn packets(&mut self, samples: &[f32], frames_per_packet: usize) -> Vec<Vec<u8>> {
        let step = frames_per_packet.max(1) * self.channels.max(1);
        samples
            .chunks(step)
            .filter_map(|chunk| self.packet(chunk))
            .collect()
    }
}

/// `frames` interleaved frames of a sine from `start_frame` on, the same value
/// in every channel.
pub fn tone(
    rate_hz: u32,
    hz: f64,
    amplitude: f64,
    start_frame: usize,
    frames: usize,
    channels: usize,
) -> Vec<f32> {
    (start_frame..start_frame + frames)
        .flat_map(|n| {
            let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(rate_hz);
            std::iter::repeat_n((amplitude * phase.sin()) as f32, channels)
        })
        .collect()
}

/// songplayer's `ramp_block`, any length: distinct-ish values over `[-1, 1)`.
pub fn ramp(samples: usize) -> Vec<f32> {
    (0..samples)
        .map(|i| (i % 2000) as f32 / 1000.0 - 1.0)
        .collect()
}
