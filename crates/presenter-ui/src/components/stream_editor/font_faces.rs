//! The uploaded faces of one font family, as the text-style form offers them
//! (#830). Pure + host-tested.
//!
//! A family like Nexa has many faces (Light, Regular, Bold, XBold, Heavy,
//! Black, plus italics). The weight `<select>` gets ONE option per weight,
//! labelled with the face's own style name ("Light 300", "Black 900") from
//! `StreamFont::style_name`. The Italic toggle is a separate checkbox, enabled
//! where the family has an italic face at the chosen weight.

use std::collections::BTreeMap;

use presenter_core::StreamFont;

/// One option of an uploaded family's weight `<select>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightOption {
    pub weight: u16,
    pub label: String,
}

/// The weight options of `family`, ascending, one per distinct weight. An
/// option is named by its upright face's style name; a weight with only an
/// italic face uses that name without "Italic"; a nameless face gets the
/// standard name of its weight, or just the number. `current` (the style's
/// stored weight) is added when no face has it, so the select never displays a
/// weight other than the one that is saved.
pub fn weight_options(fonts: &[StreamFont], family: &str, current: u16) -> Vec<WeightOption> {
    // weight → (upright face's style name, italic face's style name)
    let mut by_weight: BTreeMap<u16, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for font in fonts.iter().filter(|f| f.family == family) {
        let names = by_weight.entry(font.weight).or_default();
        let style = font.style_name.as_deref();
        if font.italic {
            names.1 = names.1.or(style);
        } else {
            names.0 = names.0.or(style);
        }
    }
    let mut options: Vec<WeightOption> = by_weight
        .into_iter()
        .map(|(weight, (upright, italic))| WeightOption {
            weight,
            label: weight_label(upright, italic, weight),
        })
        .collect();
    if !options.iter().any(|o| o.weight == current) {
        options.push(WeightOption {
            weight: current,
            label: format!("{current} (nenahraté)"),
        });
        options.sort_by_key(|o| o.weight);
    }
    options
}

/// True when `family` has an italic face at exactly `weight`.
pub fn has_italic_face(fonts: &[StreamFont], family: &str, weight: u16) -> bool {
    fonts
        .iter()
        .any(|f| f.family == family && f.italic && f.weight == weight)
}

/// "<name> <weight>", or the bare weight when no name is known.
fn weight_label(upright: Option<&str>, italic: Option<&str>, weight: u16) -> String {
    let name = upright
        .map(str::trim)
        .map(str::to_string)
        .or_else(|| italic.map(without_italic))
        .filter(|name| !name.is_empty())
        .or_else(|| standard_weight_name(weight).map(str::to_string));
    match name {
        Some(name) => format!("{name} {weight}"),
        None => weight.to_string(),
    }
}

/// An italic face's style name without its "Italic"/"Oblique" word
/// ("Light Italic" → "Light"; a bare "Italic" → "").
fn without_italic(style: &str) -> String {
    style
        .split_whitespace()
        .filter(|word| {
            !word.eq_ignore_ascii_case("italic") && !word.eq_ignore_ascii_case("oblique")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The OpenType name of a standard weight.
fn standard_weight_name(weight: u16) -> Option<&'static str> {
    Some(match weight {
        100 => "Thin",
        200 => "ExtraLight",
        300 => "Light",
        400 => "Regular",
        500 => "Medium",
        600 => "SemiBold",
        700 => "Bold",
        800 => "ExtraBold",
        900 => "Black",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, weight: u16, italic: bool, style: Option<&str>) -> StreamFont {
        StreamFont {
            id: i64::from(weight) * 2 + i64::from(italic),
            sha256: format!("{family}-{weight}-{italic}"),
            original_filename: format!("{family}.otf"),
            family: family.to_string(),
            weight,
            italic,
            format: "otf".to_string(),
            size_bytes: 1,
            style_name: style.map(str::to_string),
        }
    }

    /// The SNV Nexa family once its weights are re-derived (#830).
    fn nexa() -> Vec<StreamFont> {
        vec![
            face("Nexa", 900, true, Some("Black Italic")),
            face("Nexa", 700, true, Some("Bold Italic")),
            face("Nexa", 700, false, Some("Bold")),
            face("Nexa", 850, true, Some("Heavy Italic")),
            face("Nexa", 400, true, Some("Regular Italic")),
            face("Nexa", 800, true, Some("XBold Italic")),
            face("Nexa", 900, false, Some("Black")),
            face("Nexa", 850, false, Some("Heavy")),
            face("Nexa", 300, false, Some("Light")),
            face("Nexa", 400, false, Some("Regular")),
            face("Nexa", 800, false, Some("XBold")),
            face("Gruppo", 400, false, Some("Regular")),
        ]
    }

    fn labels(options: &[WeightOption]) -> Vec<(u16, &str)> {
        options
            .iter()
            .map(|o| (o.weight, o.label.as_str()))
            .collect()
    }

    #[test]
    fn every_weight_of_the_family_is_one_option_named_by_its_face() {
        assert_eq!(
            labels(&weight_options(&nexa(), "Nexa", 400)),
            vec![
                (300, "Light 300"),
                (400, "Regular 400"),
                (700, "Bold 700"),
                (800, "XBold 800"),
                (850, "Heavy 850"),
                (900, "Black 900"),
            ]
        );
    }

    #[test]
    fn a_weight_with_only_an_italic_face_drops_the_italic_word() {
        let fonts = vec![
            face("Solo", 300, true, Some("Light Italic")),
            face("Solo", 400, true, Some("Italic")),
        ];
        assert_eq!(
            labels(&weight_options(&fonts, "Solo", 300)),
            vec![(300, "Light 300"), (400, "Regular 400")]
        );
    }

    #[test]
    fn a_face_without_a_style_name_gets_the_standard_name_or_its_number() {
        let fonts = vec![
            face("Bare", 700, false, None),
            face("Bare", 850, false, None),
        ];
        assert_eq!(
            labels(&weight_options(&fonts, "Bare", 700)),
            vec![(700, "Bold 700"), (850, "850")]
        );
    }

    #[test]
    fn the_current_weight_stays_listed_when_no_face_has_it() {
        // An element saved at 700 in a family without a 700 face: the select
        // must still show 700, not silently fall back to the first option.
        let options = weight_options(&nexa(), "Gruppo", 700);
        assert_eq!(
            labels(&options),
            vec![(400, "Regular 400"), (700, "700 (nenahraté)")]
        );
    }

    #[test]
    fn italic_is_offered_only_where_the_family_has_an_italic_face() {
        let fonts = nexa();
        assert!(has_italic_face(&fonts, "Nexa", 900));
        assert!(has_italic_face(&fonts, "Nexa", 850));
        assert!(!has_italic_face(&fonts, "Nexa", 300), "no Light Italic");
        assert!(!has_italic_face(&fonts, "Gruppo", 400));
        assert!(!has_italic_face(&fonts, "Inter", 700), "built-in family");
    }

    fn families(options: &[FamilyOption]) -> Vec<(&str, &str)> {
        options
            .iter()
            .map(|o| (o.family.as_str(), o.label.as_str()))
            .collect()
    }

    #[test]
    fn family_options_list_built_ins_then_uploaded_families_once_each() {
        let mut fonts = nexa();
        // An uploaded face named like a built-in family is not listed twice.
        fonts.push(face("Inter", 400, false, Some("Regular")));
        let options = family_options(&fonts, "Nexa");
        let mut expected: Vec<(&str, &str)> =
            STREAM_FONT_FAMILIES.iter().map(|f| (*f, *f)).collect();
        expected.extend([("Gruppo", "Gruppo"), ("Nexa", "Nexa")]);
        assert_eq!(families(&options), expected);
    }

    #[test]
    fn a_stored_family_missing_from_the_list_stays_listed() {
        // The font list has not arrived yet (or the family was deleted): the
        // select must keep showing the stored family, not its first option.
        let options = family_options(&[], "Facet830");
        assert_eq!(
            options
                .last()
                .map(|o| (o.family.as_str(), o.label.as_str())),
            Some(("Facet830", "Facet830 (nenahraté)"))
        );
        assert_eq!(options.len(), STREAM_FONT_FAMILIES.len() + 1);
        // A built-in or an uploaded family needs no extra option.
        assert_eq!(
            family_options(&[], "Inter").len(),
            STREAM_FONT_FAMILIES.len()
        );
        assert_eq!(family_options(&[], "").len(), STREAM_FONT_FAMILIES.len());
    }
}
