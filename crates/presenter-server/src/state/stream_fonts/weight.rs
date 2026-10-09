#[cfg(test)]
mod tests {
    use super::*;

    /// Style names where only the typographic subfamily (name 17) is set.
    fn sub17(style: &str) -> StyleNames<'_> {
        StyleNames {
            family: "Nexa",
            typographic_subfamily: Some(style),
            ..StyleNames::default()
        }
    }

    fn weight_of(style: &str) -> Option<u16> {
        weight_from_style_name(&sub17(style)).map(WeightKeyword::weight)
    }

    #[test]
    fn every_style_keyword_maps_to_its_css_weight() {
        let cases: [(&str, u16); 20] = [
            ("Thin", 100),
            ("Hairline", 100),
            ("ExtraLight", 200),
            ("UltraLight", 200),
            ("Extra Light", 200),
            ("Light", 300),
            ("Book", 400),
            ("Regular", 400),
            ("Normal", 400),
            ("Medium", 500),
            ("SemiBold", 600),
            ("DemiBold", 600),
            ("Semi Bold", 600),
            ("Bold", 700),
            ("ExtraBold", 800),
            ("XBold", 800),
            ("UltraBold", 800),
            ("Extra-Bold", 800),
            ("Black", 900),
            ("Heavy", 900),
        ];
        for (style, weight) in cases {
            assert_eq!(weight_of(style), Some(weight), "style {style:?}");
        }
    }

    #[test]
    fn keywords_match_case_insensitively_and_ignore_the_italic_suffix() {
        assert_eq!(weight_of("XBOLD"), Some(800));
        assert_eq!(weight_of("black italic"), Some(900));
        assert_eq!(weight_of("Light Italic"), Some(300));
        assert_eq!(weight_of("Regular Italic"), Some(400));
        assert_eq!(weight_of("Bold Oblique"), Some(700));
    }

    #[test]
    fn heavy_and_black_stay_distinct_keywords() {
        assert_eq!(
            weight_from_style_name(&sub17("Heavy")),
            Some(WeightKeyword::Heavy)
        );
        assert_eq!(
            weight_from_style_name(&sub17("Black")),
            Some(WeightKeyword::Black)
        );
    }

    #[test]
    fn a_style_without_a_weight_keyword_maps_to_nothing() {
        assert_eq!(weight_of("Italic"), None);
        assert_eq!(weight_of("Condensed"), None);
        assert_eq!(weight_of(""), None);
    }

    #[test]
    fn typographic_subfamily_wins_over_the_subfamily() {
        // Nexa-Light: name 17 "Light", legacy name 2 "Regular".
        let names = StyleNames {
            family: "Nexa",
            typographic_subfamily: Some("Light"),
            subfamily: Some("Regular"),
            full_name: Some("Nexa-Light"),
            filename: Some("Fontfabric - Nexa-Light.otf"),
        };
        assert_eq!(weight_from_style_name(&names), Some(WeightKeyword::Light));
    }

    #[test]
    fn the_subfamily_is_used_when_there_is_no_typographic_subfamily() {
        // Nexa Bold Italic: no name 17, name 2 "Bold Italic".
        let names = StyleNames {
            family: "Nexa",
            subfamily: Some("Bold Italic"),
            full_name: Some("Nexa Bold Italic"),
            ..StyleNames::default()
        };
        assert_eq!(weight_from_style_name(&names), Some(WeightKeyword::Bold));
    }

    #[test]
    fn the_full_name_is_used_when_the_subfamilies_name_no_weight() {
        let names = StyleNames {
            family: "Nexa",
            subfamily: Some("Italic"),
            full_name: Some("Nexa Light Italic"),
            ..StyleNames::default()
        };
        assert_eq!(weight_from_style_name(&names), Some(WeightKeyword::Light));
    }

    #[test]
    fn the_filename_is_the_last_resort() {
        let names = StyleNames {
            family: "Nexa",
            subfamily: Some("Italic"),
            full_name: Some("Nexa Italic"),
            filename: Some("Fontfabric - Nexa-Black-Italic.otf"),
            ..StyleNames::default()
        };
        assert_eq!(weight_from_style_name(&names), Some(WeightKeyword::Black));
    }

    #[test]
    fn the_family_name_never_counts_as_a_style_keyword() {
        // A family whose own name holds a keyword ("Blackadder", "Lightfoot")
        // must not turn its full name / filename into a Black / Light face.
        for family in ["Blackadder ITC", "Lightfoot"] {
            let full = format!("{family} Italic");
            let file = format!("{family}-Italic.ttf");
            let names = StyleNames {
                family,
                subfamily: Some("Italic"),
                full_name: Some(&full),
                filename: Some(&file),
                ..StyleNames::default()
            };
            assert_eq!(weight_from_style_name(&names), None, "family {family:?}");
        }
    }

    #[test]
    fn a_default_os2_weight_yields_to_the_style_keyword() {
        assert_eq!(
            resolve_weight(Some(400), Some(WeightKeyword::Black)),
            (900, true)
        );
        assert_eq!(
            resolve_weight(None, Some(WeightKeyword::Light)),
            (300, true)
        );
    }

    #[test]
    fn an_os2_correct_face_is_left_alone() {
        // OS/2 700 says Bold; a stray keyword never overrides a non-default value.
        assert_eq!(
            resolve_weight(Some(700), Some(WeightKeyword::Black)),
            (700, false)
        );
        assert_eq!(
            resolve_weight(Some(300), Some(WeightKeyword::Regular)),
            (300, false)
        );
        // No keyword: the OS/2 value (or the 400 default) stands.
        assert_eq!(resolve_weight(Some(400), None), (400, false));
        assert_eq!(resolve_weight(None, None), (400, false));
    }

    fn face(weight: u16, keyword: Option<WeightKeyword>, from_style: bool) -> FaceWeight {
        FaceWeight {
            weight,
            keyword,
            from_style,
        }
    }

    #[test]
    fn heavy_moves_to_800_when_the_family_also_has_black() {
        let faces = [
            face(900, Some(WeightKeyword::Heavy), true),
            face(900, Some(WeightKeyword::Black), true),
        ];
        assert_eq!(family_weights(&faces), vec![800, 900]);
    }

    #[test]
    fn heavy_moves_to_850_when_800_is_taken_too() {
        // The Nexa set: XBold 800, Heavy and Black both style-derived 900, plus
        // their italics. Heavy at 800 would collide with XBold.
        let faces = [
            face(800, Some(WeightKeyword::ExtraBold), true),
            face(900, Some(WeightKeyword::Heavy), true),
            face(900, Some(WeightKeyword::Black), true),
            face(800, Some(WeightKeyword::ExtraBold), true),
            face(900, Some(WeightKeyword::Heavy), true),
            face(900, Some(WeightKeyword::Black), true),
            face(700, Some(WeightKeyword::Bold), false),
        ];
        assert_eq!(
            family_weights(&faces),
            vec![800, 850, 900, 800, 850, 900, 700]
        );
    }

    #[test]
    fn heavy_without_black_keeps_900() {
        let faces = [
            face(900, Some(WeightKeyword::Heavy), true),
            face(400, Some(WeightKeyword::Regular), true),
        ];
        assert_eq!(family_weights(&faces), vec![900, 400]);
    }

    #[test]
    fn an_os2_declared_heavy_is_never_moved() {
        let faces = [
            face(900, Some(WeightKeyword::Heavy), false),
            face(900, Some(WeightKeyword::Black), true),
        ];
        assert_eq!(family_weights(&faces), vec![900, 900]);
    }

    #[test]
    fn italic_is_read_from_the_style_names() {
        assert!(style_says_italic(&sub17("Black Italic")));
        assert!(style_says_italic(&StyleNames {
            family: "Nexa",
            subfamily: Some("Oblique"),
            ..StyleNames::default()
        }));
        assert!(!style_says_italic(&sub17("Black")));
        // Only the two subfamily names count, never the filename.
        assert!(!style_says_italic(&StyleNames {
            family: "Nexa",
            subfamily: Some("Regular"),
            filename: Some("Nexa-Italic.otf"),
            ..StyleNames::default()
        }));
    }
}
