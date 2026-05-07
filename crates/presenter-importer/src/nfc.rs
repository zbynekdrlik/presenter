//! NFC normalization at ProPresenter import boundaries.
//!
//! ProPresenter on macOS stores text in NFD form (decomposed Unicode).
//! presenter stores it in NFC so consumers like Resolume's text effect
//! render combining marks correctly.

use unicode_normalization::UnicodeNormalization;

/// Return `s` in Unicode Normalization Form C (canonical composed form).
///
/// Idempotent: calling on already-NFC input yields a string equal to the input.
pub(crate) fn to_nfc(s: &str) -> String {
    s.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::to_nfc;

    #[test]
    fn nfd_input_is_recomposed() {
        // 'ž' as NFD: 'z' + combining caron (U+030C)
        let nfd = "z\u{30c}";
        assert_eq!(to_nfc(nfd), "\u{17e}"); // single 'ž' codepoint
    }

    #[test]
    fn already_nfc_is_unchanged() {
        let nfc = "\u{17e}\u{ed}znim"; // 'žíznim' precomposed
        assert_eq!(to_nfc(nfc), nfc);
    }

    #[test]
    fn ascii_passthrough() {
        assert_eq!(to_nfc("hello"), "hello");
        assert_eq!(to_nfc(""), "");
    }

    #[test]
    fn slovak_phrase_recomposed() {
        // "Po Tebe Pane žíznim" with NFD ž and í
        let nfd = "Po Tebe Pane z\u{30c}i\u{301}znim";
        let nfc = "Po Tebe Pane \u{17e}\u{ed}znim";
        assert_eq!(to_nfc(nfd), nfc);
    }
}
