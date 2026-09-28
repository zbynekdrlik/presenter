//! Text-mode selection for the API stage layouts (#799).
//!
//! An API slide carries the original line (`main`) and, optionally, its
//! translation. The operator-selected [`StageTextMode`] decides which of them
//! the `api` and `api-ambient` layouts render. Pure — no DOM, host-tested.

use presenter_core::{StageDisplaySlide, StageTextMode};

/// The line(s) an API layout renders for one slide under a text mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiLines {
    /// The main (larger) line. Empty when the slide has no text at all.
    pub primary: String,
    /// The smaller line under it — only in `both` mode, only when a
    /// translation was sent and differs from the original.
    pub secondary: Option<String>,
}

impl ApiLines {
    /// No text to show (the ambient overlay hides entirely).
    pub fn is_empty(&self) -> bool {
        self.primary.is_empty() && self.secondary.is_none()
    }

    /// Apply `f` to each line (e.g. the stage's long-line tail-break, which
    /// skips text that already contains a newline — so it must run per line
    /// BEFORE [`ApiLines::joined`]).
    pub fn map_lines(self, f: impl Fn(String) -> String) -> Self {
        Self {
            primary: f(self.primary),
            secondary: self.secondary.map(&f),
        }
    }

    /// Both lines in one string (secondary on its own line) — for layouts
    /// that render a single auto-fitted text box (the `api` layout's
    /// WorshipSnv boxes).
    pub fn joined(&self) -> String {
        match &self.secondary {
            Some(secondary) => format!("{}\n{}", self.primary, secondary),
            None => self.primary.clone(),
        }
    }
}

/// Select the line(s) to render for `main` / `translation` under `mode`.
///
/// - `original`: the original line only (nothing when only a translation
///   was sent — the operator explicitly asked for the original).
/// - `translation`: the translation only; falls back to the original when no
///   translation was sent (a blank stage would look broken mid-song).
/// - `both`: original, translation below it. With no translation (or one
///   identical to the original) this is exactly `original`.
///
/// Whitespace-only lines count as empty.
pub fn select_api_lines(main: &str, translation: &str, mode: StageTextMode) -> ApiLines {
    let main = main.trim();
    let translation = translation.trim();
    let (primary, secondary) = match mode {
        StageTextMode::Original => (main, None),
        StageTextMode::Translation if !translation.is_empty() => (translation, None),
        StageTextMode::Translation => (main, None),
        StageTextMode::Both if main.is_empty() => (translation, None),
        StageTextMode::Both if translation.is_empty() || translation == main => (main, None),
        StageTextMode::Both => (main, Some(translation)),
    };
    ApiLines {
        primary: primary.to_string(),
        secondary: secondary.map(str::to_string),
    }
}

/// [`select_api_lines`] for an optional snapshot slide; a missing slide (or a
/// snapshot without a mode, i.e. a non-API snapshot) uses the default mode.
pub fn slide_api_lines(slide: Option<&StageDisplaySlide>, mode: Option<StageTextMode>) -> ApiLines {
    slide
        .map(|slide| select_api_lines(&slide.main, &slide.translation, mode.unwrap_or_default()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = "Amazing grace";
    const SK: &str = "Úžasná milosť";

    #[test]
    fn original_mode_shows_only_the_original() {
        let lines = select_api_lines(EN, SK, StageTextMode::Original);
        assert_eq!(lines.primary, EN);
        assert_eq!(lines.secondary, None);
        assert_eq!(lines.joined(), EN);
    }

    #[test]
    fn translation_mode_shows_only_the_translation() {
        let lines = select_api_lines(EN, SK, StageTextMode::Translation);
        assert_eq!(lines.primary, SK);
        assert_eq!(lines.secondary, None);
    }

    #[test]
    fn translation_mode_falls_back_to_original_without_a_translation() {
        let lines = select_api_lines(EN, "  ", StageTextMode::Translation);
        assert_eq!(lines.primary, EN);
        assert_eq!(lines.secondary, None);
    }

    #[test]
    fn both_mode_puts_the_translation_under_the_original() {
        let lines = select_api_lines(EN, SK, StageTextMode::Both);
        assert_eq!(lines.primary, EN);
        assert_eq!(lines.secondary.as_deref(), Some(SK));
        assert_eq!(lines.joined(), format!("{EN}\n{SK}"));
    }

    #[test]
    fn both_mode_without_translation_equals_original() {
        assert_eq!(
            select_api_lines(EN, "", StageTextMode::Both),
            select_api_lines(EN, "", StageTextMode::Original)
        );
        // An identical translation is not repeated.
        assert_eq!(
            select_api_lines(EN, EN, StageTextMode::Both).secondary,
            None
        );
    }

    #[test]
    fn both_mode_with_only_a_translation_shows_it_as_primary() {
        let lines = select_api_lines("", SK, StageTextMode::Both);
        assert_eq!(lines.primary, SK);
        assert_eq!(lines.secondary, None);
    }

    #[test]
    fn original_mode_with_only_a_translation_is_empty() {
        assert!(select_api_lines("", SK, StageTextMode::Original).is_empty());
    }

    #[test]
    fn no_text_is_empty_in_every_mode() {
        for mode in StageTextMode::ALL {
            assert!(select_api_lines("", "", mode).is_empty(), "{mode}");
            assert!(select_api_lines(" \n ", "", mode).is_empty(), "{mode}");
        }
    }

    #[test]
    fn map_lines_transforms_each_line_separately() {
        let lines = select_api_lines(EN, SK, StageTextMode::Both).map_lines(|l| l.to_uppercase());
        assert_eq!(lines.primary, EN.to_uppercase());
        assert_eq!(lines.secondary, Some(SK.to_uppercase()));
        let single = select_api_lines(EN, SK, StageTextMode::Original).map_lines(|l| l + "!");
        assert_eq!(single.joined(), format!("{EN}!"));
    }

    #[test]
    fn missing_slide_is_empty_and_missing_mode_defaults_to_both() {
        assert!(slide_api_lines(None, Some(StageTextMode::Both)).is_empty());
        let slide = StageDisplaySlide {
            main: EN.to_string(),
            translation: SK.to_string(),
            stage: String::new(),
            group: None,
            group_color: None,
        };
        assert_eq!(
            slide_api_lines(Some(&slide), None).secondary.as_deref(),
            Some(SK)
        );
        assert_eq!(
            slide_api_lines(Some(&slide), Some(StageTextMode::Translation)).primary,
            SK
        );
    }
}
