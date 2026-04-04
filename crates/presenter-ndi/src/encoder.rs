use anyhow::Result;
use openh264::formats::YUVSlices;

/// Wraps an openh264 encoder that accepts YUV420 planar frames and emits H.264.
pub struct VideoEncoder {
    encoder: openh264::encoder::Encoder,
    width: u32,
    height: u32,
}

impl VideoEncoder {
    /// Create a new H.264 encoder for frames of the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let encoder = openh264::encoder::Encoder::new()?;
        Ok(Self {
            encoder,
            width,
            height,
        })
    }

    /// Encode a YUV420 planar frame to an H.264 bitstream.
    ///
    /// The `yuv_data` slice must contain the Y, U and V planes contiguously:
    /// `[Y: w*h bytes] [U: w/2 * h/2 bytes] [V: w/2 * h/2 bytes]`.
    pub fn encode(&mut self, yuv_data: &[u8]) -> Result<Vec<u8>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let y_size = w * h;
        let uv_size = (w / 2) * (h / 2);

        let y_plane = &yuv_data[..y_size];
        let u_plane = &yuv_data[y_size..y_size + uv_size];
        let v_plane = &yuv_data[y_size + uv_size..y_size + uv_size * 2];

        let yuv = YUVSlices::new((y_plane, u_plane, v_plane), (w, h), (w, w / 2, w / 2));

        let bitstream = self.encoder.encode(&yuv)?;
        Ok(bitstream.to_vec())
    }
}

/// Convert UYVY to YUV420 planar.
///
/// NDI commonly sends UYVY; openh264 expects YUV420 planar input.
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

/// Convert BGRA to YUV420 planar.
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
