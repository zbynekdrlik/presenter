//! VBAN audio packets (#852 C1a, spec §5.2): the 28-byte header and the PCM
//! payload. Pure, no I/O.
//!
//! Ported from songplayer `crates/sp-server/src/playback/vban_packet.rs`
//! @ dd551b7d (the production sender to FOH; VB-Audio "VBAN Protocol
//! Specifications" rev. 13): the header layout, the constants, the SR indices
//! and format bits, [`f32_to_int16`] and [`clean_f32`]. Songplayer only SENDS;
//! the decode direction is new, from the spec's reference receive algorithm
//! (p. 10–11), and the sample decode is the sign extension of songplayer's
//! test oracle `parse_packet` without its `assert!`s: a datagram from the
//! network is untrusted, every reject is a typed [`VbanReject`] and nothing
//! here can panic.
//!
//! | bytes | field |
//! |---|---|
//! | 0..4 | `'V' 'B' 'A' 'N'` |
//! | 4 | `format_SR`: bits 0–4 SR index, 5–7 sub-protocol (AUDIO = 0x00) |
//! | 5 | `format_nbs` = frames − 1 |
//! | 6 | `format_nbc` = channels − 1 |
//! | 7 | `format_bit`: bits 0–2 data type, bit 3 reserved (0), 4–7 codec (PCM = 0x00) |
//! | 8..24 | `streamname[16]`, ASCII, zero padded |
//! | 24..28 | `nuFrame`, u32 LE, +1 per packet |
//!
//! The decode scale is songplayer's symmetric encoder scale (INT16 ±32767,
//! INT24 ±8388607, INT32 ±`i32::MAX`), clamped to ±1.0, so an INT16 or INT24
//! packet of the ported encoder round-trips bit-exactly in the integer domain.
//! INT32 is carried in an f32 (24-bit mantissa) and keeps only its top 24
//! bits: 2147483000 decodes to 2147483008 / 2^31 and re-encodes as 2147483007.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The four magic bytes every VBAN packet starts with.
pub const VBAN_MAGIC: [u8; 4] = *b"VBAN";
/// The VBAN header length.
pub const VBAN_HEADER_LEN: usize = 28;
/// The stream-name field length.
pub const VBAN_STREAM_NAME_LEN: usize = 16;
/// The spec's data maximum per packet (a conformant sender stays within it).
pub const VBAN_DATA_MAX: usize = 1436;
/// The VBAN default UDP port (spec §5.1).
pub const VBAN_DEFAULT_PORT: u16 = 6980;
/// Full scale of INT16: `±1.0 ↔ ±32767` (songplayer, symmetric).
pub const INT16_FULL_SCALE: i16 = 32_767;
/// Full scale of INT24: `±1.0 ↔ ±8388607` (songplayer, symmetric).
pub const INT24_FULL_SCALE: i32 = 8_388_607;

/// The spec's sample rates (rev. 13 p. 8), indexed by `format_SR & 0x1F`.
pub const VBAN_SAMPLE_RATES: [u32; 21] = [
    6_000, 12_000, 24_000, 48_000, 96_000, 192_000, 384_000, 8_000, 16_000, 32_000, 64_000,
    128_000, 256_000, 512_000, 11_025, 22_050, 44_100, 88_200, 176_400, 352_800, 705_600,
];

const PROTOCOL_MASK: u8 = 0xE0;
const PROTOCOL_AUDIO: u8 = 0x00;
const SR_INDEX_MASK: u8 = 0x1F;
const DATA_TYPE_MASK: u8 = 0x07;
const RESERVED_BIT: u8 = 0x08;
const CODEC_MASK: u8 = 0xF0;
const CODEC_PCM: u8 = 0x00;

/// The PCM sample types Presenter accepts (spec §5.2 step 6); BYTE8, FLOAT64
/// and the 12/10-bit types are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VbanSampleType {
    Int16,
    Int24,
    Int32,
    Float32,
}

impl VbanSampleType {
    /// The type of a `format_bit & 0x07` data-type index.
    pub fn from_data_type(index: u8) -> Option<Self> {
        match index {
            1 => Some(Self::Int16),
            2 => Some(Self::Int24),
            3 => Some(Self::Int32),
            4 => Some(Self::Float32),
            _ => None,
        }
    }

    /// The `format_bit` data-type index (codec PCM).
    pub fn data_type(self) -> u8 {
        match self {
            Self::Int16 => 1,
            Self::Int24 => 2,
            Self::Int32 => 3,
            Self::Float32 => 4,
        }
    }

    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        match self {
            Self::Int16 => 2,
            Self::Int24 => 3,
            Self::Int32 | Self::Float32 => 4,
        }
    }

    /// The wire name (`/listen/status` `sampleType`, log lines).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Int16 => "int16",
            Self::Int24 => "int24",
            Self::Int32 => "int32",
            Self::Float32 => "float32",
        }
    }

    /// One little-endian sample as ±1.0; `bytes` is exactly [`Self::bytes`]
    /// long (the caller's `chunks_exact`), anything else reads as silence.
    fn decode(self, bytes: &[u8]) -> f32 {
        let scaled = |value: f64, full_scale: f64| (value / full_scale).clamp(-1.0, 1.0) as f32;
        match (self, bytes) {
            (Self::Int16, [a, b]) => scaled(
                f64::from(i16::from_le_bytes([*a, *b])),
                f64::from(INT16_FULL_SCALE),
            ),
            (Self::Int24, [a, b, c]) => {
                let sign = if c & 0x80 != 0 { 0xFF } else { 0x00 };
                scaled(
                    f64::from(i32::from_le_bytes([*a, *b, *c, sign])),
                    f64::from(INT24_FULL_SCALE),
                )
            }
            (Self::Int32, [a, b, c, d]) => scaled(
                f64::from(i32::from_le_bytes([*a, *b, *c, *d])),
                f64::from(i32::MAX),
            ),
            (Self::Float32, [a, b, c, d]) => clean_f32(f32::from_le_bytes([*a, *b, *c, *d])),
            _ => 0.0,
        }
    }
}

/// What a converter must know about a stream. A change of any field resets
/// the mic's converter (spec §5.3); the frames per packet may vary freely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VbanAudioFormat {
    pub rate_hz: u32,
    pub channels: u16,
    pub sample_type: VbanSampleType,
}

/// One decoded VBAN audio packet (spec §5.2 step 8).
#[derive(Debug, Clone, PartialEq)]
pub struct VbanPacket {
    pub stream_name: String,
    pub counter: u32,
    pub format: VbanAudioFormat,
    /// Frames (one sample per channel): `format_nbs + 1`, 1–256.
    pub frames: u16,
    /// Interleaved samples (`frames × channels`), ±1.0.
    pub samples: Vec<f32>,
}

/// Why a datagram is not usable VBAN audio. Counted per sender in
/// `/listen/status`; the log text is English like every server log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum VbanReject {
    #[error("not a VBAN packet (shorter than the header or no VBAN magic)")]
    NotVban,
    #[error("not VBAN audio (sub-protocol {0:#04x})")]
    NonAudio(u8),
    #[error("unsupported VBAN codec {0:#04x}")]
    UnsupportedCodec(u8),
    #[error("unsupported VBAN sample type {0}")]
    UnsupportedType(u8),
    #[error("corrupt VBAN packet (reserved bit, SR index or size)")]
    Corrupt,
}

/// Decode one datagram with the spec's checks, in its order: magic + length,
/// AUDIO sub-protocol, PCM codec, reserved bit, SR index, sample type, size.
pub fn decode_packet(datagram: &[u8]) -> Result<VbanPacket, VbanReject> {
    let Some((header, payload)) = datagram.split_at_checked(VBAN_HEADER_LEN) else {
        return Err(VbanReject::NotVban);
    };
    if header[0..4] != VBAN_MAGIC {
        return Err(VbanReject::NotVban);
    }
    let (format_sr, nbs, nbc, format_bit) = (header[4], header[5], header[6], header[7]);
    let protocol = format_sr & PROTOCOL_MASK;
    if protocol != PROTOCOL_AUDIO {
        return Err(VbanReject::NonAudio(protocol));
    }
    let codec = format_bit & CODEC_MASK;
    if codec != CODEC_PCM {
        return Err(VbanReject::UnsupportedCodec(codec));
    }
    if format_bit & RESERVED_BIT != 0 {
        return Err(VbanReject::Corrupt);
    }
    let rate_hz = VBAN_SAMPLE_RATES
        .get(usize::from(format_sr & SR_INDEX_MASK))
        .copied()
        .ok_or(VbanReject::Corrupt)?;
    let data_type = format_bit & DATA_TYPE_MASK;
    let sample_type =
        VbanSampleType::from_data_type(data_type).ok_or(VbanReject::UnsupportedType(data_type))?;
    let frames = u16::from(nbs) + 1;
    let channels = u16::from(nbc) + 1;
    if payload.len() != usize::from(frames) * usize::from(channels) * sample_type.bytes() {
        return Err(VbanReject::Corrupt);
    }
    Ok(VbanPacket {
        stream_name: stream_name_of(&header[8..24]),
        counter: u32::from_le_bytes([header[24], header[25], header[26], header[27]]),
        format: VbanAudioFormat {
            rate_hz,
            channels,
            sample_type,
        },
        frames,
        samples: payload
            .chunks_exact(sample_type.bytes())
            .map(|bytes| sample_type.decode(bytes))
            .collect(),
    })
}

/// A header's stream name: up to the first zero byte; a byte that is not
/// printable ASCII reads as `_` (the substitution songplayer's
/// `stream_name_bytes` writes).
pub fn stream_name_of(field: &[u8]) -> String {
    field
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| {
            if (0x20..0x7F).contains(&byte) {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect()
}

/// One f32 sample as INT16 (songplayer, verbatim): clamped, scaled by
/// [`INT16_FULL_SCALE`], rounded half away from zero; NaN is silence.
pub fn f32_to_int16(x: f32) -> i16 {
    (f64::from(x).clamp(-1.0, 1.0) * f64::from(INT16_FULL_SCALE)).round() as i16
}

/// One f32 sample as VBAN FLOAT32 carries it (songplayer, verbatim): clamped
/// to ±1, a non-finite value as silence.
pub fn clean_f32(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
