//! #842: derive an output slug from the name the operator types.
//!
//! The editor's „Nový výstup" form asks only for a display name. The slug (the
//! `/stream/{slug}` address OBS and Companion use) is derived from it and never
//! changes afterwards, so renaming an output keeps every Companion button and
//! OBS browser source working.

use super::STREAM_SLUG_MAX;
use crate::search::normalise_for_search;

/// The slug for an output named `name`: diacritics folded and lowercased
/// ("Moderátor" → "moderator"), ASCII letters and digits kept, every other run
/// of characters turned into one `-`, no `-` at either end, at most
/// [`STREAM_SLUG_MAX`] bytes. The result can still be empty or reserved, so
/// check it with [`super::validate_slug`] before using it.
pub fn slug_from_name(name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for ch in normalise_for_search(name).chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(ch);
        } else {
            separator = true;
        }
    }
    // ASCII only, so any byte index is a char boundary.
    slug.truncate(STREAM_SLUG_MAX);
    slug.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::super::validate_slug;
    use super::*;

    #[test]
    fn a_name_becomes_a_lowercase_ascii_slug() {
        assert_eq!(slug_from_name("Strih"), "strih");
        assert_eq!(slug_from_name("Moderátor 2"), "moderator-2");
        assert_eq!(slug_from_name("  Ľudová   hudba!! "), "ludova-hudba");
        assert_eq!(slug_from_name("Kázeň / Live"), "kazen-live");
        assert_eq!(slug_from_name("stream-2"), "stream-2");
    }

    #[test]
    fn the_derived_slug_passes_validation() {
        for name in ["Strih", "Moderátor 2", "Kázeň / Live", "Stream PP"] {
            let slug = slug_from_name(name);
            assert!(validate_slug(&slug).is_ok(), "{name:?} -> {slug:?}");
        }
    }

    #[test]
    fn a_name_without_letters_or_digits_gives_an_empty_slug() {
        assert_eq!(slug_from_name("***"), "");
        assert_eq!(slug_from_name("  "), "");
        assert!(validate_slug(&slug_from_name("***")).is_err());
    }

    #[test]
    fn a_reserved_name_is_left_to_validation() {
        assert_eq!(slug_from_name("API"), "api");
        assert!(validate_slug("api").is_err());
    }

    #[test]
    fn a_long_name_is_cut_to_the_maximum_without_a_trailing_dash() {
        let name = format!("{} {}", "a".repeat(STREAM_SLUG_MAX - 1), "b".repeat(10));
        let slug = slug_from_name(&name);
        assert!(slug.len() <= STREAM_SLUG_MAX, "{}", slug.len());
        assert!(!slug.ends_with('-'), "{slug}");
        assert!(validate_slug(&slug).is_ok());
    }
}
