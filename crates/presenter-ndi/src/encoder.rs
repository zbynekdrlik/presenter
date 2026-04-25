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
pub fn uyvy_to_bgra(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {
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

impl JpegEncoder {
    /// Resize BGRA pixel data to `target_height` (preserving aspect) and JPEG-encode.
    ///
    /// If `src_height == target_height`, this is a fast path that skips resize.
    /// Otherwise uses `image::imageops::resize` with the `Triangle` filter,
    /// chosen for cheap CPU cost over Lanczos quality (the difference is
    /// imperceptible at typical NDI-display sizes).
    pub fn encode_bgra_resized(
        &self,
        bgra: &[u8],
        src_width: u32,
        src_height: u32,
        target_height: u32,
    ) -> Result<Vec<u8>> {
        if target_height == src_height {
            return self.encode_bgra(bgra, src_width, src_height);
        }
        let target_width = (src_width * target_height) / src_height;
        // Make even — turbojpeg with Sub2x2 chroma requires even dims.
        let target_width = target_width & !1;
        let target_height = target_height & !1;

        let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(src_width, src_height, bgra.to_vec())
            .ok_or_else(|| anyhow::anyhow!("BGRA buffer size mismatch: {} bytes for {}x{}", bgra.len(), src_width, src_height))?;
        let resized = image::imageops::resize(&img, target_width, target_height, image::imageops::FilterType::Triangle);
        self.encode_bgra(resized.as_raw(), target_width, target_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bgra(w: u32, h: u32) -> Vec<u8> {
        // Simple gradient so resize has something to interpolate
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.push((x % 256) as u8);    // B
                out.push((y % 256) as u8);    // G
                out.push(((x + y) % 256) as u8); // R
                out.push(255);                // A
            }
        }
        out
    }

    #[test]
    fn encode_bgra_resized_passthrough_when_target_equals_source() {
        let bgra = make_bgra(64, 64);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 64, 64, 64).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]), "JPEG SOI marker missing");
    }

    #[test]
    fn encode_bgra_resized_downscales_aspect_preserved() {
        // Resize 1920x1080 → 720 height. Width must scale to 1280 (preserving 16:9).
        let bgra = make_bgra(1920, 1080);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 1920, 1080, 720).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));

        // Decode and check dims
        let img = turbojpeg::decompress(&jpeg, turbojpeg::PixelFormat::BGRA).unwrap();
        assert_eq!(img.height, 720);
        assert_eq!(img.width, 1280);
    }

    #[test]
    fn encode_bgra_resized_rejects_wrong_buffer_size() {
        let bgra = vec![0u8; 16]; // way too small
        let enc = JpegEncoder::new(75);
        let err = enc.encode_bgra_resized(&bgra, 100, 100, 50).unwrap_err();
        assert!(err.to_string().contains("buffer size mismatch"));
    }

    #[test]
    fn uyvy_to_bgra_produces_4bytes_per_pixel() {
        // 4x2 dummy UYVY frame
        let uyvy = vec![128u8; 4 * 2 * 2]; // 2 bytes per pixel
        let bgra = uyvy_to_bgra(&uyvy, 4, 2);
        assert_eq!(bgra.len(), 4 * 2 * 4);
    }
}
