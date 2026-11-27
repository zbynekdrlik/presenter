//! The guest translation page's pure half (#999 L3), host-tested: the speech
//! frames (s16le → f32, a linear rate change when the phone's audio runs at
//! another rate), the transcript's paragraphs and the Slovak texts of every
//! state and refusal. The page (`pages::guest`) only wires these to the DOM.

use presenter_core::listen::guest::{GuestRefusal, GuestSessionState, GuestStopReason};

/// A new paragraph after this long without text (the preacher paused).
pub const PARAGRAPH_GAP_MS: f64 = 2_500.0;
/// A paragraph this long ends at its next sentence end.
pub const PARAGRAPH_SOFT_CHARS: usize = 160;
/// The newest paragraphs kept on screen.
pub const MAX_PARAGRAPHS: usize = 60;

/// s16le bytes → samples in −1…1 (an odd last byte is ignored).
pub fn pcm_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0)
        .collect()
}

/// Linear rate change for a phone whose audio context refused 24 kHz (speech,
/// so linear is enough). Same rate → the input unchanged.
pub fn resample(input: &[f32], from: f32, to: f32) -> Vec<f32> {
    if input.is_empty() || from <= 0.0 || to <= 0.0 || (from - to).abs() < f32::EPSILON {
        return input.to_vec();
    }
    let ratio = from / to;
    let out_len = ((input.len() as f32) / ratio).round().max(1.0) as usize;
    let last = input.len() - 1;
    (0..out_len)
        .map(|i| {
            let pos = i as f32 * ratio;
            let left = (pos.floor() as usize).min(last);
            let right = (left + 1).min(last);
            let frac = pos - left as f32;
            input[left] * (1.0 - frac) + input[right] * frac
        })
        .collect()
}

/// One paragraph: the translation and the Slovak original of the same stretch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paragraph {
    pub id: u64,
    pub text: String,
    pub original: String,
}

/// The transcript as fragments arrive.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub paragraphs: Vec<Paragraph>,
    next_id: u64,
    last_text_ms: Option<f64>,
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .ends_with(|c: char| matches!(c, '.' | '?' | '!' | '…'))
}

impl Transcript {
    fn open_new(&mut self) {
        self.next_id += 1;
        self.paragraphs.push(Paragraph {
            id: self.next_id,
            ..Paragraph::default()
        });
        if self.paragraphs.len() > MAX_PARAGRAPHS {
            self.paragraphs.remove(0);
        }
    }

    /// Whether a fragment arriving at `now_ms` starts a new paragraph.
    fn breaks_before(&self, now_ms: f64) -> bool {
        let Some(current) = self.paragraphs.last() else {
            return true;
        };
        let paused = self
            .last_text_ms
            .is_some_and(|last| now_ms - last >= PARAGRAPH_GAP_MS);
        let long =
            current.text.chars().count() >= PARAGRAPH_SOFT_CHARS && ends_sentence(&current.text);
        (paused || long) && !current.text.trim().is_empty()
    }

    /// Append a translated fragment (Gemini sends them with their own spaces).
    pub fn push_text(&mut self, fragment: &str, now_ms: f64) {
        if fragment.is_empty() {
            return;
        }
        if self.paragraphs.is_empty() || self.breaks_before(now_ms) {
            self.open_new();
        }
        self.last_text_ms = Some(now_ms);
        if let Some(current) = self.paragraphs.last_mut() {
            let fragment = if current.text.is_empty() {
                fragment.trim_start()
            } else {
                fragment
            };
            current.text.push_str(fragment);
        }
    }

    /// Append a fragment of the Slovak original to the current paragraph.
    pub fn push_original(&mut self, fragment: &str) {
        if self.paragraphs.is_empty() {
            self.open_new();
        }
        if let Some(current) = self.paragraphs.last_mut() {
            let fragment = if current.original.is_empty() {
                fragment.trim_start()
            } else {
                fragment
            };
            current.original.push_str(fragment);
        }
    }

    /// The model ended a turn: the next fragment starts a paragraph.
    pub fn end_turn(&mut self) {
        if self
            .paragraphs
            .last()
            .is_some_and(|p| !p.text.trim().is_empty())
        {
            self.open_new();
        }
    }
}

/// `data-state` + the Slovak line of a session state.
pub fn state_text(state: &GuestSessionState) -> (&'static str, &'static str) {
    match state {
        GuestSessionState::Connecting => ("connecting", "Pripájam…"),
        GuestSessionState::Waiting => ("waiting", "Čakám, kým kazateľ prehovorí"),
        GuestSessionState::Live => ("live", "Naživo"),
        GuestSessionState::Reconnecting => ("reconnecting", "Obnovujem spojenie…"),
        GuestSessionState::Stopped { reason } => ("stopped", stop_text(*reason)),
    }
}

fn stop_text(reason: GuestStopReason) -> &'static str {
    match reason {
        GuestStopReason::Key | GuestStopReason::Model => {
            "Preklad je teraz nedostupný (chyba služby)"
        }
        GuestStopReason::DailyCap => "Dnešný čas prekladu je vyčerpaný",
    }
}

/// The Slovak line of a refusal.
pub fn refusal_text(reason: GuestRefusal) -> &'static str {
    match reason {
        GuestRefusal::UnknownLanguage => "Tento jazyk sa neponúka",
        GuestRefusal::NoKey => "Preklad tu nie je nastavený",
        GuestRefusal::NoSource => "Mikrofón kazateľa tu nie je pripojený",
        GuestRefusal::Full => {
            "Naraz sa prekladajú najviac 4 jazyky — vyberte jeden z tých, ktoré už bežia (●)"
        }
        GuestRefusal::DailyCap => "Dnešný čas prekladu je vyčerpaný",
    }
}

/// The reconnect pause after `failures` closed sockets: 1, 2, 4, 8, 10 s.
pub fn reconnect_delay_ms(failures: u32) -> u32 {
    let shift = failures.saturating_sub(1).min(4);
    (1_000_u32 << shift).min(10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_becomes_float_and_the_rate_changes_linearly() {
        assert_eq!(
            pcm_to_f32(&[0, 0x80, 0xFF, 0x7F, 0x00, 0x40, 7]),
            vec![-1.0, 32_767.0 / 32_768.0, 0.5]
        );
        let same = resample(&[0.1, 0.2], 24_000.0, 24_000.0);
        assert_eq!(same, vec![0.1, 0.2]);
        let up = resample(&[0.0, 1.0, 0.0, -1.0], 24_000.0, 48_000.0);
        assert_eq!(up.len(), 8);
        assert!((up[1] - 0.5).abs() < 1e-6, "{up:?}");
        assert!((up[2] - 1.0).abs() < 1e-6, "{up:?}");
        let down = resample(&vec![0.25; 2_400], 24_000.0, 44_100.0);
        assert_eq!(down.len(), 4_410);
        assert!(down.iter().all(|s| (s - 0.25).abs() < 1e-6));
        assert!(resample(&[], 24_000.0, 48_000.0).is_empty());
    }

    #[test]
    fn fragments_join_and_a_pause_starts_a_paragraph() {
        let mut t = Transcript::default();
        t.push_text(" They had it promised,", 0.0);
        t.push_text(" they crossed the Red", 400.0);
        t.push_original("mali to zasľúbené,");
        t.push_text(" Sea.", 900.0);
        assert_eq!(t.paragraphs.len(), 1);
        assert_eq!(
            t.paragraphs[0].text,
            "They had it promised, they crossed the Red Sea."
        );
        assert_eq!(t.paragraphs[0].original, "mali to zasľúbené,");
        t.push_text(" And then", 900.0 + PARAGRAPH_GAP_MS);
        assert_eq!(t.paragraphs.len(), 2);
        assert_eq!(t.paragraphs[1].text, "And then");
        assert_ne!(t.paragraphs[0].id, t.paragraphs[1].id);
        t.end_turn();
        t.push_text(" next", 3_500.0);
        assert_eq!(t.paragraphs.len(), 3);
    }

    #[test]
    fn a_long_paragraph_ends_at_a_sentence_end_and_old_ones_go() {
        let mut t = Transcript::default();
        let mut now = 0.0;
        for _ in 0..40 {
            t.push_text(" word", now);
            now += 100.0;
        }
        assert_eq!(t.paragraphs.len(), 1, "no sentence end yet");
        t.push_text(" end.", now);
        t.push_text(" Next", now + 100.0);
        assert_eq!(t.paragraphs.len(), 2);
        for i in 0..(MAX_PARAGRAPHS * 2) {
            t.push_text(" x", 10_000.0 * (i as f64 + 1.0));
        }
        assert_eq!(t.paragraphs.len(), MAX_PARAGRAPHS);
    }

    #[test]
    fn every_state_and_refusal_has_slovak_text() {
        assert_eq!(state_text(&GuestSessionState::Live), ("live", "Naživo"));
        assert_eq!(state_text(&GuestSessionState::Waiting).0, "waiting");
        let stopped = GuestSessionState::Stopped {
            reason: GuestStopReason::DailyCap,
        };
        assert_eq!(
            state_text(&stopped),
            ("stopped", "Dnešný čas prekladu je vyčerpaný")
        );
        assert!(refusal_text(GuestRefusal::Full).contains("4 jazyky"));
        assert_eq!(
            refusal_text(GuestRefusal::NoSource),
            "Mikrofón kazateľa tu nie je pripojený"
        );
        let delays: Vec<u32> = (1..=6).map(reconnect_delay_ms).collect();
        assert_eq!(delays, vec![1_000, 2_000, 4_000, 8_000, 10_000, 10_000]);
    }
}
