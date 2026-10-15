//! A frame's level (#852 C1a, spec §5.4): the RMS of 16 kHz s16 PCM — what the
//! ASR gets — in dBFS. songplayer's `pcm_dbfs` (`scripts/dub_live_session.py`
//! @ dd551b7d, also `eval/dubbing/live_translate_continuous_probe.py`):
//! samples / 32768, floored at −120 dBFS (JSON has no −inf), rounded to
//! 0.01 dB; "voiced" is above −50 dBFS (songplayer's `VOICED_DBFS`, the
//! silence gate of spec §6.4).

/// The level of digital silence (songplayer `SILENCE_DBFS`).
pub const SILENCE_DBFS: f32 = -120.0;
/// Above this a frame is speech (songplayer `VOICED_DBFS`).
pub const VOICED_DBFS: f32 = -50.0;

/// RMS level of mono s16 samples in dBFS.
pub fn rms_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return SILENCE_DBFS;
    }
    let energy: f64 = samples
        .iter()
        .map(|&s| {
            let w = f64::from(s) / 32_768.0;
            w * w
        })
        .sum();
    let rms = (energy / samples.len() as f64).sqrt();
    if rms <= 0.0 {
        return SILENCE_DBFS;
    }
    let db = (20.0 * rms.log10()).max(f64::from(SILENCE_DBFS));
    ((db * 100.0).round() / 100.0) as f32
}

/// songplayer: `db > VOICED_DBFS`.
pub fn is_voiced(dbfs: f32) -> bool {
    dbfs > VOICED_DBFS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::f32_to_int16;

    #[test]
    fn the_levels_match_songplayers_pcm_dbfs() {
        // the Python mirror (Task 2 Step 1) of songplayer's pcm_dbfs:
        assert_eq!(rms_dbfs(&[3000i16, -3000].repeat(50)), -20.77);
        assert_eq!(rms_dbfs(&[30i16, -30].repeat(50)), -60.77);
        assert_eq!(rms_dbfs(&[32_767, -32_767]), 0.0);
        assert_eq!(rms_dbfs(&[0; 8]), SILENCE_DBFS);
        assert_eq!(rms_dbfs(&[]), SILENCE_DBFS);
    }

    #[test]
    fn a_1khz_tone_at_half_scale_is_minus_9_dbfs() {
        let tone: Vec<i16> = (0..1600)
            .map(|n| {
                let x = 0.5 * (std::f64::consts::TAU * 1000.0 * f64::from(n) / 16_000.0).sin();
                f32_to_int16(x as f32)
            })
            .collect();
        assert_eq!(rms_dbfs(&tone), -9.03);
    }

    #[test]
    fn voiced_is_strictly_above_minus_50() {
        assert!(is_voiced(-49.99));
        assert!(!is_voiced(-50.0));
        assert!(!is_voiced(SILENCE_DBFS));
    }
}
