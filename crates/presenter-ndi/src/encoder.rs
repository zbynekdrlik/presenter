use anyhow::{Context, Result};
use vpx_encode as vpx;

/// Wraps a libvpx VP8 encoder that accepts YUV420 planar frames.
pub struct VideoEncoder {
    encoder: vpx::Encoder,
    pts: i64,
}

impl VideoEncoder {
    /// Create a new VP8 encoder for frames of the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let config = vpx::Config {
            width,
            height,
            timebase: [1, 30],
            bitrate: 2000,
            codec: vpx::VideoCodecId::VP8,
        };
        let encoder = vpx::Encoder::new(config).context("failed to create VP8 encoder")?;
        Ok(Self { encoder, pts: 0 })
    }

    /// Encode a YUV420 planar frame to VP8.
    ///
    /// The `yuv_data` slice must contain Y, U and V planes contiguously:
    /// `[Y: w*h bytes] [U: w/2 * h/2 bytes] [V: w/2 * h/2 bytes]`.
    pub fn encode(&mut self, yuv_data: &[u8]) -> Result<Vec<u8>> {
        let packets = self
            .encoder
            .encode(self.pts, yuv_data)
            .context("VP8 encode failed")?;
        self.pts += 1;

        let mut output = Vec::new();
        for pkt in packets {
            output.extend_from_slice(pkt.data);
        }
        Ok(output)
    }
}

/// Convert UYVY to YUV420 planar.
pub fn uyvy_to_yuv420(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    let mut yuv = vec![0u8; y_size + uv_size * 2];

    let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    for y in 0..h {
        for x in (0..w).step_by(2) {
            let uyvy_offset = (y * w + x) * 2;
            let u = uyvy[uyvy_offset];
            let y0 = uyvy[uyvy_offset + 1];
            let v = uyvy[uyvy_offset + 2];
            let y1 = uyvy[uyvy_offset + 3];

            y_plane[y * w + x] = y0;
            y_plane[y * w + x + 1] = y1;

            if y % 2 == 0 {
                let uv_x = x / 2;
                let uv_y = y / 2;
                u_plane[uv_y * (w / 2) + uv_x] = u;
                v_plane[uv_y * (w / 2) + uv_x] = v;
            }
        }
    }

    yuv
}

/// Convert BGRA/BGRX to YUV420 planar.
pub fn bgra_to_yuv420(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    let mut yuv = vec![0u8; y_size + uv_size * 2];

    let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    for y in 0..h {
        for x in 0..w {
            let bgra_offset = (y * w + x) * 4;
            let b = bgra[bgra_offset] as f32;
            let g = bgra[bgra_offset + 1] as f32;
            let r = bgra[bgra_offset + 2] as f32;

            let y_val = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
            y_plane[y * w + x] = y_val;

            if y % 2 == 0 && x % 2 == 0 {
                let u_val = (128.0 - 0.169 * r - 0.331 * g + 0.500 * b).clamp(0.0, 255.0) as u8;
                let v_val = (128.0 + 0.500 * r - 0.419 * g - 0.081 * b).clamp(0.0, 255.0) as u8;
                let uv_x = x / 2;
                let uv_y = y / 2;
                u_plane[uv_y * (w / 2) + uv_x] = u_val;
                v_plane[uv_y * (w / 2) + uv_x] = v_val;
            }
        }
    }

    yuv
}
