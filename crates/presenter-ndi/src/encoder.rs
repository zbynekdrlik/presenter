use anyhow::{Context, Result};

/// JPEG encoder using libjpeg-turbo for minimal latency.
pub struct JpegEncoder {
    quality: i32,
}

impl JpegEncoder {
    /// Create a new JPEG encoder with the given quality (1-100).
    pub fn new(quality: i32) -> Self {
        Self {
            quality: quality.clamp(1, 100),
        }
    }

    /// Encode BGRA/BGRX pixel data to JPEG.
    ///
    /// Returns the compressed JPEG bytes.
    pub fn encode_bgra(&self, bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let image = turbojpeg::Image {
            pixels: bgra,
            width: width as usize,
            pitch: width as usize * 4,
            height: height as usize,
            format: turbojpeg::PixelFormat::BGRA,
        };
        let buf = turbojpeg::compress(image, self.quality, turbojpeg::Subsamp::Sub2x2)
            .context("JPEG encode failed")?;
        Ok(buf.to_vec())
    }

    /// Encode UYVY pixel data to JPEG.
    ///
    /// Converts UYVY → BGRA first, then encodes.
    pub fn encode_uyvy(&self, uyvy: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let bgra = uyvy_to_bgra(uyvy, width, height);
        self.encode_bgra(&bgra, width, height)
    }
}

/// Convert UYVY to BGRA for JPEG encoding.
fn uyvy_to_bgra(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut bgra = vec![0u8; w * h * 4];

    for y in 0..h {
        for x in (0..w).step_by(2) {
            let uyvy_offset = (y * w + x) * 2;
            let u = uyvy[uyvy_offset] as f32 - 128.0;
            let y0 = uyvy[uyvy_offset + 1] as f32;
            let v = uyvy[uyvy_offset + 2] as f32 - 128.0;
            let y1 = uyvy[uyvy_offset + 3] as f32;

            // YUV to RGB
            let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g0 = (y0 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

            let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g1 = (y1 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

            let idx0 = (y * w + x) * 4;
            bgra[idx0] = b0;
            bgra[idx0 + 1] = g0;
            bgra[idx0 + 2] = r0;
            bgra[idx0 + 3] = 255;

            let idx1 = (y * w + x + 1) * 4;
            bgra[idx1] = b1;
            bgra[idx1 + 1] = g1;
            bgra[idx1 + 2] = r1;
            bgra[idx1 + 3] = 255;
        }
    }

    bgra
}
