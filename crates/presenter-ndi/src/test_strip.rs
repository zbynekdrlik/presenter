//! Clock-strip painting/decoding for the synthetic E2E sender.
//!
//! 26 blocks of 48×48 px at (48,48) in a 2560-px-wide UYVY frame:
//! block 0 = always white, block 1 = always black (threshold calibration),
//! blocks 2..=25 = 24-bit big-endian `unix_millis % 2^24` (white=1, black=0).
//! Geometry survives the 1280×720 downscale and 2.5 Mbps H264 encode
//! (verified live 2026-06-11). The Playwright latency e2e decodes the strip
//! from a canvas and computes glass-to-glass latency = Date.now() − value
//! (sender and browser run on the same CI machine → same clock).

pub const STRIP_BLOCK_PX: usize = 48;
pub const STRIP_X0: usize = 48;
pub const STRIP_Y0: usize = 48;
pub const STRIP_DATA_BITS: usize = 24;
pub const STRIP_MODULUS: u64 = 1 << STRIP_DATA_BITS;

/// Paint one block (UYVY: byte pairs [U Y V Y]; luma set, chroma neutral).
fn paint_block(data: &mut [u8], stride: usize, idx: usize, white: bool) {
    let y_val: u8 = if white { 235 } else { 16 };
    let x_px = STRIP_X0 + idx * STRIP_BLOCK_PX;
    for row in STRIP_Y0..(STRIP_Y0 + STRIP_BLOCK_PX) {
        let base = row * stride + x_px * 2;
        for p in (0..STRIP_BLOCK_PX * 2).step_by(2) {
            data[base + p] = 128;
            data[base + p + 1] = y_val;
        }
    }
}

/// Paint the full strip encoding `now_ms % 2^24` into a UYVY frame.
pub fn paint_strip(data: &mut [u8], stride: usize, now_ms: u64) {
    let val = (now_ms % STRIP_MODULUS) as u32;
    paint_block(data, stride, 0, true);
    paint_block(data, stride, 1, false);
    for bit in 0..STRIP_DATA_BITS {
        paint_block(
            data,
            stride,
            2 + bit,
            (val >> (STRIP_DATA_BITS - 1 - bit)) & 1 == 1,
        );
    }
}

/// Decode the strip from a UYVY frame (inverse of `paint_strip`; test-side).
pub fn decode_strip(data: &[u8], stride: usize) -> Option<u32> {
    let luma = |idx: usize| -> u32 {
        let x_px = STRIP_X0 + idx * STRIP_BLOCK_PX + STRIP_BLOCK_PX / 2;
        let y = STRIP_Y0 + STRIP_BLOCK_PX / 2;
        data[y * stride + x_px * 2 + 1] as u32
    };
    let white = luma(0);
    let black = luma(1);
    if white <= black + 50 {
        return None;
    }
    let thr = (white + black) / 2;
    let mut val: u32 = 0;
    for bit in 0..STRIP_DATA_BITS {
        val = (val << 1) | u32::from(luma(2 + bit) > thr);
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_roundtrip_paints_and_decodes() {
        let stride = 2560 * 2;
        let mut frame = vec![100u8; stride * 1440];
        paint_strip(&mut frame, stride, 1_781_179_287_123);
        assert_eq!(
            decode_strip(&frame, stride),
            Some((1_781_179_287_123u64 % STRIP_MODULUS) as u32)
        );
    }

    #[test]
    fn strip_decode_returns_none_without_strip() {
        let stride = 2560 * 2;
        let frame = vec![100u8; stride * 1440];
        assert_eq!(decode_strip(&frame, stride), None);
    }
}
