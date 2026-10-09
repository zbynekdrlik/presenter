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
}
