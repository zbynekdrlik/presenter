//! Which translations the console's bar offers (#850 spec §7.1 item 1b, owner
//! decision 2026-10-13): the full Slovak ones first, then KJV; a partial one
//! (MIL, the Gospels only) behind "…"; NLT only as the line below — it is
//! fetched on demand from Tyndale (#826) and never a main translation.

/// The main row, in this order, when installed.
const MAIN_ORDER: [&str; 4] = ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"];
/// The "Pod ním" (secondary) row, in this order, when installed.
const SECONDARY_ORDER: [&str; 2] = ["eng-nlt", "eng-kjv"];
/// Remote translations: never offered as the main one.
const REMOTE: [&str; 1] = ["eng-nlt"];

/// The bar's three groups for the installed translation codes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranslationChips {
    pub main: Vec<String>,
    /// Installed but not in the main row (partial or new ones), behind "…".
    pub more: Vec<String>,
    pub secondary: Vec<String>,
}

pub fn translation_chips(installed: &[String]) -> TranslationChips {
    let has = |code: &str| installed.iter().any(|c| c == code);
    let ordered = |order: &[&str]| -> Vec<String> {
        order
            .iter()
            .filter(|code| has(code))
            .map(|code| (*code).to_string())
            .collect()
    };
    TranslationChips {
        main: ordered(&MAIN_ORDER),
        more: installed
            .iter()
            .filter(|code| !MAIN_ORDER.contains(&code.as_str()) && !REMOTE.contains(&code.as_str()))
            .cloned()
            .collect(),
        secondary: ordered(&SECONDARY_ORDER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn slovak_first_kjv_last_mil_behind_more_and_nlt_only_below() {
        // The order `/bible/translations` answers in (by name).
        let installed = codes(&[
            "eng-kjv", "slk-mil", "slk-roh", "slk-seb", "slk-sevp", "eng-nlt",
        ]);
        assert_eq!(
            translation_chips(&installed),
            TranslationChips {
                main: codes(&["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"]),
                more: codes(&["slk-mil"]),
                secondary: codes(&["eng-nlt", "eng-kjv"]),
            }
        );
    }

    #[test]
    fn a_site_without_some_translations_shows_only_what_it_has() {
        let chips = translation_chips(&codes(&["slk-sevp", "eng-kjv", "deu-lut"]));
        assert_eq!(chips.main, codes(&["slk-sevp", "eng-kjv"]));
        assert_eq!(chips.more, codes(&["deu-lut"]));
        assert_eq!(chips.secondary, codes(&["eng-kjv"]));
    }
}
