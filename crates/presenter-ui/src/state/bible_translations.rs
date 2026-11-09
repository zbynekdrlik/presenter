//! Which translations a translation row shows and what its ▾ list holds
//! (#921, owner 2026-11-09: preferred translations in ONE row, every other
//! one behind a ▾, "keď budem mať 100 prekladov, celá plocha budú tlačidlá").
//! Used by the console's bar (Hlavný / Druhý preklad) and Study's favourites
//! row. The row never grows: at most `cap` chips, and a selected translation
//! outside the preferred ones takes the last chip's place, so the row always
//! says what is on. The ▾ list holds every installed translation, grouped by
//! language. NLT is fetched on demand from Tyndale (#826) and is never the
//! console's main translation.

use presenter_core::BibleTranslation;

/// The known translations, in the order a list shows them.
const ORDER: [&str; 10] = [
    "slk-seb", "slk-roh", "slk-sevp", "slk-ssv", "slk-bot", "slk-mil", "ces-csp", "eng-kjv",
    "eng-ampc", "eng-nlt",
];
/// Remote translations: never offered as the console's main one.
pub const REMOTE: [&str; 1] = ["eng-nlt"];

/// Chips the console's main row holds (the second row one less: "žiadny"
/// takes a place). Measured at the narrowest panels (#921): a phone's row is
/// 330 px next to its inline label, a desktop's 284 px.
pub fn console_chip_cap(phone: bool, secondary: bool) -> usize {
    let main = if phone { 3 } else { 4 };
    if secondary {
        main - 1
    } else {
        main
    }
}

/// Language group of a translation: Slovak, Czech, English, then the rest.
fn language_rank(translation: &BibleTranslation) -> u8 {
    let language = translation.language.to_ascii_lowercase();
    let prefix = translation.code.split('-').next().unwrap_or("");
    match (language.as_str(), prefix) {
        ("sk" | "slk", _) | (_, "slk") => 0,
        ("cs" | "ces", _) | (_, "ces") => 1,
        ("en" | "eng", _) | (_, "eng") => 2,
        _ => 3,
    }
}

const GROUP_LABELS: [&str; 4] = ["Slovenské", "České", "Anglické", "Ďalšie"];

/// One heading of a ▾ list and its translations, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationGroup {
    pub label: &'static str,
    pub codes: Vec<String>,
}

/// The installed translations in list order: by language group, the known
/// ones in [`ORDER`] first, an unknown one after them by code.
fn sorted(installed: &[BibleTranslation]) -> Vec<&BibleTranslation> {
    let mut list: Vec<&BibleTranslation> = installed.iter().collect();
    list.sort_by_key(|t| {
        let known = ORDER
            .iter()
            .position(|code| *code == t.code)
            .unwrap_or(ORDER.len());
        (language_rank(t), known, t.code.clone())
    });
    list.dedup_by(|a, b| a.code == b.code);
    list
}

/// Every installed translation `offered` lets in, grouped by language for a
/// ▾ list (empty groups left out).
pub fn translation_groups(
    installed: &[BibleTranslation],
    offered: impl Fn(&str) -> bool,
) -> Vec<TranslationGroup> {
    let mut groups: Vec<TranslationGroup> = Vec::new();
    for translation in sorted(installed) {
        if !offered(&translation.code) {
            continue;
        }
        let label = GROUP_LABELS[usize::from(language_rank(translation))];
        match groups.last_mut() {
            Some(group) if group.label == label => group.codes.push(translation.code.clone()),
            _ => groups.push(TranslationGroup {
                label,
                codes: vec![translation.code.clone()],
            }),
        }
    }
    groups
}

/// The chips of one row: the preferred translations that are installed and
/// `offered`, in the preferred order, at most `cap`; a selected translation
/// outside them replaces the last one (or fills a free place).
pub fn row_chips(
    preferred: &[String],
    installed: &[BibleTranslation],
    selected: Option<&str>,
    cap: usize,
    offered: impl Fn(&str) -> bool,
) -> Vec<String> {
    let has = |code: &str| installed.iter().any(|t| t.code == code);
    let mut chips: Vec<String> = Vec::new();
    for code in preferred {
        if chips.len() == cap {
            break;
        }
        if has(code) && offered(code) && !chips.contains(code) {
            chips.push(code.clone());
        }
    }
    if let Some(code) = selected.filter(|code| has(code) && cap > 0) {
        if !chips.iter().any(|c| c == code) {
            if chips.len() == cap {
                chips.pop();
            }
            chips.push(code.to_string());
        }
    }
    chips
}

/// Study's favourites row (#921): the `(slot, translation)` chips
/// `visible_favourites` gives, at most `cap`, the open translation in the
/// last place when it is none of them (that chip keeps the slot it took, so a
/// long press there still customises the slot).
pub fn favourite_row(
    chips: Vec<(usize, String)>,
    selected: Option<&str>,
    cap: usize,
) -> Vec<(usize, String)> {
    let mut row: Vec<(usize, String)> = chips.into_iter().take(cap).collect();
    if let Some(code) = selected {
        if !row.iter().any(|(_, c)| c == code) {
            match row.last_mut() {
                Some(last) => last.1 = code.to_string(),
                None if cap > 0 => row.push((0, code.to_string())),
                None => {}
            }
        }
    }
    row
}

/// Favourite chips Study's row holds: four on a narrow phone, so the field
/// keeps its width next to ▾, five otherwise.
pub fn study_chip_cap(narrow: bool) -> usize {
    if narrow {
        4
    } else {
        5
    }
}

/// The console's main row and list leave out a remote translation unless it
/// is the selected main one (a stored preference keeps its chip).
pub fn offered_as_main(code: &str, selected_main: Option<&str>) -> bool {
    !REMOTE.contains(&code) || selected_main == Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(code: &str, language: &str) -> BibleTranslation {
        BibleTranslation {
            code: code.to_string(),
            name: code.to_string(),
            language: language.to_string(),
            show_in_dashboard: true,
            source: None,
        }
    }

    /// The SNV list in the order `/bible/translations` answers (by name).
    fn snv() -> Vec<BibleTranslation> {
        [
            ("eng-ampc", "en"),
            ("slk-bot", "sk"),
            ("slk-ssv", "sk"),
            ("eng-kjv", "en"),
            ("slk-mil", "sk"),
            ("slk-roh", "sk"),
            ("slk-seb", "sk"),
            ("slk-sevp", "sk"),
            ("ces-csp", "cs"),
            ("eng-nlt", "en"),
        ]
        .iter()
        .map(|(code, language)| translation(code, language))
        .collect()
    }

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn the_list_holds_every_translation_grouped_by_language() {
        let groups = translation_groups(&snv(), all);
        let labels: Vec<&str> = groups.iter().map(|g| g.label).collect();
        assert_eq!(labels, ["Slovenské", "České", "Anglické"]);
        assert_eq!(
            groups[0].codes,
            codes(&["slk-seb", "slk-roh", "slk-sevp", "slk-ssv", "slk-bot", "slk-mil"])
        );
        assert_eq!(groups[1].codes, codes(&["ces-csp"]));
        assert_eq!(groups[2].codes, codes(&["eng-kjv", "eng-ampc", "eng-nlt"]));
    }

    #[test]
    fn the_main_list_has_no_nlt_unless_it_is_the_main() {
        let main_list = translation_groups(&snv(), |c| offered_as_main(c, Some("slk-seb")));
        assert!(!main_list
            .iter()
            .any(|g| g.codes.iter().any(|c| c == "eng-nlt")));
        let stored = translation_groups(&snv(), |c| offered_as_main(c, Some("eng-nlt")));
        assert!(stored
            .iter()
            .any(|g| g.codes.iter().any(|c| c == "eng-nlt")));
    }

    #[test]
    fn an_unknown_translation_joins_its_language_group() {
        let installed = vec![
            translation("deu-lut", "de"),
            translation("eng-web", "en"),
            translation("slk-new", "sk"),
            translation("eng-kjv", "en"),
            translation("ces-b21", "cs"),
        ];
        let groups = translation_groups(&installed, all);
        assert_eq!(groups[0].codes, codes(&["slk-new"]));
        assert_eq!(groups[2].codes, codes(&["eng-kjv", "eng-web"]));
        assert_eq!(groups[3].label, "Ďalšie");
        assert_eq!(groups[3].codes, codes(&["deu-lut"]));
    }

    #[test]
    fn the_row_shows_the_preferred_ones_up_to_its_cap() {
        let preferred = codes(&["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"]);
        assert_eq!(
            row_chips(&preferred, &snv(), Some("slk-seb"), 4, all),
            preferred
        );
        // A phone holds three.
        assert_eq!(
            row_chips(&preferred, &snv(), Some("slk-roh"), 3, all),
            codes(&["slk-seb", "slk-roh", "slk-sevp"])
        );
        // A preferred translation that is not installed is skipped, the next one moves up.
        let site = vec![translation("slk-roh", "sk"), translation("eng-kjv", "en")];
        assert_eq!(
            row_chips(&preferred, &site, None, 4, all),
            codes(&["slk-roh", "eng-kjv"])
        );
    }

    #[test]
    fn a_selected_translation_outside_takes_the_last_place() {
        let preferred = codes(&["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"]);
        assert_eq!(
            row_chips(&preferred, &snv(), Some("ces-csp"), 4, all),
            codes(&["slk-seb", "slk-roh", "slk-sevp", "ces-csp"])
        );
        // With a free place it is added, never over the cap.
        let short = codes(&["eng-nlt", "eng-kjv"]);
        assert_eq!(
            row_chips(&short, &snv(), Some("eng-ampc"), 3, all),
            codes(&["eng-nlt", "eng-kjv", "eng-ampc"])
        );
        assert_eq!(
            row_chips(&short, &snv(), Some("eng-ampc"), 2, all),
            codes(&["eng-nlt", "eng-ampc"])
        );
        // Not installed: nothing changes.
        assert_eq!(row_chips(&short, &snv(), Some("deu-lut"), 2, all), short);
    }

    #[test]
    fn the_main_row_never_offers_nlt_unless_stored() {
        let preferred = codes(&["eng-nlt", "slk-seb"]);
        let offered = |c: &str| offered_as_main(c, Some("slk-seb"));
        assert_eq!(
            row_chips(&preferred, &snv(), Some("slk-seb"), 4, offered),
            codes(&["slk-seb"])
        );
        let stored = |c: &str| offered_as_main(c, Some("eng-nlt"));
        assert_eq!(
            row_chips(&preferred, &snv(), Some("eng-nlt"), 4, stored),
            codes(&["eng-nlt", "slk-seb"])
        );
    }

    #[test]
    fn study_shows_the_open_translation_in_the_last_favourite_place() {
        let chips: Vec<(usize, String)> = ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv", "eng-nlt"]
            .iter()
            .enumerate()
            .map(|(slot, code)| (slot, code.to_string()))
            .collect();
        // A favourite is open: the row is the favourites, up to the cap.
        assert_eq!(favourite_row(chips.clone(), Some("slk-roh"), 5), chips);
        assert_eq!(favourite_row(chips.clone(), Some("slk-roh"), 4).len(), 4);
        // AMPC is open: it takes the last place (slot kept), never a sixth chip.
        let row = favourite_row(chips.clone(), Some("eng-ampc"), 5);
        assert_eq!(row.len(), 5);
        assert_eq!(row[4], (4, "eng-ampc".to_string()));
        let narrow = favourite_row(chips.clone(), Some("ces-csp"), 4);
        assert_eq!(narrow.last(), Some(&(3, "ces-csp".to_string())));
        // The fifth favourite open on a narrow phone is outside the four shown.
        let fifth = favourite_row(chips, Some("eng-nlt"), 4);
        assert_eq!(fifth.last(), Some(&(3, "eng-nlt".to_string())));
        assert_eq!(study_chip_cap(true), 4);
        assert_eq!(study_chip_cap(false), 5);
    }

    #[test]
    fn the_second_row_leaves_a_place_for_none() {
        assert_eq!(console_chip_cap(true, false), 3);
        assert_eq!(console_chip_cap(true, true), 2);
        assert_eq!(console_chip_cap(false, false), 4);
        assert_eq!(console_chip_cap(false, true), 3);
    }
}
