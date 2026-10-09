//! A face's weight and italic flag from its STYLE NAME (#830). Pure.
//!
//! Some commercial families ship mislabelled files. Nexa (Fontfabric) on
//! SNV/PP declares `OS/2.usWeightClass` 400 for Light, Heavy, Black and XBold;
//! the real style is only in the name table ("Light", "Black", …). Taking the
//! weight from OS/2 alone made five faces collide on (Nexa, 400, normal), so
//! `/stream/fonts.css` served whichever `@font-face` rule came last.
//!
//! So when OS/2 says the default 400 (or there is no OS/2 table), the weight
//! comes from the first style name that names one: the typographic subfamily
//! (name 17), then the subfamily (2), the full name (4) and finally the
//! original filename. A non-default OS/2 weight is always trusted.
//! [`family_weights`] then keeps a family's Heavy and Black faces apart.

/// A weight word found in a style name, as the OpenType `usWeightClass`
/// table names them. Heavy and Black are kept apart (both 900) so
/// [`family_weights`] can separate them within a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WeightKeyword {
    Thin,
    ExtraLight,
    Light,
    Regular,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Heavy,
    Black,
}

impl WeightKeyword {
    /// The CSS / `usWeightClass` weight the word stands for.
    pub(crate) fn weight(self) -> u16 {
        match self {
            Self::Thin => 100,
            Self::ExtraLight => 200,
            Self::Light => 300,
            Self::Regular => 400,
            Self::Medium => 500,
            Self::SemiBold => 600,
            Self::Bold => 700,
            Self::ExtraBold => 800,
            Self::Heavy | Self::Black => 900,
        }
    }
}

/// The spellings, matched against a lower-cased name with spaces and
/// punctuation removed. Compound words come BEFORE the words they contain
/// (`extrabold` before `bold`, `extralight` before `light`).
const KEYWORDS: [(&str, WeightKeyword); 18] = [
    ("extralight", WeightKeyword::ExtraLight),
    ("ultralight", WeightKeyword::ExtraLight),
    ("xlight", WeightKeyword::ExtraLight),
    ("extrabold", WeightKeyword::ExtraBold),
    ("ultrabold", WeightKeyword::ExtraBold),
    ("xbold", WeightKeyword::ExtraBold),
    ("semibold", WeightKeyword::SemiBold),
    ("demibold", WeightKeyword::SemiBold),
    ("hairline", WeightKeyword::Thin),
    ("thin", WeightKeyword::Thin),
    ("light", WeightKeyword::Light),
    ("regular", WeightKeyword::Regular),
    ("normal", WeightKeyword::Regular),
    ("book", WeightKeyword::Regular),
    ("medium", WeightKeyword::Medium),
    ("black", WeightKeyword::Black),
    ("heavy", WeightKeyword::Heavy),
    ("bold", WeightKeyword::Bold),
];

/// The places a face names its style, in the order they are trusted.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StyleNames<'a> {
    /// The family the face belongs to (name 16 → 1). It is cut out of the full
    /// name and the filename, so "Blackadder Italic" is not a Black face.
    pub family: &'a str,
    /// Name 17, e.g. "Black Italic".
    pub typographic_subfamily: Option<&'a str>,
    /// Name 2, e.g. "Bold Italic" (or just "Italic" when name 17 holds the weight).
    pub subfamily: Option<&'a str>,
    /// Name 4, e.g. "Nexa Light Italic".
    pub full_name: Option<&'a str>,
    /// The uploaded file's name, e.g. "Fontfabric - Nexa-Black.otf".
    pub filename: Option<&'a str>,
}

impl StyleNames<'_> {
    /// The face's own style label for the editor: name 17, else name 2.
    pub(crate) fn display(&self) -> Option<String> {
        self.typographic_subfamily
            .or(self.subfamily)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
}

/// The weight word of the first style name that carries one (name 17, then 2,
/// then the full name, then the filename), or `None`.
pub(crate) fn weight_from_style_name(names: &StyleNames) -> Option<WeightKeyword> {
    let family = squash(names.family);
    let candidates = [
        names.typographic_subfamily.map(squash),
        names.subfamily.map(squash),
        names.full_name.map(|n| without_family(&squash(n), &family)),
        names
            .filename
            .map(|f| without_family(&squash(file_stem(f)), &family)),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|name| keyword_in(&name))
}

/// The face weight and whether it came from the style name: a non-default OS/2
/// weight is trusted; the default 400 (or no OS/2 table) yields to the keyword.
pub(crate) fn resolve_weight(
    os2_weight: Option<u16>,
    keyword: Option<WeightKeyword>,
) -> (u16, bool) {
    match (os2_weight, keyword) {
        (Some(weight), _) if weight != 400 => (weight, false),
        (_, Some(keyword)) => (keyword.weight(), true),
        (weight, None) => (weight.unwrap_or(400), false),
    }
}

/// True when name 17 or name 2 says Italic or Oblique (the filename never
/// counts: a family's files are often all named after the family).
pub(crate) fn style_says_italic(names: &StyleNames) -> bool {
    [names.typographic_subfamily, names.subfamily]
        .into_iter()
        .flatten()
        .map(squash)
        .any(|name| name.contains("italic") || name.contains("oblique"))
}

/// One face's weight as [`family_weights`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FaceWeight {
    pub weight: u16,
    pub keyword: Option<WeightKeyword>,
    /// `weight` came from `keyword` (see [`resolve_weight`]).
    pub from_style: bool,
}

/// The final weights of one family's faces, in the same order. Heavy and Black
/// both mean 900, so when the family has a Black face at 900, every Heavy face
/// whose weight came from its name moves below it: to 800, or to 850 when
/// another face already holds 800 (Nexa: XBold 800, Heavy 850, Black 900).
/// Upright and italic faces move together, so the Italic toggle keeps working.
pub(crate) fn family_weights(faces: &[FaceWeight]) -> Vec<u16> {
    let style_heavy = |f: &FaceWeight| f.from_style && f.keyword == Some(WeightKeyword::Heavy);
    let black_at_900 = faces
        .iter()
        .any(|f| f.keyword == Some(WeightKeyword::Black) && f.weight == 900);
    if !black_at_900 {
        return faces.iter().map(|f| f.weight).collect();
    }
    let taken_800 = faces.iter().any(|f| !style_heavy(f) && f.weight == 800);
    let heavy = if taken_800 { 850 } else { 800 };
    faces
        .iter()
        .map(|f| if style_heavy(f) { heavy } else { f.weight })
        .collect()
}

/// The first weight word in an already-squashed name.
fn keyword_in(squashed: &str) -> Option<WeightKeyword> {
    KEYWORDS
        .iter()
        .find(|(word, _)| squashed.contains(word))
        .map(|&(_, keyword)| keyword)
}

/// Lower-case letters and digits only: "Extra-Bold Italic" → "extrabolditalic".
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `name` with its (squashed) family name cut out, wherever it sits: a full
/// name starts with it, a filename may carry a vendor prefix first
/// ("Fontfabric - Nexa-Black").
fn without_family(name: &str, family: &str) -> String {
    if family.is_empty() {
        return name.to_string();
    }
    name.replacen(family, "", 1)
}

/// The filename without its extension.
fn file_stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(stem, _)| stem)
}

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
