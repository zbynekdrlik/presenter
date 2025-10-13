use presenter_core::bible::canonical_book_by_code;
use presenter_core::BibleTranslation;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum BibleSourceFormat {
    UsfmZip {
        book_name_overrides: HashMap<String, String>,
    },
    MySwordSqlite {
        book_names: Vec<String>,
    },
    ObohuSqlite,
}

impl BibleSourceFormat {
    pub(crate) fn book_name(&self, code: &str) -> Option<String> {
        match self {
            BibleSourceFormat::UsfmZip {
                book_name_overrides,
            } => {
                if let Some(name) = book_name_overrides.get(code) {
                    return Some(name.clone());
                }
                default_book_name(code).map(|name| name.to_string())
            }
            BibleSourceFormat::MySwordSqlite { .. } | BibleSourceFormat::ObohuSqlite => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BibleTranslationSpec {
    pub translation: BibleTranslation,
    pub source: BibleSource,
    pub format: BibleSourceFormat,
}

#[derive(Clone, Debug)]
pub enum BibleSource {
    Url { url: String },
    LocalFile { env_var: String, hint: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleImportSummary {
    pub translation_code: String,
    pub passage_count: usize,
}

pub fn default_translation_specs() -> Vec<BibleTranslationSpec> {
    // Temporary: limit defaults to sources verified working in CI as of 2025-10-12.
    // Follow-up: re-add Roháček and Evanjelický once reliable mirrors are confirmed.
    vec![
        king_james_spec(),
        slovak_ekumenicky_spec(),
        slovak_rohacek_spec(),
        slovak_evangelicky_spec(),
    ]
}

pub const KING_JAMES_SOURCE: &str = "https://ebible.org/Scriptures/eng-kjv_usfm.zip";
pub const SLOVAK_ECUMENICKY_SOURCE: &str =
    "https://mysword-bible.info/download/getfile.php?file=SlovakEcumenicalTranslation.bbl.mybible.zip";
pub const SLOVAK_ROHACEK_SOURCE: &str =
    "https://github.com/otvorenie/obohu-sqlite/releases/download/2023-08-17/ROH-AV.SQLite3.zip";
pub const SLOVAK_EVANGELICKY_SOURCE: &str =
    "https://github.com/otvorenie/obohu-sqlite/releases/download/2023-08-17/SEVP-NT.SQLite3.zip";

fn king_james_spec() -> BibleTranslationSpec {
    let mut overrides = HashMap::new();
    overrides.insert("PSA".to_string(), "Psalms".to_string());
    overrides.insert("SNG".to_string(), "Song of Songs".to_string());

    BibleTranslationSpec {
        translation: BibleTranslation::new("eng-kjv", "King James Version", "en")
            .with_source(KING_JAMES_SOURCE),
        source: BibleSource::Url {
            url: KING_JAMES_SOURCE.to_string(),
        },
        format: BibleSourceFormat::UsfmZip {
            book_name_overrides: overrides,
        },
    }
}

pub(crate) fn slovak_ekumenicky_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk")
            .with_source(SLOVAK_ECUMENICKY_SOURCE),
        source: BibleSource::Url {
            url: SLOVAK_ECUMENICKY_SOURCE.to_string(),
        },
        format: BibleSourceFormat::MySwordSqlite {
            book_names: slovak_ekumenicky_book_names(),
        },
    }
}

pub(crate) fn slovak_rohacek_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-roh", "Roháčkov preklad", "sk")
            .with_source(SLOVAK_ROHACEK_SOURCE),
        source: BibleSource::Url {
            url: SLOVAK_ROHACEK_SOURCE.to_string(),
        },
        format: BibleSourceFormat::ObohuSqlite,
    }
}

pub(crate) fn slovak_evangelicky_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-sevp", "Slovenský evanjelický preklad", "sk")
            .with_source(SLOVAK_EVANGELICKY_SOURCE),
        source: BibleSource::Url {
            url: SLOVAK_EVANGELICKY_SOURCE.to_string(),
        },
        format: BibleSourceFormat::ObohuSqlite,
    }
}

#[allow(dead_code)]
fn czech_b21_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("ces-b21", "Bible 21", "cs").with_source("manual"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_CZECH_B21".to_string(),
            hint: "Set PRESENTER_BIBLE_CZECH_B21 to the path of the Czech Bible USFM zip you are licensed to use.".to_string(),
        },
        format: BibleSourceFormat::UsfmZip {
            book_name_overrides: HashMap::new(),
        },
    }
}

const SLOVAK_ECUMENICKY_BOOKS: &[&str] = &[
    "Genezis",
    "Exodus",
    "Levitikus",
    "Numeri",
    "Deuteronómium",
    "Józua",
    "Sudcovia",
    "Rút",
    "1 Samuel",
    "2 Samuel",
    "1 Kráľov",
    "2 Kráľov",
    "1 Kroník",
    "2 Kroník",
    "Ezdráš",
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
    "Hozeáš",
    "Joel",
    "Ámos",
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
    "Skutky",
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
    "Filemonovi",
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

fn slovak_ekumenicky_book_names() -> Vec<String> {
    SLOVAK_ECUMENICKY_BOOKS
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

fn default_book_name(code: &str) -> Option<&'static str> {
    canonical_book_by_code(code).map(|meta| meta.english_name)
}
