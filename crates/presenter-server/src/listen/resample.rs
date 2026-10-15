//! A mic's converter to 16 kHz mono s16 (#852 C1a, spec §5.4).
//!
//! songplayer's converter pattern (`crates/sp-server/src/playback/vban_rate.rs`
//! @ dd551b7d): rubato's synchronous `Fft` with BOTH sides fixed, so every
//! input chunk gives an exact number of output frames; the converter and its
//! checks are built in their own fn ([`build_fft`] — cargo-mutants never
//! mutates a fn named `new`); a refusal is a reason string; a chunk rubato
//! refuses goes out as silence. Two deliberate differences:
//! - the chunk is 20 ms of input (`rate / 50` frames → 320 frames at 16 kHz),
//!   not one 100 ms block: with `FixedSync::Both` the FFT is the whole chunk
//!   and the delay half of it, so a 100 ms chunk would add 50 ms, and spec
//!   §8.5 budgets ≤ 120 ms for the 100 ms framing + the resampler (here
//!   100 + 10 ms). rubato's cut-off at a 320-frame block is 0.958 × 8 kHz ≈
//!   7.66 kHz (`calculate_cutoff`, BlackmanHarris2) — plenty for speech;
//! - mono in, mono out (`mic_chain` picks the channel first).
//!
//! 16 kHz passes through; every other rate of the VBAN list whose 20 ms is a
//! whole number of frames is converted (all but 11 025 Hz, refused). Output:
//! 100 ms frames of 1600 samples, `f32_to_int16` (songplayer's symmetric scale).

use presenter_core::audio::vban::f32_to_int16;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// The ASR input rate (spec §5.4: Soniox and Gemini Live take 16 kHz s16le).
const OUTPUT_RATE_HZ: u32 = 16_000;
/// One 100 ms frame at 16 kHz.
const FRAME_SAMPLES: usize = 1_600;
const CHUNKS_PER_SECOND: u32 = 50;
const CHUNK_OUT: usize = (OUTPUT_RATE_HZ / CHUNKS_PER_SECOND) as usize;

pub(crate) struct Resampler16k {
    /// `None` = a 16 kHz stream, passed through.
    fft: Option<Fft<f32>>,
    chunk_in: usize,
    pending_in: Vec<f32>,
    chunk_out: Vec<f32>,
    pending_out: Vec<i16>,
}

impl Resampler16k {
    pub(crate) fn new(rate_hz: u32) -> Result<Self, String> {
        Ok(Self {
            fft: build_fft(rate_hz)?,
            chunk_in: (rate_hz / CHUNKS_PER_SECOND) as usize,
            pending_in: Vec::new(),
            chunk_out: vec![0.0; CHUNK_OUT],
            pending_out: Vec::with_capacity(2 * FRAME_SAMPLES),
        })
    }

    /// Mono samples at the input rate in; every completed 100 ms frame out.
    pub(crate) fn push(&mut self, mono: &[f32]) -> Vec<Vec<i16>> {
        match self.fft.as_mut() {
            None => self
                .pending_out
                .extend(mono.iter().copied().map(f32_to_int16)),
            Some(fft) => {
                self.pending_in.extend_from_slice(mono);
                let mut used = 0;
                while self.pending_in.len() - used >= self.chunk_in {
                    let chunk = &self.pending_in[used..used + self.chunk_in];
                    if convert_chunk(fft, chunk, &mut self.chunk_out).is_some() {
                        self.pending_out
                            .extend(self.chunk_out.iter().copied().map(f32_to_int16));
                    } else {
                        let silent = self.pending_out.len() + CHUNK_OUT;
                        self.pending_out.resize(silent, 0);
                    }
                    used += self.chunk_in;
                }
                self.pending_in.drain(..used);
            }
        }
        self.take_frames()
    }

    /// The converter's delay in 16 kHz samples (0 when passing through).
    #[cfg(test)]
    pub(crate) fn delay_frames(&self) -> usize {
        self.fft.as_ref().map_or(0, |fft| fft.output_delay())
    }

    fn take_frames(&mut self) -> Vec<Vec<i16>> {
        let whole = self.pending_out.len() / FRAME_SAMPLES;
        (0..whole)
            .map(|_| self.pending_out.drain(..FRAME_SAMPLES).collect())
            .collect()
    }
}

/// rubato's converter for `rate_hz` → 16 kHz in 20 ms chunks (`None`: a
/// 16 kHz stream passes through), or why not.
fn build_fft(rate_hz: u32) -> Result<Option<Fft<f32>>, String> {
    if rate_hz == OUTPUT_RATE_HZ {
        return Ok(None);
    }
    let refused = format!("nepodporovaná vzorkovacia frekvencia {rate_hz} Hz");
    if rate_hz == 0 || rate_hz % CHUNKS_PER_SECOND != 0 {
        return Err(refused);
    }
    let chunk_in = (rate_hz / CHUNKS_PER_SECOND) as usize;
    let fft = Fft::<f32>::new(
        rate_hz as usize,
        OUTPUT_RATE_HZ as usize,
        chunk_in,
        1,
        FixedSync::Both,
    )
    .map_err(|err| format!("{refused}: {err}"))?;
    if fft.input_frames_next() == chunk_in && fft.output_frames_next() == CHUNK_OUT {
        Ok(Some(fft))
    } else {
        Err(refused)
    }
}

/// One chunk through rubato (songplayer's adapter calls); `None` = refused.
fn convert_chunk(fft: &mut Fft<f32>, input: &[f32], out: &mut [f32]) -> Option<()> {
    let frames_out = out.len();
    let adapter_in = InterleavedSlice::new(input, 1, input.len()).ok()?;
    let mut adapter_out = InterleavedSlice::new_mut(out, 1, frames_out).ok()?;
    fft.process_into_buffer(&adapter_in, &mut adapter_out, None)
        .ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::audio::vban::VBAN_SAMPLE_RATES;

    /// `seconds` of a mono sine at `rate_hz`.
    fn sine(rate_hz: u32, hz: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate_hz) * seconds) as usize;
        (0..n)
            .map(|k| {
                let phase = std::f64::consts::TAU * hz * k as f64 / f64::from(rate_hz);
                (amplitude * phase.sin()) as f32
            })
            .collect()
    }

    /// Fed in pieces of 256 frames (one VBAN packet's worth).
    fn convert(resampler: &mut Resampler16k, input: &[f32]) -> Vec<Vec<i16>> {
        input
            .chunks(256)
            .flat_map(|piece| resampler.push(piece))
            .collect()
    }

    #[test]
    fn a_16k_stream_passes_through_unconverted() {
        let mut resampler = Resampler16k::new(16_000).unwrap();
        let frames = resampler.push(&[0.5; 3_200]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().flatten().all(|&s| s == 16_384));
        assert_eq!(resampler.delay_frames(), 0);
    }

    #[test]
    fn every_supported_rate_gives_exact_100_ms_frames() {
        // songplayer `every_other_rate_gives_exactly_rate_over_30_frames_per_block`,
        // for every rate of the VBAN list (the plan's Python mirror of rubato's
        // sizing rule checks each: rate/50 in, 320 out, delay 160).
        let rates = VBAN_SAMPLE_RATES
            .iter()
            .filter(|&&rate| rate != 11_025 && rate != 16_000);
        for &rate in rates {
            let mut resampler =
                Resampler16k::new(rate).unwrap_or_else(|err| panic!("{rate}: {err}"));
            let frames = convert(&mut resampler, &sine(rate, 440.0, 0.5, 1.0));
            assert_eq!(frames.len(), 10, "{rate} Hz: 1 s in = 10 frames out");
            assert!(
                frames.iter().all(|frame| frame.len() == FRAME_SAMPLES),
                "{rate}"
            );
            assert_eq!(
                resampler.delay_frames(),
                160,
                "{rate}: half of the 320-frame FFT"
            );
        }
    }

    #[test]
    fn eleven_kilohertz_and_zero_are_refused_with_a_slovak_reason() {
        let Err(err) = Resampler16k::new(11_025) else {
            panic!("11025 Hz has no whole 20 ms chunk");
        };
        assert_eq!(err, "nepodporovaná vzorkovacia frekvencia 11025 Hz");
        assert!(Resampler16k::new(0).is_err());
    }

    #[test]
    fn a_1khz_tone_stays_1khz_from_48k_to_16k() {
        // songplayer `a_1khz_tone_stays_1khz_at_96k`, here 48 → 16 kHz (the
        // Python mirror of rubato's Fft: 899 crossings, peak 16384).
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let out: Vec<i16> = convert(&mut resampler, &sine(48_000, 1_000.0, 0.5, 1.0)).concat();
        let steady = &out[1_600..];
        assert_eq!(steady.len(), 14_400, "0.9 s past the first frame");
        let rising = steady.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        assert!(
            (898..=902).contains(&rising),
            "{rising} rising zero crossings in 0.9 s"
        );
        let peak = steady.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(
            (14_745..=18_022).contains(&peak),
            "the level is kept: {peak}"
        );
    }

    #[test]
    fn the_delay_is_a_constant_160_samples() {
        // A Gaussian pulse (σ = 0.5 ms, far inside the 7.66 kHz pass band) at
        // t = 0.25 s comes out at 0.25 s × 16 kHz + 160 = sample 4160.
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let pulse: Vec<f32> = (0..24_000)
            .map(|k| {
                let t = (f64::from(k) / 48_000.0 - 0.25) / 0.000_5;
                (0.8 * (-0.5 * t * t).exp()) as f32
            })
            .collect();
        let out: Vec<i16> = convert(&mut resampler, &pulse).concat();
        let peak_at = out
            .iter()
            .enumerate()
            .max_by_key(|(_, sample)| sample.unsigned_abs())
            .map(|(index, _)| index)
            .unwrap();
        assert!(
            (4_159..=4_161).contains(&peak_at),
            "pulse peak at {peak_at}, expected 4160"
        );
        assert_eq!(resampler.delay_frames(), 160);
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut resampler = Resampler16k::new(44_100).unwrap();
        let frames = convert(&mut resampler, &[0.0; 22_050]);
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().flatten().all(|&s| s == 0));
    }

    #[test]
    fn odd_piece_sizes_still_give_whole_frames() {
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let input = sine(48_000, 440.0, 0.5, 0.5);
        let frames: Vec<Vec<i16>> = input
            .chunks(97)
            .flat_map(|piece| resampler.push(piece))
            .collect();
        assert_eq!(frames.len(), 5);
    }
}
