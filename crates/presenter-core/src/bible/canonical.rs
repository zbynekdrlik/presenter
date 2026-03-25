use std::collections::HashMap;
use std::sync::LazyLock;

use super::search::normalise_book_key;

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

/// Common Slovak abbreviations used in church bulletins and pastor's notes.
const SLOVAK_ABBREVIATIONS: &[(&str, &str)] = &[
    ("Sk", "ACT"),
    ("Mar", "MRK"),
    ("Iz", "ISA"),
    ("Luk", "LUK"),
    ("Mat", "MAT"),
    ("Ján", "JHN"),
    ("Žid", "HEB"),
    ("Rim", "ROM"),
    ("Gal", "GAL"),
    ("Ef", "EPH"),
    ("Fil", "PHP"),
    ("Kol", "COL"),
    ("Tít", "TIT"),
    ("Flm", "PHM"),
    ("Žalm", "PSA"),
    ("Prísl", "PRO"),
    ("Jer", "JER"),
    ("Ez", "EZK"),
    ("Dan", "DAN"),
    ("1Sa", "1SA"),
    ("2Sa", "2SA"),
    ("1Kra", "1KI"),
    ("2Kra", "2KI"),
    ("1Kor", "1CO"),
    ("2Kor", "2CO"),
    ("1Sol", "1TH"),
    ("2Sol", "2TH"),
    ("1Tim", "1TI"),
    ("2Tim", "2TI"),
    ("1Pt", "1PE"),
    ("2Pt", "2PE"),
    ("1Jn", "1JN"),
    ("2Jn", "2JN"),
    ("3Jn", "3JN"),
    ("Zj", "REV"),
    ("Oz", "HOS"),
    ("Am", "AMO"),
    ("Ab", "OBA"),
    ("Jon", "JON"),
    ("Mich", "MIC"),
    ("Nah", "NAM"),
    ("Hab", "HAB"),
    ("Sof", "ZEP"),
    ("Hag", "HAG"),
    ("Zach", "ZEC"),
    ("Mal", "MAL"),
    ("Jób", "JOB"),
    ("Kaz", "ECC"),
    ("Pies", "SNG"),
];

static BOOK_NAME_ALIASES: LazyLock<HashMap<String, BibleBookCanonical>> = LazyLock::new(|| {
    let mut map: HashMap<String, BibleBookCanonical> = HashMap::new();

    for meta in CANONICAL_BOOKS.iter().copied() {
        register_alias_variants(&mut map, meta.english_name, meta);
        register_alias_variants(&mut map, meta.code, meta);
        register_alias_variants(&mut map, meta.code.to_ascii_lowercase(), meta);
    }

    #[allow(clippy::cast_possible_truncation)]
    for (index, alias) in SLOVAK_ECUMENICKY_NAMES.iter().enumerate() {
        // SAFETY: Array has fixed size < 100 elements, so index fits in u16
        if let Some(meta) = canonical_book_by_number((index as u16) + 1) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    for (index, alias) in SLOVAK_OBOHU_NAMES.iter().enumerate() {
        // SAFETY: Array has fixed size < 100 elements, so index fits in u16
        if let Some(meta) = canonical_book_by_number((index as u16) + 1) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    for &(alias, code) in ENGLISH_EXTRA_ALIASES {
        if let Some(meta) = canonical_book_by_code(code) {
            register_alias_variants(&mut map, alias, meta);
        }
    }

    for &(alias, code) in SLOVAK_ABBREVIATIONS {
        if let Some(meta) = canonical_book_by_code(code) {
            register_alias(&mut map, alias, meta);
        }
    }

    map
});

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
