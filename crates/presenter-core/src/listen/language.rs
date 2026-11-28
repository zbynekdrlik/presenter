//! A mic's spoken language from its own transcript (#852 T2, ROZHODNUTÉ
//! 28.11.2026: no separate language detection — the language is Soniox's
//! token `language` flag). The `pulpit` selector ranks a non-Slovak mic (the
//! original speaker, a guest preacher) before a Slovak one (his live
//! interpreter), so the guests' translation follows the ORIGINAL.
//!
//! [`LanguageWindow`] keeps the language of a mic's last
//! [`LANGUAGE_WINDOW_TOKENS`] final word tokens; [`dominant_language`] is the
//! pure rule over them. Soniox tags every token and keeps a sentence coherent,
//! but in real time a tag can flip while context arrives, so only FINAL tokens
//! count, and one "Amen" in another language never flips a mic: the majority
//! of the window decides, at least [`MIN_LANGUAGE_TOKENS`] tagged tokens are
//! needed, and a tie keeps the newest token's language.

use std::collections::VecDeque;

use crate::listen::transcript::AsrToken;

/// The final word tokens a mic's language is taken over (~20 words: Soniox
/// splits a longer word into sub-word tokens).
pub const LANGUAGE_WINDOW_TOKENS: usize = 30;
/// Tagged tokens needed before a mic has a language at all.
pub const MIN_LANGUAGE_TOKENS: usize = 6;

/// What a speaker speaks, as the `pulpit` policy ranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpokenLanguage {
    /// Slovak (the pastor, or the interpreter of a guest).
    Slovak,
    /// Any other language: the guest preacher.
    Foreign,
}

impl SpokenLanguage {
    /// A Soniox language code. Czech counts as Slovak: the two are close and
    /// a Slovak speaker tagged `cs` is still the interpreter, never a guest.
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "sk" | "cs" => Self::Slovak,
            _ => Self::Foreign,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slovak => "slovak",
            Self::Foreign => "foreign",
        }
    }
}

/// The dominant language of `languages` (oldest first): the majority, a tie
/// to the newest; `None` below [`MIN_LANGUAGE_TOKENS`].
pub fn dominant_language(languages: &[SpokenLanguage]) -> Option<SpokenLanguage> {
    if languages.len() < MIN_LANGUAGE_TOKENS {
        return None;
    }
    let slovak = languages
        .iter()
        .filter(|l| **l == SpokenLanguage::Slovak)
        .count();
    let foreign = languages.len() - slovak;
    match slovak.cmp(&foreign) {
        std::cmp::Ordering::Greater => Some(SpokenLanguage::Slovak),
        std::cmp::Ordering::Less => Some(SpokenLanguage::Foreign),
        std::cmp::Ordering::Equal => languages.last().copied(),
    }
}

/// One mic's window of final tagged word tokens.
#[derive(Debug, Clone, Default)]
pub struct LanguageWindow {
    tokens: VecDeque<SpokenLanguage>,
}

impl LanguageWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count a token when it is final, tagged, carries a letter (no
    /// punctuation, no `<end>` / `<fin>` marker) and is no translation.
    pub fn push(&mut self, token: &AsrToken) {
        if !token.is_final || token.is_translation || is_marker(&token.text) {
            return;
        }
        if !token.text.chars().any(char::is_alphabetic) {
            return;
        }
        let Some(code) = token.language.as_deref() else {
            return;
        };
        if self.tokens.len() == LANGUAGE_WINDOW_TOKENS {
            self.tokens.pop_front();
        }
        self.tokens.push_back(SpokenLanguage::from_code(code));
    }

    /// The window's dominant language ([`dominant_language`]).
    pub fn dominant(&self) -> Option<SpokenLanguage> {
        let languages: Vec<SpokenLanguage> = self.tokens.iter().copied().collect();
        dominant_language(&languages)
    }
}

/// Soniox's control tokens (`<end>`, `<fin>`) are no words.
fn is_marker(text: &str) -> bool {
    let text = text.trim();
    text.starts_with('<') && text.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, language: Option<&str>, is_final: bool) -> AsrToken {
        AsrToken {
            text: text.to_string(),
            is_final,
            language: language.map(str::to_string),
            ..AsrToken::default()
        }
    }

    fn many(language: SpokenLanguage, n: usize) -> Vec<SpokenLanguage> {
        vec![language; n]
    }

    #[test]
    fn too_few_tokens_have_no_language() {
        let five = many(SpokenLanguage::Foreign, MIN_LANGUAGE_TOKENS - 1);
        assert_eq!(dominant_language(&five), None);
        let six = many(SpokenLanguage::Foreign, MIN_LANGUAGE_TOKENS);
        assert_eq!(dominant_language(&six), Some(SpokenLanguage::Foreign));
    }

    #[test]
    fn the_majority_decides_and_a_tie_keeps_the_newest() {
        let mut mostly_slovak = many(SpokenLanguage::Slovak, 7);
        mostly_slovak.push(SpokenLanguage::Foreign);
        assert_eq!(
            dominant_language(&mostly_slovak),
            Some(SpokenLanguage::Slovak),
            "one English word in Slovak speech keeps the mic Slovak"
        );
        let mut tie = many(SpokenLanguage::Slovak, 4);
        tie.extend(many(SpokenLanguage::Foreign, 4));
        assert_eq!(dominant_language(&tie), Some(SpokenLanguage::Foreign));
        let mut tie_to_slovak = many(SpokenLanguage::Foreign, 4);
        tie_to_slovak.extend(many(SpokenLanguage::Slovak, 4));
        assert_eq!(
            dominant_language(&tie_to_slovak),
            Some(SpokenLanguage::Slovak)
        );
    }

    #[test]
    fn czech_is_slovak_and_any_other_code_is_foreign() {
        assert_eq!(SpokenLanguage::from_code("sk"), SpokenLanguage::Slovak);
        assert_eq!(SpokenLanguage::from_code("CS"), SpokenLanguage::Slovak);
        assert_eq!(SpokenLanguage::from_code("en"), SpokenLanguage::Foreign);
        assert_eq!(SpokenLanguage::from_code("de"), SpokenLanguage::Foreign);
    }

    #[test]
    fn only_final_tagged_word_tokens_count() {
        let mut window = LanguageWindow::new();
        for _ in 0..10 {
            window.push(&token(" grace", Some("en"), false));
            window.push(&token(".", Some("en"), true));
            window.push(&token("<end>", Some("en"), true));
            window.push(&token(" milosť", None, true));
        }
        let mut translated = token(" milosť", Some("sk"), true);
        translated.is_translation = true;
        window.push(&translated);
        assert_eq!(window.dominant(), None, "nothing countable arrived");
        for _ in 0..MIN_LANGUAGE_TOKENS {
            window.push(&token(" grace", Some("en"), true));
        }
        assert_eq!(window.dominant(), Some(SpokenLanguage::Foreign));
    }

    #[test]
    fn the_window_follows_the_last_thirty_tokens() {
        let mut window = LanguageWindow::new();
        for _ in 0..LANGUAGE_WINDOW_TOKENS {
            window.push(&token(" grace", Some("en"), true));
        }
        for _ in 0..(LANGUAGE_WINDOW_TOKENS / 2) {
            window.push(&token(" milosť", Some("sk"), true));
        }
        assert_eq!(
            window.dominant(),
            Some(SpokenLanguage::Slovak),
            "a tie at half the window goes to the newest"
        );
        for _ in 0..(LANGUAGE_WINDOW_TOKENS / 2) {
            window.push(&token(" milosť", Some("sk"), true));
        }
        assert_eq!(window.tokens.len(), LANGUAGE_WINDOW_TOKENS);
        assert!(window.tokens.iter().all(|l| *l == SpokenLanguage::Slovak));
    }
}
