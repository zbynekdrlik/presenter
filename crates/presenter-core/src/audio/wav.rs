//! The recorder's file format (#852 C1b, spec §5.5): 16 kHz mono s16le — the
//! frames the ASR gets — behind the canonical 44-byte PCM WAV header. The
//! recorder patches the two size fields after every batch it writes, so a
//! crash leaves a file whose header covers the audio up to the last batch. A
//! file never comes near the 4 GiB a u32 size field can hold: one mic's day is at
//! most 2.76 GB (24 h × 32 000 B/s) and the recorder starts new files at midnight.

/// The header length; the audio starts here.
pub const WAV_HEADER_LEN: usize = 44;
/// The recorder's sample rate (spec §5.4: 16 kHz mono s16le).
pub const WAV_SAMPLE_RATE_HZ: u32 = 16_000;
/// Mono s16: one sample is one block of two bytes.
pub const WAV_BLOCK_BYTES: u16 = 2;
/// Where the RIFF size field (`36 + data`) is.
pub const WAV_RIFF_SIZE_AT: u64 = 4;
/// Where the data size field is.
pub const WAV_DATA_SIZE_AT: u64 = 40;

/// The header of a file holding `data_bytes` of audio.
pub fn wav_header(data_bytes: u32) -> [u8; WAV_HEADER_LEN] {
    let byte_rate = WAV_SAMPLE_RATE_HZ * u32::from(WAV_BLOCK_BYTES);
    let mut header = [0u8; WAV_HEADER_LEN];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&wav_riff_size(data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    header[24..28].copy_from_slice(&WAV_SAMPLE_RATE_HZ.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&WAV_BLOCK_BYTES.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

/// The RIFF size field of a file holding `data_bytes` of audio.
pub fn wav_riff_size(data_bytes: u32) -> u32 {
    data_bytes.saturating_add(36)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Python's `wave` module writes for one second of 16 kHz mono s16
    /// (Task 6 Step 1).
    const ONE_SECOND: [u8; WAV_HEADER_LEN] = [
        82, 73, 70, 70, 36, 125, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32, 16, 0, 0, 0, 1, 0, 1, 0,
        128, 62, 0, 0, 0, 125, 0, 0, 2, 0, 16, 0, 100, 97, 116, 97, 0, 125, 0, 0,
    ];

    #[test]
    fn one_second_has_the_header_python_writes() {
        assert_eq!(wav_header(32_000), ONE_SECOND);
        assert_eq!(&wav_header(0)[4..8], &36u32.to_le_bytes());
        assert_eq!(&wav_header(0)[40..44], &0u32.to_le_bytes());
    }

    #[test]
    fn the_size_fields_sit_where_the_recorder_patches_them() {
        let header = wav_header(0x0102_0304);
        let riff = WAV_RIFF_SIZE_AT as usize;
        let data = WAV_DATA_SIZE_AT as usize;
        assert_eq!(
            &header[riff..riff + 4],
            &(0x0102_0304u32 + 36).to_le_bytes()
        );
        assert_eq!(&header[data..data + 4], &0x0102_0304u32.to_le_bytes());
    }

    #[test]
    fn the_riff_size_is_the_audio_plus_36_and_never_wraps() {
        assert_eq!(wav_riff_size(0), 36);
        assert_eq!(wav_riff_size(32_000), 32_036);
        assert_eq!(wav_riff_size(u32::MAX), u32::MAX, "saturates, never wraps");
    }
}
