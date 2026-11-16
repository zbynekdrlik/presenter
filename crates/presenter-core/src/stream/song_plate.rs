//! #956: the text of the song lower third ("menovka piesne").
//!
//! The plate names the song selected in Presenter — the title of the
//! presentation on stage, with diacritics. AbleSet plays no role (#962, owner
//! 2026-11-16), not even as a fallback. The title carries the worship setlist
//! number in front ("040 Žalm 8", "100. Nad všetko si ty", "137, Tancuj
//! generácia"); the plate leaves it out. One function, so the on-air plate and
//! the Companion button label can never disagree.

/// Most digits a leading setlist number has ("040", "1001").
const SETLIST_NUMBER_MAX_DIGITS: usize = 4;

/// Characters that may follow the setlist number before the space ("100. ",
/// "137, ", "12 - ").
const SETLIST_NUMBER_SEPARATORS: &[char] = &['.', ',', '-', '–', ':'];

/// The song plate's primary text: the selected presentation's title without
/// the leading setlist number. `None` when no presentation is selected or its
/// title is blank.
pub fn song_plate_title(presentation_name: Option<&str>) -> Option<String> {
    let source = presentation_name.filter(|name| !name.trim().is_empty())?;
    Some(strip_setlist_number(source.trim()).to_string())
}

/// `title` without ONE leading setlist number: 1–4 digits followed by
/// whitespace, or by one separator (`.` `,` `-` `–` `:`) and whitespace or a
/// letter. Digits anywhere else stay ("102 10 000 armád" → "10 000 armád"),
/// and a title that would become empty is returned unchanged ("040", "040 ").
/// Accepted limit: a title whose OWN first word is a short number and that
/// has no setlist number would lose it — none of the prod titles does.
fn strip_setlist_number(title: &str) -> &str {
    let digits = title.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > SETLIST_NUMBER_MAX_DIGITS {
        return title;
    }
    // ASCII digits, so `digits` is a char boundary.
    let after_number = &title[digits..];
    let after_space = after_number.trim_start();
    let after_separator = match after_space.strip_prefix(SETLIST_NUMBER_SEPARATORS) {
        // The separator must be followed by whitespace or a letter ("233.Boh
        // je tu" is on prod): "1.5 GHz" and "15.2. Kázeň" are titles.
        Some(rest) if rest.starts_with(|c: char| c.is_whitespace() || c.is_alphabetic()) => rest,
        Some(_) => return title,
        // No separator: the number must be followed by whitespace ("040 Žalm").
        None if after_space.len() < after_number.len() => after_space,
        None => return title,
    };
    let rest = after_separator.trim_start();
    if rest.is_empty() {
        title
    } else {
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_every_prod_number_form() {
        assert_eq!(strip_setlist_number("040 Žalm 8"), "Žalm 8");
        assert_eq!(
            strip_setlist_number("100. Nad všetko si ty"),
            "Nad všetko si ty"
        );
        assert_eq!(
            strip_setlist_number("137, Tancuj generácia"),
            "Tancuj generácia"
        );
        assert_eq!(strip_setlist_number("12 - Haleluja"), "Haleluja");
        assert_eq!(strip_setlist_number("7 Sláva"), "Sláva");
        assert_eq!(strip_setlist_number("1001   Pieseň"), "Pieseň");
        assert_eq!(
            strip_setlist_number("233.Tancujme, tancujme"),
            "Tancujme, tancujme"
        );
        assert_eq!(strip_setlist_number("248.Boh je tu"), "Boh je tu");
    }

    #[test]
    fn strips_only_one_leading_number() {
        assert_eq!(strip_setlist_number("102 10 000 armád"), "10 000 armád");
        assert_eq!(strip_setlist_number("Žalm 8"), "Žalm 8");
        assert_eq!(
            strip_setlist_number("418 Ja budem rásť - Žalm 1"),
            "Ja budem rásť - Žalm 1"
        );
    }

    #[test]
    fn keeps_titles_that_are_not_a_setlist_number() {
        // Five digits is not a setlist number.
        assert_eq!(strip_setlist_number("10000 dôvodov"), "10000 dôvodov");
        // Number glued to a word, or a separator without a space.
        assert_eq!(strip_setlist_number("3dni"), "3dni");
        assert_eq!(strip_setlist_number("1.5 GHz"), "1.5 GHz");
        assert_eq!(strip_setlist_number("15.2. Kázeň"), "15.2. Kázeň");
        // Nothing would be left.
        assert_eq!(strip_setlist_number("040"), "040");
        assert_eq!(strip_setlist_number("040 "), "040 ");
        assert_eq!(strip_setlist_number("040. "), "040. ");
        assert_eq!(strip_setlist_number(""), "");
    }

    #[test]
    fn the_plate_names_the_selected_presentation() {
        assert_eq!(
            song_plate_title(Some("040 Žalm 8")).as_deref(),
            Some("Žalm 8")
        );
    }

    #[test]
    fn nothing_on_stage_is_none() {
        assert_eq!(song_plate_title(None), None);
        assert_eq!(song_plate_title(Some("")), None);
        assert_eq!(song_plate_title(Some("  ")), None);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            song_plate_title(Some("  040 Žalm 8 ")).as_deref(),
            Some("Žalm 8")
        );
    }
}
