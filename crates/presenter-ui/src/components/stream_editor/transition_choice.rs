//! The editor's content-transition choice (#834) — pure + host-tested.
//!
//! `TransitionFields` (`element_form.rs`) offers three radios: cut, crossfade
//! and fade through empty. Switching between the two fades KEEPS the duration
//! the operator set; coming from a cut starts at the default fade.

use presenter_core::{ContentTransition, STREAM_DEFAULT_FADE_MS};

/// One of the three content-transition radios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionChoice {
    /// „Strih" — instant swap.
    Cut,
    /// „Prelínať (crossfade)" — old and new text overlap while fading.
    Crossfade,
    /// „Prelínať cez prázdno" — old text fades out, then the new one fades in.
    FadeThrough,
}

impl TransitionChoice {
    /// Which radio a stored transition selects.
    pub fn of(transition: &ContentTransition) -> Self {
        match transition {
            ContentTransition::Cut => TransitionChoice::Cut,
            ContentTransition::Fade { .. } => TransitionChoice::Crossfade,
            ContentTransition::FadeThrough { .. } => TransitionChoice::FadeThrough,
        }
    }

    /// `current` switched to this choice, keeping its fade duration (the
    /// default fade when `current` is a cut).
    pub fn apply(self, current: &ContentTransition) -> ContentTransition {
        let duration_ms = duration_ms(current).unwrap_or(STREAM_DEFAULT_FADE_MS);
        match self {
            TransitionChoice::Cut => ContentTransition::Cut,
            TransitionChoice::Crossfade => ContentTransition::Fade { duration_ms },
            TransitionChoice::FadeThrough => ContentTransition::FadeThrough { duration_ms },
        }
    }
}

/// The fade duration of a timed transition; `None` for a cut.
pub fn duration_ms(transition: &ContentTransition) -> Option<u32> {
    match transition {
        ContentTransition::Cut => None,
        ContentTransition::Fade { duration_ms }
        | ContentTransition::FadeThrough { duration_ms } => Some(*duration_ms),
    }
}

/// Set the duration of a timed transition (a cut has none — left unchanged).
pub fn set_duration_ms(transition: &mut ContentTransition, ms: u32) {
    match transition {
        ContentTransition::Cut => {}
        ContentTransition::Fade { duration_ms }
        | ContentTransition::FadeThrough { duration_ms } => {
            *duration_ms = ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_stored_transition_selects_its_radio() {
        assert_eq!(
            TransitionChoice::of(&ContentTransition::Cut),
            TransitionChoice::Cut
        );
        assert_eq!(
            TransitionChoice::of(&ContentTransition::Fade { duration_ms: 300 }),
            TransitionChoice::Crossfade
        );
        assert_eq!(
            TransitionChoice::of(&ContentTransition::FadeThrough { duration_ms: 300 }),
            TransitionChoice::FadeThrough
        );
    }

    #[test]
    fn switching_between_the_fades_keeps_the_duration() {
        let fade = ContentTransition::Fade { duration_ms: 700 };
        let through = TransitionChoice::FadeThrough.apply(&fade);
        assert_eq!(through, ContentTransition::FadeThrough { duration_ms: 700 });
        assert_eq!(TransitionChoice::Crossfade.apply(&through), fade);
    }

    #[test]
    fn leaving_and_returning_from_a_cut() {
        let cut = TransitionChoice::Cut.apply(&ContentTransition::Fade { duration_ms: 700 });
        assert_eq!(cut, ContentTransition::Cut);
        assert_eq!(
            TransitionChoice::FadeThrough.apply(&cut),
            ContentTransition::FadeThrough {
                duration_ms: STREAM_DEFAULT_FADE_MS
            }
        );
    }

    #[test]
    fn duration_reads_and_writes_both_fades_only() {
        let mut through = ContentTransition::FadeThrough { duration_ms: 300 };
        set_duration_ms(&mut through, 900);
        assert_eq!(duration_ms(&through), Some(900));
        let mut fade = ContentTransition::Fade { duration_ms: 300 };
        set_duration_ms(&mut fade, 450);
        assert_eq!(duration_ms(&fade), Some(450));
        let mut cut = ContentTransition::Cut;
        set_duration_ms(&mut cut, 900);
        assert_eq!(cut, ContentTransition::Cut);
        assert_eq!(duration_ms(&cut), None);
    }
}
