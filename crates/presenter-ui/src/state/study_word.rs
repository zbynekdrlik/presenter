//! The KJV word sheet and the Strong's view, the pure half (#851 spec §6.6):
//! which word a URL names, ◀ ▶ through the tagged words, what a verse marks,
//! the scope chips and how the field reads a number. No `web_sys` here,
//! host-tested with `cargo test --lib`.

use presenter_core::bible::resolver::Step;
use presenter_core::bible::{normalize_strongs, BookScope, MarkupSegment, SegmentKind};

use crate::state::study::StudyVerse;

/// The one translation with Strong's numbers: the Strong's view reads it.
pub const STRONGS_TRANSLATION: &str = "eng-kjv";
/// localStorage key of the "Čísla" chip, after `presenter:`.
pub const KEY_NUMBERS: &str = "study.strongsNumbers";
/// A page of the Strong's view (spec §6.6), the server's maximum.
pub const OCCURRENCES_PAGE: usize = 100;
/// The whole Bible, the scope a Strong's view opens with.
pub const SCOPE_ALL: &str = "all";

/// A tagged word: `w=G4102&wv=1.3`. `strongs` is the entry on screen: one of
/// the word's numbers, or one a derivation link led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordPlace {
    pub strongs: String,
    pub verse: u16,
    /// The segment's index in the verse's markup.
    pub token: usize,
}

impl WordPlace {
    /// `w` and `wv` of a URL; anything broken names no word.
    pub fn parse(strongs: &str, place: &str) -> Option<Self> {
        let (verse, token) = place.split_once('.')?;
        Some(Self {
            strongs: normalize_strongs(strongs)?,
            verse: verse.parse().ok().filter(|verse| *verse > 0)?,
            token: token.parse().ok()?,
        })
    }

    pub fn to_params(&self) -> [String; 2] {
        [
            format!("w={}", self.strongs),
            format!("wv={}.{}", self.verse, self.token),
        ]
    }

    /// The same word on another of its numbers, or on a linked entry.
    pub fn with_strongs(&self, strongs: &str) -> Self {
        Self {
            strongs: strongs.to_string(),
            ..self.clone()
        }
    }
}

/// What the sheet needs from the text on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordInfo {
    /// The word as printed.
    pub text: String,
    /// All its numbers: one tab each.
    pub numbers: Vec<String>,
    pub previous: Option<WordPlace>,
    pub next: Option<WordPlace>,
}

fn tagged(segment: &MarkupSegment) -> bool {
    segment.kind == SegmentKind::Text && !segment.strongs.is_empty()
}

/// Every tagged word of these verses in reading order: `(verse, token, segment)`.
fn tagged_words<'a>(
    verses: &'a [StudyVerse],
    only: Option<u16>,
) -> impl Iterator<Item = (u16, usize, &'a MarkupSegment)> + 'a {
    verses
        .iter()
        .filter(move |verse| only.is_none_or(|wanted| wanted == verse.verse))
        .filter_map(|verse| Some((verse.verse, verse.markup.as_ref()?)))
        .flat_map(|(verse, markup)| {
            markup
                .0
                .iter()
                .enumerate()
                .filter(|(_, segment)| tagged(segment))
                .map(move |(token, segment)| (verse, token, segment))
        })
}

/// The word a place names among the words on the page. `only` = the one verse
/// a phone's compare view shows. `None`: no such word here, so no sheet. ◀ ▶
/// walk these words and stop at the ends: the sheet never leaves the page.
pub fn find_word(verses: &[StudyVerse], place: &WordPlace, only: Option<u16>) -> Option<WordInfo> {
    let words: Vec<_> = tagged_words(verses, only).collect();
    let index = words
        .iter()
        .position(|(verse, token, _)| (*verse, *token) == (place.verse, place.token))?;
    let (_, _, segment) = words[index];
    let neighbour = |found: Option<&(u16, usize, &MarkupSegment)>| {
        let (verse, token, segment) = found?;
        Some(WordPlace {
            strongs: segment.strongs.first()?.clone(),
            verse: *verse,
            token: *token,
        })
    };
    Some(WordInfo {
        text: segment.text.clone(),
        numbers: segment.strongs.clone(),
        previous: neighbour(index.checked_sub(1).and_then(|before| words.get(before))),
        next: neighbour(words.get(index + 1)),
    })
}

/// Does this text have anything for the "Čísla" chip to show?
pub fn has_numbers(verses: &[StudyVerse]) -> bool {
    tagged_words(verses, None).next().is_some()
}

/// One step of ◀ ▶.
pub fn neighbour_word(info: &WordInfo, dir: Step) -> Option<&WordPlace> {
    match dir {
        Step::Prev => info.previous.as_ref(),
        Step::Next => info.next.as_ref(),
    }
}

/// The verse's text cut at the words carrying `number`; `true` = marked. Only
/// the verse's Text segments: a note or a heading is no part of a hit.
pub fn marked_segments(
    text: &str,
    verse: Option<&[MarkupSegment]>,
    number: &str,
) -> Vec<(String, bool)> {
    let Some(segments) = verse else {
        return vec![(text.to_string(), false)];
    };
    let mut parts: Vec<(String, bool)> = Vec::new();
    for segment in segments.iter().filter(|segment| segment.kind.is_text()) {
        let marked = segment.strongs.iter().any(|held| held == number);
        match parts.last_mut() {
            Some((held, was)) if *was == marked => held.push_str(&segment.text),
            _ => parts.push((segment.text.clone(), marked)),
        }
    }
    parts
}

/// The definition's first sentence (spec §6.6), where the peek stops: at a `;`
/// or a `.` that ends a sentence, never at the dots of `i.e.`.
pub fn first_sentence(definition: &str) -> &str {
    const ABBREVIATIONS: [&str; 5] = ["i.e", "e.g", "etc", "comp", "viz"];
    let text = definition.trim();
    for (index, c) in text.char_indices() {
        if c != ';' && c != '.' {
            continue;
        }
        let after = &text[index + c.len_utf8()..];
        if !after.is_empty() && !after.starts_with(char::is_whitespace) {
            continue;
        }
        let before = &text[..index];
        if c == '.' && ABBREVIATIONS.iter().any(|short| before.ends_with(short)) {
            continue;
        }
        return before;
    }
    text
}

/// A derivation cut at its numbers: `(piece, Some(number))` is a link.
pub fn linked_parts(text: &str) -> Vec<(String, Option<String>)> {
    let mut parts: Vec<(String, Option<String>)> = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    let mut previous = None;
    while let Some(c) = rest.chars().next() {
        let digits = rest[c.len_utf8()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
        let end = c.len_utf8() + digits;
        let closes = !rest[end..].starts_with(|next: char| next.is_alphanumeric());
        let opens = !previous.is_some_and(char::is_alphanumeric);
        if matches!(c, 'G' | 'H') && (1..=4).contains(&digits) && closes && opens {
            if !plain.is_empty() {
                parts.push((std::mem::take(&mut plain), None));
            }
            let piece = &rest[..end];
            parts.push((piece.to_string(), normalize_strongs(piece)));
            previous = rest[..end].chars().next_back();
            rest = &rest[end..];
            continue;
        }
        plain.push(c);
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    if !plain.is_empty() {
        parts.push((plain, None));
    }
    parts
}

/// The form a scope has in a URL and on the server: `all`, `ot`, `nt` or a book
/// code. Anything else is the whole Bible.
pub fn normalize_scope(scope: &str) -> String {
    let scope = scope.trim();
    match BookScope::parse(scope) {
        Some(BookScope::OldTestament) => "ot".to_string(),
        Some(BookScope::NewTestament) => "nt".to_string(),
        Some(BookScope::Book(_)) => scope.to_ascii_uppercase(),
        Some(BookScope::All) | None => SCOPE_ALL.to_string(),
    }
}

/// A scope chip of the Strong's view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopeChip {
    /// `data-scope`, the URL's value.
    pub scope: String,
    /// `None` = a book: the page names it from the translation's structure.
    pub label: Option<&'static str>,
    pub count: u64,
}

/// `(book code, verses)` of a number, whole Bible (`perBook`): Celá Biblia, SZ,
/// NZ and each book, with their counts. A chip with nothing behind it would be
/// a dead button, so it stays only as the scope the URL already names.
pub fn scope_chips(per_book: &[(String, u64)], current: &str) -> Vec<ScopeChip> {
    let within = |scope: BookScope| {
        let (first, last) = scope.book_numbers();
        per_book
            .iter()
            .filter(|(code, _)| {
                matches!(
                    BookScope::parse(code),
                    Some(BookScope::Book(number)) if (first..=last).contains(&number)
                )
            })
            .map(|(_, count)| count)
            .sum::<u64>()
    };
    let mut chips = vec![
        ScopeChip {
            scope: SCOPE_ALL.to_string(),
            label: Some("Celá Biblia"),
            count: within(BookScope::All),
        },
        ScopeChip {
            scope: "ot".to_string(),
            label: Some("SZ"),
            count: within(BookScope::OldTestament),
        },
        ScopeChip {
            scope: "nt".to_string(),
            label: Some("NZ"),
            count: within(BookScope::NewTestament),
        },
    ];
    chips.extend(per_book.iter().map(|(code, count)| ScopeChip {
        scope: code.clone(),
        label: None,
        count: *count,
    }));
    if !chips.iter().any(|chip| chip.scope == current) {
        chips.push(ScopeChip {
            scope: current.to_string(),
            label: None,
            count: 0,
        });
    }
    chips.retain(|chip| chip.count > 0 || chip.scope == current || chip.scope == SCOPE_ALL);
    chips
}

/// `G4102:all`, the Strong's view's settle marker.
pub fn occurrences_key(number: &str, scope: &str) -> String {
    format!("{number}:{scope}")
}

/// `Výskyty: G4102 · KJV`, its history row.
pub fn strongs_label(number: &str) -> String {
    format!("Výskyty: {number} · KJV")
}

/// How the field reads a Strong's number (spec §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrongsOffer {
    pub number: String,
    /// `G26` opens it; `g 26` stays the resolver's, the number is the alternative.
    pub first: bool,
}

/// `^[GgHh]\d{1,4}$` comes first; with a space between, the resolver does.
pub fn strongs_offer(input: &str) -> Option<StrongsOffer> {
    let input = input.trim();
    let mut chars = input.chars();
    let letter = chars.next().filter(|c| "GgHh".contains(*c))?;
    let rest = chars.as_str();
    let digits = rest.trim_start();
    let valid = (1..=4).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit());
    if !valid {
        return None;
    }
    Some(StrongsOffer {
        number: normalize_strongs(&format!("{letter}{digits}"))?,
        first: rest.len() == digits.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::bible::VerseMarkup;

    fn word(text: &str, numbers: &[&str]) -> MarkupSegment {
        MarkupSegment {
            strongs: numbers.iter().map(ToString::to_string).collect(),
            ..MarkupSegment::text(text)
        }
    }

    fn verse(number: u16, segments: Vec<MarkupSegment>) -> StudyVerse {
        StudyVerse {
            verse: number,
            text: VerseMarkup(segments.clone()).plain_text(),
            markup: Some(VerseMarkup(segments)),
        }
    }

    fn chapter() -> Vec<StudyVerse> {
        vec![
            verse(
                1,
                vec![
                    MarkupSegment::heading("Title"),
                    word("Now", &["G1161"]),
                    MarkupSegment::text(" "),
                    word("faith", &["G4102", "G9999"]),
                ],
            ),
            StudyVerse {
                verse: 2,
                text: "Plain.".to_string(),
                markup: None,
            },
            verse(3, vec![word("hope", &["G1680"])]),
        ]
    }

    fn at(verse: u16, token: usize) -> WordPlace {
        WordPlace {
            strongs: "G1".to_string(),
            verse,
            token,
        }
    }

    #[test]
    fn a_word_place_round_trips_and_a_broken_one_is_none() {
        let place = WordPlace::parse("g4102", "1.3").expect("a word");
        assert_eq!(place.strongs, "G4102");
        assert_eq!(place.to_params(), ["w=G4102", "wv=1.3"]);
        for (strongs, position) in [("x", "1.3"), ("G1", "1"), ("G1", "0.3"), ("G1", "1.x")] {
            assert_eq!(WordPlace::parse(strongs, position), None, "{position}");
        }
    }

    #[test]
    fn a_word_on_the_page_gives_its_text_numbers_and_neighbours() {
        let info = find_word(&chapter(), &at(1, 3), None).expect("faith");
        assert_eq!(info.text, "faith");
        assert_eq!(info.numbers, ["G4102", "G9999"]);
        assert_eq!(info.previous, Some(at(1, 1).with_strongs("G1161")));
        // Verse 2 has no markup: the step goes over it.
        assert_eq!(info.next, Some(at(3, 0).with_strongs("G1680")));
    }

    #[test]
    fn the_steps_stop_at_both_ends_of_the_chapter() {
        let first = find_word(&chapter(), &at(1, 1), None).expect("first");
        assert_eq!(first.previous, None);
        assert_eq!(neighbour_word(&first, Step::Prev), None);
        let last = find_word(&chapter(), &at(3, 0), None).expect("last");
        assert_eq!(last.next, None);
        assert_eq!(neighbour_word(&last, Step::Next), None);
    }

    #[test]
    fn a_place_with_no_tagged_word_is_no_word() {
        for place in [at(1, 0), at(1, 2), at(1, 9), at(2, 0), at(9, 0)] {
            assert_eq!(find_word(&chapter(), &place, None), None, "{place:?}");
        }
    }

    #[test]
    fn one_verse_on_screen_keeps_the_word_and_the_steps_inside_it() {
        let info = find_word(&chapter(), &at(1, 3), Some(1)).expect("faith");
        assert_eq!(info.next, None);
        assert_eq!(find_word(&chapter(), &at(3, 0), Some(1)), None);
    }

    #[test]
    fn numbers_are_only_in_a_text_that_has_them() {
        assert!(has_numbers(&chapter()));
        let plain = [StudyVerse {
            verse: 1,
            text: "Viera.".to_string(),
            markup: Some(VerseMarkup(vec![MarkupSegment::note("Pozn.")])),
        }];
        assert!(!has_numbers(&plain));
    }

    #[test]
    fn a_hit_marks_every_word_with_the_number_and_keeps_the_text() {
        let segments = [
            word("Now", &["G1161"]),
            MarkupSegment::text(" "),
            word("faith", &["G1", "G4102"]),
            MarkupSegment::note("note"),
            MarkupSegment::text(", "),
            word("faith", &["G4102"]),
        ];
        let parts = marked_segments("Now faith, faith", Some(&segments), "G4102");
        assert_eq!(
            parts,
            [
                ("Now ".to_string(), false),
                ("faith".to_string(), true),
                (", ".to_string(), false),
                ("faith".to_string(), true),
            ]
        );
    }

    #[test]
    fn a_hit_without_markup_is_its_plain_text() {
        assert_eq!(
            marked_segments("Plain.", None, "G1"),
            [("Plain.".to_string(), false)]
        );
    }

    #[test]
    fn the_first_sentence_ends_at_a_semicolon_or_a_dot_but_not_at_an_abbreviation() {
        assert_eq!(
            first_sentence("persuasion, i.e. credence; moral conviction"),
            "persuasion, i.e. credence"
        );
        assert_eq!(first_sentence("a word. Then more."), "a word");
        assert_eq!(first_sentence("no end at all"), "no end at all");
        assert_eq!(first_sentence("3.5 cubits"), "3.5 cubits");
    }

    #[test]
    fn a_derivation_links_every_number_and_keeps_every_character() {
        let parts = linked_parts("from G3982; compare H430 (ʼĕlôhîym), HG12, G12345.");
        let joined: String = parts.iter().map(|(piece, _)| piece.as_str()).collect();
        assert_eq!(joined, "from G3982; compare H430 (ʼĕlôhîym), HG12, G12345.");
        let links: Vec<_> = parts.iter().filter_map(|(_, link)| link.clone()).collect();
        assert_eq!(links, ["G3982", "H430"]);
    }

    #[test]
    fn a_scope_keeps_one_form_and_nonsense_is_the_whole_bible() {
        assert_eq!(normalize_scope("NT"), "nt");
        assert_eq!(normalize_scope("heb"), "HEB");
        assert_eq!(normalize_scope(""), "all");
        assert_eq!(normalize_scope("nonsense"), "all");
    }

    #[test]
    fn scope_chips_count_verses_and_hide_what_is_empty() {
        let per_book = [("ROM".to_string(), 3), ("HEB".to_string(), 2)];
        let chips = scope_chips(&per_book, "all");
        let scopes: Vec<_> = chips
            .iter()
            .map(|chip| (chip.scope.as_str(), chip.count))
            .collect();
        assert_eq!(
            scopes,
            [("all", 5), ("nt", 5), ("ROM", 3), ("HEB", 2)],
            "no SZ: no such verses"
        );
    }

    #[test]
    fn the_scope_a_url_names_keeps_its_chip_even_when_empty() {
        let per_book = [("ROM".to_string(), 3)];
        let scopes: Vec<_> = scope_chips(&per_book, "GEN")
            .into_iter()
            .map(|chip| chip.scope)
            .collect();
        assert_eq!(scopes, ["all", "nt", "ROM", "GEN"]);
        let old = scope_chips(&per_book, "ot");
        assert!(old.iter().any(|chip| chip.scope == "ot" && chip.count == 0));
    }

    #[test]
    fn a_number_without_a_space_comes_first_with_one_it_is_the_alternative() {
        let number = |input: &str| strongs_offer(input).map(|o| (o.number, o.first));
        assert_eq!(number("G4102"), Some(("G4102".to_string(), true)));
        assert_eq!(number("g26"), Some(("G26".to_string(), true)));
        assert_eq!(number(" h0430 "), Some(("H430".to_string(), true)));
        assert_eq!(number("g 26"), Some(("G26".to_string(), false)));
    }

    #[test]
    fn anything_else_is_no_number() {
        for input in ["G", "G12345", "X12", "G26a", "viera", "G0", "26", ""] {
            assert_eq!(strongs_offer(input), None, "{input}");
        }
    }

    #[test]
    fn the_settle_key_and_the_label_name_the_number() {
        assert_eq!(occurrences_key("G4102", "all"), "G4102:all");
        assert_eq!(strongs_label("G4102"), "Výskyty: G4102 · KJV");
    }
}
