use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BibleReferenceError {
    #[error("chapter must be positive")]
    InvalidChapter,
    #[error("verse numbers must be positive and ordered")]
    InvalidVerseRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleReference {
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
}

impl BibleReference {
    pub fn new<T: Into<String>>(
        book: T,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> Result<Self, BibleReferenceError> {
        Self::validate(chapter, verse_start, verse_end)?;
        Ok(Self {
            book: book.into(),
            book_code: None,
            book_number: None,
            chapter,
            verse_start,
            verse_end,
        })
    }

    pub fn new_with_code<T: Into<String>, U: Into<String>>(
        book: T,
        book_code: U,
        book_number: u16,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> Result<Self, BibleReferenceError> {
        Self::validate(chapter, verse_start, verse_end)?;
        Ok(Self {
            book: book.into(),
            book_code: Some(book_code.into()),
            book_number: Some(book_number),
            chapter,
            verse_start,
            verse_end,
        })
    }

    fn validate(chapter: u16, verse_start: u16, verse_end: u16) -> Result<(), BibleReferenceError> {
        if chapter == 0 {
            return Err(BibleReferenceError::InvalidChapter);
        }
        if verse_start == 0 || verse_end == 0 || verse_start > verse_end {
            return Err(BibleReferenceError::InvalidVerseRange);
        }
        Ok(())
    }

    pub fn to_human_readable(&self) -> String {
        if self.verse_start == self.verse_end {
            format!("{} {}:{}", self.book, self.chapter, self.verse_start)
        } else {
            format!(
                "{} {}:{}-{}",
                self.book, self.chapter, self.verse_start, self.verse_end
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleTranslation {
    pub code: String,
    pub name: String,
    pub language: String,
    #[serde(default = "BibleTranslation::default_show_in_dashboard")]
    pub show_in_dashboard: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl BibleTranslation {
    const fn default_show_in_dashboard() -> bool {
        true
    }

    pub fn new(
        code: impl Into<String>,
        name: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            language: language.into(),
            show_in_dashboard: Self::default_show_in_dashboard(),
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_show_in_dashboard(mut self, show: bool) -> Self {
        self.show_in_dashboard = show;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePassage {
    pub reference: BibleReference,
    pub translation: BibleTranslation,
    pub text: String,
}

impl BiblePassage {
    pub fn new(reference: BibleReference, translation: BibleTranslation, text: String) -> Self {
        Self {
            reference,
            translation,
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_translation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_translation: Option<String>,
    pub character_limit: u32,
}

impl Default for BiblePreferences {
    fn default() -> Self {
        Self {
            main_translation: None,
            secondary_translation: None,
            character_limit: 320,
        }
    }
}

impl BiblePreferences {
    pub fn with_main_translation(mut self, code: Option<String>) -> Self {
        self.main_translation = code;
        self
    }

    pub fn with_secondary_translation(mut self, code: Option<String>) -> Self {
        self.secondary_translation = code;
        self
    }

    pub fn with_character_limit(mut self, limit: u32) -> Self {
        self.character_limit = limit;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePreferencesDraft {
    pub main_translation: Option<String>,
    pub secondary_translation: Option<String>,
    pub character_limit: Option<u32>,
}

impl BiblePreferencesDraft {
    pub fn apply(self, mut base: BiblePreferences) -> BiblePreferences {
        if let Some(main) = self.main_translation {
            base.main_translation = Some(main);
        }
        if let Some(secondary) = self.secondary_translation {
            base.secondary_translation = Some(secondary);
        }
        if let Some(limit) = self.character_limit {
            base.character_limit = limit;
        }
        base
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleBookChapterSummary {
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verse_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleBroadcast {
    pub passage: BiblePassage,
    pub triggered_at: DateTime<Utc>,
}

impl BibleBroadcast {
    pub fn new(passage: BiblePassage, triggered_at: DateTime<Utc>) -> Self {
        Self {
            passage,
            triggered_at,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BibleIngestionError {
    #[error("passage translation '{found}' does not match batch translation '{expected}'")]
    TranslationMismatch { expected: String, found: String },
    #[error("duplicate passage for {book} {chapter}:{start}-{end}")]
    DuplicatePassage {
        book: String,
        chapter: u16,
        start: u16,
        end: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleIngestionBatch {
    translation: BibleTranslation,
    passages: Vec<BiblePassage>,
}

impl BibleIngestionBatch {
    pub fn new(
        translation: BibleTranslation,
        passages: Vec<BiblePassage>,
    ) -> Result<Self, BibleIngestionError> {
        let expected = translation.code.clone();
        let mut seen = std::collections::HashSet::new();
        for passage in &passages {
            let found = &passage.translation.code;
            if found != &expected {
                return Err(BibleIngestionError::TranslationMismatch {
                    expected: expected.clone(),
                    found: found.clone(),
                });
            }
            let key = (
                passage.reference.book.clone(),
                passage.reference.chapter,
                passage.reference.verse_start,
                passage.reference.verse_end,
            );
            if !seen.insert(key.clone()) {
                return Err(BibleIngestionError::DuplicatePassage {
                    book: key.0,
                    chapter: key.1,
                    start: key.2,
                    end: key.3,
                });
            }
        }
        Ok(Self {
            translation,
            passages,
        })
    }

    pub fn translation(&self) -> &BibleTranslation {
        &self.translation
    }

    pub fn passages(&self) -> &[BiblePassage] {
        &self.passages
    }

    pub fn into_parts(self) -> (BibleTranslation, Vec<BiblePassage>) {
        (self.translation, self.passages)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BibleBookCanonical {
    pub code: &'static str,
    pub number: u16,
    pub english_name: &'static str,
}

const CANONICAL_BOOKS: [BibleBookCanonical; 66] = [
    BibleBookCanonical {
        code: "GEN",
        number: 1,
        english_name: "Genesis",
    },
    BibleBookCanonical {
        code: "EXO",
        number: 2,
        english_name: "Exodus",
    },
    BibleBookCanonical {
        code: "LEV",
        number: 3,
        english_name: "Leviticus",
    },
    BibleBookCanonical {
        code: "NUM",
        number: 4,
        english_name: "Numbers",
    },
    BibleBookCanonical {
        code: "DEU",
        number: 5,
        english_name: "Deuteronomy",
    },
    BibleBookCanonical {
        code: "JOS",
        number: 6,
        english_name: "Joshua",
    },
    BibleBookCanonical {
        code: "JDG",
        number: 7,
        english_name: "Judges",
    },
    BibleBookCanonical {
        code: "RUT",
        number: 8,
        english_name: "Ruth",
    },
    BibleBookCanonical {
        code: "1SA",
        number: 9,
        english_name: "1 Samuel",
    },
    BibleBookCanonical {
        code: "2SA",
        number: 10,
        english_name: "2 Samuel",
    },
    BibleBookCanonical {
        code: "1KI",
        number: 11,
        english_name: "1 Kings",
    },
    BibleBookCanonical {
        code: "2KI",
        number: 12,
        english_name: "2 Kings",
    },
    BibleBookCanonical {
        code: "1CH",
        number: 13,
        english_name: "1 Chronicles",
    },
    BibleBookCanonical {
        code: "2CH",
        number: 14,
        english_name: "2 Chronicles",
    },
    BibleBookCanonical {
        code: "EZR",
        number: 15,
        english_name: "Ezra",
    },
    BibleBookCanonical {
        code: "NEH",
        number: 16,
        english_name: "Nehemiah",
    },
    BibleBookCanonical {
        code: "EST",
        number: 17,
        english_name: "Esther",
    },
    BibleBookCanonical {
        code: "JOB",
        number: 18,
        english_name: "Job",
    },
    BibleBookCanonical {
        code: "PSA",
        number: 19,
        english_name: "Psalm",
    },
    BibleBookCanonical {
        code: "PRO",
        number: 20,
        english_name: "Proverbs",
    },
    BibleBookCanonical {
        code: "ECC",
        number: 21,
        english_name: "Ecclesiastes",
    },
    BibleBookCanonical {
        code: "SNG",
        number: 22,
        english_name: "Song of Songs",
    },
    BibleBookCanonical {
        code: "ISA",
        number: 23,
        english_name: "Isaiah",
    },
    BibleBookCanonical {
        code: "JER",
        number: 24,
        english_name: "Jeremiah",
    },
    BibleBookCanonical {
        code: "LAM",
        number: 25,
        english_name: "Lamentations",
    },
    BibleBookCanonical {
        code: "EZK",
        number: 26,
        english_name: "Ezekiel",
    },
    BibleBookCanonical {
        code: "DAN",
        number: 27,
        english_name: "Daniel",
    },
    BibleBookCanonical {
        code: "HOS",
        number: 28,
        english_name: "Hosea",
    },
    BibleBookCanonical {
        code: "JOL",
        number: 29,
        english_name: "Joel",
    },
    BibleBookCanonical {
        code: "AMO",
        number: 30,
        english_name: "Amos",
    },
    BibleBookCanonical {
        code: "OBA",
        number: 31,
        english_name: "Obadiah",
    },
    BibleBookCanonical {
        code: "JON",
        number: 32,
        english_name: "Jonah",
    },
    BibleBookCanonical {
        code: "MIC",
        number: 33,
        english_name: "Micah",
    },
    BibleBookCanonical {
        code: "NAM",
        number: 34,
        english_name: "Nahum",
    },
    BibleBookCanonical {
        code: "HAB",
        number: 35,
        english_name: "Habakkuk",
    },
    BibleBookCanonical {
        code: "ZEP",
        number: 36,
        english_name: "Zephaniah",
    },
    BibleBookCanonical {
        code: "HAG",
        number: 37,
        english_name: "Haggai",
    },
    BibleBookCanonical {
        code: "ZEC",
        number: 38,
        english_name: "Zechariah",
    },
    BibleBookCanonical {
        code: "MAL",
        number: 39,
        english_name: "Malachi",
    },
    BibleBookCanonical {
        code: "MAT",
        number: 40,
        english_name: "Matthew",
    },
    BibleBookCanonical {
        code: "MRK",
        number: 41,
        english_name: "Mark",
    },
    BibleBookCanonical {
        code: "LUK",
        number: 42,
        english_name: "Luke",
    },
    BibleBookCanonical {
        code: "JHN",
        number: 43,
        english_name: "John",
    },
    BibleBookCanonical {
        code: "ACT",
        number: 44,
        english_name: "Acts",
    },
    BibleBookCanonical {
        code: "ROM",
        number: 45,
        english_name: "Romans",
    },
    BibleBookCanonical {
        code: "1CO",
        number: 46,
        english_name: "1 Corinthians",
    },
    BibleBookCanonical {
        code: "2CO",
        number: 47,
        english_name: "2 Corinthians",
    },
    BibleBookCanonical {
        code: "GAL",
        number: 48,
        english_name: "Galatians",
    },
    BibleBookCanonical {
        code: "EPH",
        number: 49,
        english_name: "Ephesians",
    },
    BibleBookCanonical {
        code: "PHP",
        number: 50,
        english_name: "Philippians",
    },
    BibleBookCanonical {
        code: "COL",
        number: 51,
        english_name: "Colossians",
    },
    BibleBookCanonical {
        code: "1TH",
        number: 52,
        english_name: "1 Thessalonians",
    },
    BibleBookCanonical {
        code: "2TH",
        number: 53,
        english_name: "2 Thessalonians",
    },
    BibleBookCanonical {
        code: "1TI",
        number: 54,
        english_name: "1 Timothy",
    },
    BibleBookCanonical {
        code: "2TI",
        number: 55,
        english_name: "2 Timothy",
    },
    BibleBookCanonical {
        code: "TIT",
        number: 56,
        english_name: "Titus",
    },
    BibleBookCanonical {
        code: "PHM",
        number: 57,
        english_name: "Philemon",
    },
    BibleBookCanonical {
        code: "HEB",
        number: 58,
        english_name: "Hebrews",
    },
    BibleBookCanonical {
        code: "JAS",
        number: 59,
        english_name: "James",
    },
    BibleBookCanonical {
        code: "1PE",
        number: 60,
        english_name: "1 Peter",
    },
    BibleBookCanonical {
        code: "2PE",
        number: 61,
        english_name: "2 Peter",
    },
    BibleBookCanonical {
        code: "1JN",
        number: 62,
        english_name: "1 John",
    },
    BibleBookCanonical {
        code: "2JN",
        number: 63,
        english_name: "2 John",
    },
    BibleBookCanonical {
        code: "3JN",
        number: 64,
        english_name: "3 John",
    },
    BibleBookCanonical {
        code: "JUD",
        number: 65,
        english_name: "Jude",
    },
    BibleBookCanonical {
        code: "REV",
        number: 66,
        english_name: "Revelation",
    },
];

const SLOVAK_ECUMENICKY_NAMES: [&str; 66] = [
    "Genezis",
    "Exodus",
    "Levitikus",
    "Numeri",
    "Deuteronómium",
    "Jozue",
    "Sudcov",
    "Rút",
    "1 Samuelova",
    "2 Samuelova",
    "1 Kráľov",
    "2 Kráľov",
    "1 Kroník",
    "2 Kroník",
    "Ezdraš",
    "Nehemiáš",
    "Ester",
    "Jób",
    "Žalmy",
    "Príslovia",
    "Kazateľ",
    "Pieseň piesní",
    "Izaiáš",
    "Jeremiáš",
    "Náreky",
    "Ezechiel",
    "Daniel",
    "Ozeáš",
    "Joel",
    "Amos",
    "Abdiáš",
    "Jonáš",
    "Micheáš",
    "Nahum",
    "Habakuk",
    "Sofoniáš",
    "Haggeus",
    "Zachariáš",
    "Malachiáš",
    "Matúš",
    "Marek",
    "Lukáš",
    "Ján",
    "Skutky apoštolov",
    "Rimanom",
    "1 Korinťanom",
    "2 Korinťanom",
    "Galaťanom",
    "Efezanom",
    "Filipanom",
    "Kolosanom",
    "1 Tesaloničanom",
    "2 Tesaloničanom",
    "1 Timotejovi",
    "2 Timotejovi",
    "Títovi",
    "Filimonovi",
    "Hebrejom",
    "Jakub",
    "1 Peter",
    "2 Peter",
    "1 Ján",
    "2 Ján",
    "3 Ján",
    "Júda",
    "Zjavenie",
];

const SLOVAK_OBOHU_NAMES: [&str; 66] = [
    "Genesis",
    "Exodus",
    "Levitikus",
    "Numeri",
    "Deuteronómium",
    "Jozue",
    "Sudcovia",
    "Rút",
    "1 Samuel",
    "2 Samuel",
    "1 Kniha kráľov",
    "2 Kniha kráľov",
    "1 Kniha kroník; Paralipomenon",
    "2 Kniha kroník; Paralipomenon",
    "Ezdráš",
    "Nehemiáš",
    "Ester",
    "Jób",
    "Žalmy",
    "Príslovia",
    "Kazateľ",
    "Pieseň piesní; Veľpieseň",
    "Izaiáš",
    "Jeremiáš",
    "Žalospevy; Plač Jeremiášov",
    "Ezechiel",
    "Daniel",
    "Ozeáš",
    "Joel",
    "Amos",
    "Obadiáš",
    "Jonáš",
    "Micheáš",
    "Nahum",
    "Abakuk",
    "Sofoniáš",
    "Aggeus",
    "Zachariáš",
    "Malachiáš",
    "Evanjelium podľa Matúša",
    "Evanjelium podľa Mareka",
    "Evanjelium podľa Lukáša",
    "Evanjelium podľa Jána",
    "Skutky apoštolov",
    "List apoštola Pavla Rímskym",
    "Prvýlist apoštola Pavla Korintským",
    "Druhýlist apoštola Pavla Korintským",
    "List apoštola Pavla Galatským",
    "List apoštola Pavla Efezským",
    "List apoštola Pavla Filipským",
    "List apoštola Pavla Kolosenským",
    "Prvýlist apoštola Pavla Tesalonickým",
    "Druhýlist apoštola Pavla Tesalonickým",
    "Prvýlist apoštola Pavla Timoteovi",
    "Druhýlist apoštola Pavla Timoteovi",
    "List apoštola Pavla Títovi",
    "List apoštola Pavla Filemonovi",
    "List Židom",
    "List Jakuba",
    "Prvýlist apoštola Petra",
    "Druhýlist apoštola Petra",
    "Prvýlist Jánov",
    "Druhýlist Jánov",
    "Tretílist Jánov",
    "List Júdov",
    "Zjavenie Jána",
];

const ENGLISH_EXTRA_ALIASES: &[(&str, &str)] = &[
    ("Psalms", "PSA"),
    ("Songs of Songs", "SNG"),
    ("Song of Solomon", "SNG"),
    ("Canticles", "SNG"),
    ("Revelation of John", "REV"),
    ("Book of Revelation", "REV"),
    ("Revelation", "REV"),
    ("Acts of the Apostles", "ACT"),
    ("Acts", "ACT"),
];

static BOOK_NAME_ALIASES: Lazy<HashMap<String, BibleBookCanonical>> = Lazy::new(|| {
    let mut map: HashMap<String, BibleBookCanonical> = HashMap::new();

    for meta in CANONICAL_BOOKS.iter().copied() {
        register_alias_variants(&mut map, meta.english_name, meta);
        register_alias_variants(&mut map, meta.code, meta);
        register_alias_variants(&mut map, meta.code.to_ascii_lowercase(), meta);
    }

    for (index, alias) in SLOVAK_ECUMENICKY_NAMES.iter().enumerate() {
        if let Some(meta) = canonical_book_by_number((index as u16) + 1) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    for (index, alias) in SLOVAK_OBOHU_NAMES.iter().enumerate() {
        if let Some(meta) = canonical_book_by_number((index as u16) + 1) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    for &(alias, code) in ENGLISH_EXTRA_ALIASES {
        if let Some(meta) = canonical_book_by_code(code) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    map
});

fn normalise_book_key(input: &str) -> String {
    let mut result = String::new();
    for ch in input.nfd().filter(|c| !is_combining_mark(*c)) {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        }
    }
    result
}

fn register_alias(
    map: &mut HashMap<String, BibleBookCanonical>,
    alias: &str,
    meta: BibleBookCanonical,
) {
    let key = normalise_book_key(alias);
    if key.is_empty() {
        return;
    }
    map.entry(key).or_insert(meta);
}

fn register_alias_variants(
    map: &mut HashMap<String, BibleBookCanonical>,
    alias: impl AsRef<str>,
    meta: BibleBookCanonical,
) {
    let alias_str = alias.as_ref();
    if alias_str.trim().is_empty() {
        return;
    }
    register_alias(map, alias_str, meta);

    for part in alias_str.split([';', '/', ',', '(', ')']) {
        let trimmed = part.trim();
        if !trimmed.is_empty() && trimmed != alias_str {
            register_alias(map, trimmed, meta);
        }
    }

    if let Some(stripped) = alias_str.strip_prefix("List apoštola Pavla ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("List ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Evanjelium podľa ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Prvýlist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Druhýlist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Tretílist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Prvylist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Druhylist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("Tretilist ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("1 Kniha ") {
        register_alias(map, stripped, meta);
    }
    if let Some(stripped) = alias_str.strip_prefix("2 Kniha ") {
        register_alias(map, stripped, meta);
    }
}

pub fn canonical_book_by_name(name: &str) -> Option<BibleBookCanonical> {
    let key = normalise_book_key(name);
    if key.is_empty() {
        return None;
    }
    BOOK_NAME_ALIASES.get(&key).copied()
}

pub fn canonical_book_by_code(code: &str) -> Option<BibleBookCanonical> {
    CANONICAL_BOOKS
        .iter()
        .copied()
        .find(|meta| meta.code.eq_ignore_ascii_case(code))
}

pub fn canonical_book_by_number(number: u16) -> Option<BibleBookCanonical> {
    CANONICAL_BOOKS
        .iter()
        .copied()
        .find(|meta| meta.number == number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_reference_ranges() {
        assert_eq!(
            BibleReference::new("John", 1, 5, 3).unwrap_err(),
            BibleReferenceError::InvalidVerseRange
        );
    }

    #[test]
    fn formats_reference() {
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        assert_eq!(reference.to_human_readable(), "John 3:16");
        let range = BibleReference::new("Psalm", 23, 1, 3).unwrap();
        assert_eq!(range.to_human_readable(), "Psalm 23:1-3");
    }

    #[test]
    fn ingestion_batch_rejects_translation_mismatch() {
        let translation = BibleTranslation::new("sk-seb", "Slovak", "sk");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference,
            BibleTranslation::new("en-kjv", "KJV", "en"),
            "For God so loved".to_string(),
        );
        let err = BibleIngestionBatch::new(translation, vec![passage]).unwrap_err();
        assert!(matches!(
            err,
            BibleIngestionError::TranslationMismatch { expected, found }
            if expected == "sk-seb" && found == "en-kjv"
        ));
    }

    #[test]
    fn ingestion_batch_rejects_duplicate_references() {
        let translation = BibleTranslation::new("en-kjv", "KJV", "en");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference.clone(),
            translation.clone(),
            "For God so loved".to_string(),
        );
        let err =
            BibleIngestionBatch::new(translation, vec![passage.clone(), passage]).unwrap_err();
        assert!(matches!(
            err,
            BibleIngestionError::DuplicatePassage { book, chapter, start, end }
            if book == "John" && chapter == 3 && start == 16 && end == 16
        ));
    }

    #[test]
    fn ingestion_batch_accepts_unique_passages() {
        let translation = BibleTranslation::new("en-kjv", "KJV", "en");
        let reference = BibleReference::new("John", 3, 16, 17).unwrap();
        let next = BibleReference::new("John", 3, 18, 18).unwrap();
        let passages = vec![
            BiblePassage::new(
                reference,
                translation.clone(),
                "For God so loved".to_string(),
            ),
            BiblePassage::new(next, translation.clone(), "He that believeth".to_string()),
        ];
        let batch = BibleIngestionBatch::new(translation.clone(), passages.clone()).unwrap();
        assert_eq!(batch.translation(), &translation);
        assert_eq!(batch.passages(), passages.as_slice());
    }

    #[test]
    fn canonical_lookup_matches_slovak_aliases() {
        let judges = canonical_book_by_name("Sudcovia").expect("Judges alias present");
        assert_eq!(judges.code, "JDG");
        assert_eq!(judges.number, 7);

        let romans = canonical_book_by_name("List apoštola Pavla Rímskym").expect("Romans alias");
        assert_eq!(romans.code, "ROM");
        assert_eq!(romans.number, 45);

        let revelation = canonical_book_by_name("Revelation of John").expect("Revelation alias");
        assert_eq!(revelation.code, "REV");
        assert_eq!(revelation.number, 66);
    }
}
