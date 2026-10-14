//! #852 C1a: the VBAN decoder against songplayer's pinned encoder bytes
//! (`vban_packet_tests.rs` / `vban_packet_tests_format.rs` @ dd551b7d) and
//! the spec's receive checks (rev. 13 p. 10–11).

use super::VbanSampleType::{Float32, Int16, Int24, Int32};
use super::*;
use crate::audio::vban_encode::{
    f32_to_int24, f32_to_int32, header, int24_le, ramp, sr_index_of, stream_name_bytes, VbanEncoder,
};

/// INT16 mono 48 kHz, 4 frames [0.0, 0.5, -0.25, 1.5], stream GUEST_EN, counter
/// 0x01020304 — the same packet `scripts/dev/listen/test_vban_send.py` pins,
/// so the Python box sender and this decoder cannot drift apart.
const GOLDEN: &str = concat!(
    "5642414e",         // VBAN
    "03030001",         // SR index 3 (48 kHz), 4 frames, 1 channel, INT16 PCM
    "47554553545f454e", // "GUEST_EN"
    "0000000000000000", // name padding
    "04030201",         // counter 0x01020304
    "0000004000e0ff7f", // 0, 16384, -8192, 32767
);

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn golden_with(index: usize, value: u8) -> Vec<u8> {
    let mut bytes = hex(GOLDEN);
    bytes[index] = value;
    bytes
}

#[test]
fn golden_packet_decodes_and_the_encoder_rebuilds_it() {
    let packet = decode_packet(&hex(GOLDEN)).unwrap();
    assert_eq!(packet.stream_name, "GUEST_EN");
    assert_eq!(packet.counter, 0x0102_0304);
    assert_eq!(
        packet.format,
        VbanAudioFormat {
            rate_hz: 48_000,
            channels: 1,
            sample_type: Int16
        }
    );
    assert_eq!(packet.frames, 4);
    let ints: Vec<i16> = packet.samples.iter().map(|&x| f32_to_int16(x)).collect();
    assert_eq!(ints, vec![0, 16_384, -8_192, 32_767]);
    let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, Int16).starting_at(0x0102_0304);
    assert_eq!(
        encoder.packet(&[0.0, 0.5, -0.25, 1.5]).unwrap(),
        hex(GOLDEN)
    );
    assert_eq!(encoder.next_counter(), 0x0102_0305);
}

#[test]
fn the_header_is_the_vban_spec_layout_little_endian() {
    // songplayer vban_packet_tests.rs, the same expected bytes.
    let h = header(48_000, 200, 2, Int24, "sp-program", 0x0102_0304).unwrap();
    let expected: [u8; 28] = [
        b'V', b'B', b'A', b'N', //
        0x03, // SR index 3 = 48 kHz, sub protocol AUDIO (0x00)
        199,  // 200 samples − 1
        1,    // 2 channels − 1
        0x02, // INT24, reserved bit 0, codec PCM (0x00)
        b's', b'p', b'-', b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0, 0, 0, 0, 0, 0, //
        0x04, 0x03, 0x02, 0x01, // nuFrame, little-endian
    ];
    assert_eq!(h, expected);
    assert_eq!(header(48_000, 0, 1, Int16, "x", 0), None, "0 frames");
    assert_eq!(header(48_000, 257, 1, Int16, "x", 0), None, "257 frames");
    assert_eq!(header(48_000, 1, 257, Int16, "x", 0), None, "257 channels");
    assert_eq!(
        header(44_000, 1, 1, Int16, "x", 0),
        None,
        "not in the SR list"
    );
}

#[test]
fn the_stream_name_is_16_ascii_bytes_zero_padded() {
    // songplayer, verbatim:
    assert_eq!(&stream_name_bytes("sp-program"), b"sp-program\0\0\0\0\0\0");
    assert_eq!(
        &stream_name_bytes("a-very-long-stream-name"),
        b"a-very-long-stre",
        "truncated to 16"
    );
    assert_eq!(
        &stream_name_bytes("é\tx"),
        b"__x\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "non-ASCII and control characters become _"
    );
    assert_eq!(stream_name_bytes(""), [0u8; 16]);
    // the decode side: up to the first zero byte, the same substitution.
    assert_eq!(stream_name_of(b"GUEST_EN\0\0\0\0\0\0\0\0"), "GUEST_EN");
    assert_eq!(stream_name_of(b"a-very-long-stre"), "a-very-long-stre");
    assert_eq!(stream_name_of(b"ab\0cd\0\0\0\0\0\0\0\0\0\0\0"), "ab");
    assert_eq!(
        stream_name_of(&[0xC3, 0xA9, b'x', 0x7F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        "__x_"
    );
}

#[test]
fn int24_conversion_is_bit_exact_and_clamps() {
    // songplayer, verbatim.
    assert_eq!(f32_to_int24(0.0), 0);
    assert_eq!(f32_to_int24(1.0), 8_388_607, "+1.0 = +full scale");
    assert_eq!(f32_to_int24(-1.0), -8_388_607, "-1.0 = -full scale");
    assert_eq!(f32_to_int24(1.5), 8_388_607, "> 1.0 clamps");
    assert_eq!(f32_to_int24(-2.0), -8_388_607, "< -1.0 clamps");
    assert_eq!(f32_to_int24(f32::INFINITY), 8_388_607);
    assert_eq!(f32_to_int24(f32::NEG_INFINITY), -8_388_607);
    assert_eq!(f32_to_int24(f32::NAN), 0, "NaN is silence");
    assert_eq!(f32_to_int24(0.5), 4_194_304, "4194303.5 rounds away from 0");
    assert_eq!(f32_to_int24(-0.5), -4_194_304);
    assert_eq!(f32_to_int24(0.25), 2_097_152);
    assert_eq!(f32_to_int24(-0.25), -2_097_152);
    assert_eq!(f32_to_int24(1.0 / 8_388_607.0), 1, "one LSB");

    assert_eq!(int24_le(8_388_607), [0xFF, 0xFF, 0x7F]);
    assert_eq!(int24_le(-8_388_607), [0x01, 0x00, 0x80]);
    assert_eq!(int24_le(-1), [0xFF, 0xFF, 0xFF]);
    assert_eq!(int24_le(0x12_3456), [0x56, 0x34, 0x12]);
    assert_eq!(int24_le(f32_to_int24(1.0)), [0xFF, 0xFF, 0x7F]);
    assert_eq!(int24_le(f32_to_int24(-1.0)), [0x01, 0x00, 0x80]);
}

#[test]
fn int16_and_float32_samples_are_exact() {
    // songplayer vban_packet_tests_format.rs, verbatim.
    assert_eq!(f32_to_int16(1.0), 32_767);
    assert_eq!(f32_to_int16(-1.0), -32_767);
    assert_eq!(f32_to_int16(0.5), 16_384, "16383.5 rounds away from zero");
    assert_eq!(f32_to_int16(-0.25), -8_192);
    assert_eq!(f32_to_int16(2.0), 32_767);
    assert_eq!(f32_to_int16(-2.0), -32_767);
    assert_eq!(f32_to_int16(f32::NAN), 0);
    assert_eq!(clean_f32(1.5), 1.0);
    assert_eq!(clean_f32(-1.5), -1.0);
    assert_eq!(clean_f32(-0.25), -0.25);
    assert_eq!(clean_f32(f32::NAN), 0.0);
    assert_eq!(clean_f32(f32::INFINITY), 0.0);
    assert_eq!(clean_f32(f32::NEG_INFINITY), 0.0);
}

#[test]
fn every_index_of_the_sr_list_decodes_to_its_rate() {
    // songplayer's pinned indices, now from the full rev. 13 list:
    assert_eq!(sr_index_of(48_000), Some(3));
    assert_eq!(sr_index_of(96_000), Some(4));
    assert_eq!(sr_index_of(192_000), Some(5));
    assert_eq!(sr_index_of(44_100), Some(16));
    assert_eq!(sr_index_of(88_200), Some(17));
    assert_eq!(sr_index_of(16_000), Some(8));
    assert_eq!(sr_index_of(11_025), Some(14));
    assert_eq!(sr_index_of(44_000), None);
    for (index, &rate) in VBAN_SAMPLE_RATES.iter().enumerate() {
        let bytes = golden_with(4, index as u8);
        assert_eq!(
            decode_packet(&bytes).unwrap().format.rate_hz,
            rate,
            "index {index}"
        );
    }
    // songplayer's format bits (INT16 0x01, INT24 0x02, FLOAT32 0x04) + INT32:
    let types = [Int16, Int24, Int32, Float32];
    assert_eq!(types.map(VbanSampleType::data_type), [1, 2, 3, 4]);
    assert_eq!(types.map(VbanSampleType::bytes), [2, 3, 4, 4]);
    assert_eq!(VbanSampleType::from_data_type(3), Some(Int32));
    assert_eq!(VbanSampleType::from_data_type(0), None, "BYTE8");
}

struct Geometry {
    rate: u32,
    sample: VbanSampleType,
    frames: usize,
    packets: usize,
    len: usize,
}

const fn g(
    rate: u32,
    sample: VbanSampleType,
    frames: usize,
    packets: usize,
    len: usize,
) -> Geometry {
    Geometry {
        rate,
        sample,
        frames,
        packets,
        len,
    }
}

/// songplayer `vban_packet_tests_format.rs` `TABLE`: the packet geometry of one
/// 1/30 s stereo block per rate × format (frames per packet, packets, bytes).
const TABLE: [Geometry; 15] = [
    g(44_100, Int16, 245, 6, 1008),
    g(44_100, Int24, 210, 7, 1288),
    g(44_100, Float32, 147, 10, 1204),
    g(48_000, Int16, 200, 8, 828),
    g(48_000, Int24, 200, 8, 1228),
    g(48_000, Float32, 160, 10, 1308),
    g(88_200, Int16, 245, 12, 1008),
    g(88_200, Int24, 210, 14, 1288),
    g(88_200, Float32, 147, 20, 1204),
    g(96_000, Int16, 200, 16, 828),
    g(96_000, Int24, 200, 16, 1228),
    g(96_000, Float32, 160, 20, 1308),
    g(192_000, Int16, 256, 25, 1052),
    g(192_000, Int24, 200, 32, 1228),
    g(192_000, Float32, 160, 40, 1308),
];

#[test]
fn songplayers_15_geometries_round_trip_through_the_decoder() {
    for t in TABLE {
        let block = ramp(t.rate as usize / 30 * 2);
        let packets = VbanEncoder::new("sp-program", t.rate, 2, t.sample).packets(&block, t.frames);
        assert_eq!(packets.len(), t.packets, "{} {:?}", t.rate, t.sample);
        let mut decoded = Vec::new();
        for (k, bytes) in packets.iter().enumerate() {
            assert_eq!(bytes.len(), t.len, "{} {:?}", t.rate, t.sample);
            let p = decode_packet(bytes).unwrap();
            assert_eq!(
                p.format,
                VbanAudioFormat {
                    rate_hz: t.rate,
                    channels: 2,
                    sample_type: t.sample
                }
            );
            assert_eq!(usize::from(p.frames), t.frames);
            assert_eq!(p.counter, k as u32);
            decoded.extend(p.samples);
        }
        assert_eq!(decoded.len(), block.len());
        for (i, (&back, &x)) in decoded.iter().zip(&block).enumerate() {
            let same = match t.sample {
                Int16 => f32_to_int16(back) == f32_to_int16(x),
                Int24 => f32_to_int24(back) == f32_to_int24(x),
                Int32 | Float32 => back == clean_f32(x),
            };
            assert!(same, "{} {:?} sample {i}: {back} vs {x}", t.rate, t.sample);
        }
    }
}

#[test]
fn mono_and_stereo_int16_and_int32_round_trip() {
    for channels in [1usize, 2] {
        for sample in [Int16, Int32] {
            let block = ramp(256 * channels);
            let bytes = VbanEncoder::new("GUEST_EN", 48_000, channels, sample)
                .packet(&block)
                .unwrap();
            assert_eq!(
                bytes.len(),
                VBAN_HEADER_LEN + 256 * channels * sample.bytes()
            );
            let p = decode_packet(&bytes).unwrap();
            assert_eq!((usize::from(p.format.channels), p.frames), (channels, 256));
            for (&back, &x) in p.samples.iter().zip(&block) {
                match sample {
                    Int16 => assert_eq!(f32_to_int16(back), f32_to_int16(x)),
                    _ => assert!((back - x).abs() <= 1.0e-6, "{back} vs {x}"),
                }
            }
        }
    }
    assert_eq!(f32_to_int32(1.0), i32::MAX);
    assert_eq!(f32_to_int32(-1.0), -i32::MAX);
}

#[test]
fn int24_samples_sign_extend_like_songplayers_oracle() {
    let mut bytes = header(48_000, 5, 1, Int24, "x", 0).unwrap().to_vec();
    for v in [8_388_607, -8_388_607, -1, 0x12_3456] {
        bytes.extend_from_slice(&int24_le(v));
    }
    bytes.extend_from_slice(&[0x00, 0x00, 0x80]); // -8388608, one past full scale
    let p = decode_packet(&bytes).unwrap();
    let back: Vec<i32> = p.samples.iter().map(|&x| f32_to_int24(x)).collect();
    assert_eq!(back, vec![8_388_607, -8_388_607, -1, 0x12_3456, -8_388_607]);
    assert_eq!(p.samples[0], 1.0);
    assert_eq!(p.samples[4], -1.0, "clamped");
}

#[test]
fn every_reject_has_its_type() {
    let golden = hex(GOLDEN);
    let cases = [
        (Vec::new(), VbanReject::NotVban),
        (golden[..27].to_vec(), VbanReject::NotVban),
        (golden_with(3, b'X'), VbanReject::NotVban),
        (golden_with(4, 0x20 | 3), VbanReject::NonAudio(0x20)), // SERIAL
        (golden_with(4, 0x40 | 3), VbanReject::NonAudio(0x40)), // TXT (VB-Matrix)
        (golden_with(4, 0x60 | 3), VbanReject::NonAudio(0x60)), // SERVICE
        (golden_with(4, 0xE0 | 3), VbanReject::NonAudio(0xE0)), // USER
        (golden_with(7, 0x10 | 1), VbanReject::UnsupportedCodec(0x10)), // VBCA
        (golden_with(7, 0x20 | 1), VbanReject::UnsupportedCodec(0x20)), // VBCV
        (golden_with(7, 0xF0 | 1), VbanReject::UnsupportedCodec(0xF0)), // USER
        (golden_with(7, 0x08 | 1), VbanReject::Corrupt),        // reserved bit
        (golden_with(4, 21), VbanReject::Corrupt),              // SR index 21
        (golden_with(4, 31), VbanReject::Corrupt),
        (golden_with(7, 0x00), VbanReject::UnsupportedType(0)), // BYTE8
        (golden_with(7, 0x05), VbanReject::UnsupportedType(5)), // FLOAT64
        (golden_with(7, 0x06), VbanReject::UnsupportedType(6)), // 12 bits
        (golden_with(7, 0x07), VbanReject::UnsupportedType(7)), // 10 bits
        (golden[..golden.len() - 1].to_vec(), VbanReject::Corrupt),
        (
            [golden.as_slice(), &[0u8][..]].concat(),
            VbanReject::Corrupt,
        ),
        (golden_with(5, 4), VbanReject::Corrupt), // 5 frames announced, 4 sent
        (golden_with(6, 1), VbanReject::Corrupt), // 2 channels announced, 1 sent
    ];
    for (bytes, expected) in cases {
        assert_eq!(decode_packet(&bytes), Err(expected), "{bytes:02x?}");
    }
}

#[test]
fn the_checks_run_in_the_spec_order() {
    let mut bytes = hex(GOLDEN);
    bytes[4] = 0x40 | 31; // non-audio AND an SR index past the list
    bytes[7] = 0x18; // VBCA AND the reserved bit
    bytes.push(0); // AND one byte too many
    assert_eq!(decode_packet(&bytes), Err(VbanReject::NonAudio(0x40)));
    bytes[4] = 31;
    assert_eq!(
        decode_packet(&bytes),
        Err(VbanReject::UnsupportedCodec(0x10))
    );
    bytes[7] = 0x09;
    assert_eq!(
        decode_packet(&bytes),
        Err(VbanReject::Corrupt),
        "reserved bit"
    );
    bytes[7] = 0x01;
    assert_eq!(
        decode_packet(&bytes),
        Err(VbanReject::Corrupt),
        "SR index 31"
    );
    bytes[4] = 3;
    bytes[7] = 0x05;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::UnsupportedType(5)));
    bytes[7] = 0x01;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::Corrupt), "size last");
}

#[test]
fn the_frame_counter_wraps_at_u32_max() {
    let mut encoder = VbanEncoder::new("x", 48_000, 1, Int16).starting_at(u32::MAX - 1);
    let counters: Vec<u32> = (0..4)
        .map(|_| {
            decode_packet(&encoder.packet(&[0.0; 8]).unwrap())
                .unwrap()
                .counter
        })
        .collect();
    assert_eq!(counters, vec![u32::MAX - 1, u32::MAX, 0, 1]);
    assert_eq!(encoder.next_counter(), 2);
    encoder.skip(3);
    assert_eq!(encoder.next_counter(), 5);
    let mut stereo = VbanEncoder::new("x", 48_000, 2, Int16);
    assert_eq!(stereo.packet(&[0.0; 3]), None, "not whole frames");
    assert_eq!(stereo.next_counter(), 0, "a refused packet does not count");
}

#[test]
fn the_format_serializes_for_the_status_endpoint() {
    let format = VbanAudioFormat {
        rate_hz: 48_000,
        channels: 2,
        sample_type: Int24,
    };
    assert_eq!(
        serde_json::to_value(format).unwrap(),
        serde_json::json!({"rateHz": 48000, "channels": 2, "sampleType": "int24"})
    );
    assert_eq!(Float32.as_str(), "float32");
    assert_eq!(
        serde_json::to_value(Int32).unwrap(),
        serde_json::json!("int32")
    );
}
