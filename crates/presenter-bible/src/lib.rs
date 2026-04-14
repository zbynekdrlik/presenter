mod parsers;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::BibleTranslation;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

#[async_trait]
pub trait BibleContentProvider: Send + Sync {
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>>;
}

#[derive(Clone, Debug)]
pub struct HttpBibleContentProvider {
    client: reqwest::Client,
}

fn resolve_cache_path(url: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PRESENTER_BIBLE_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join(sanitise_url(url)));
        }
    }
    if let Some(base) = std::env::var_os("XDG_CACHE_HOME") {
        let mut path = PathBuf::from(base);
        path.push("presenter");
        path.push("bibles");
        path.push(sanitise_url(url));
        return Some(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".cache");
        path.push("presenter");
        path.push("bibles");
        path.push(sanitise_url(url));
        return Some(path);
    }
    None
}

fn sanitise_url(url: &str) -> String {
    url.chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' => ch,
            _ => '_',
        })
        .collect()
}

impl HttpBibleContentProvider {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("PresenterBibleScraper/0.1")
            .build()?;
        Ok(Self { client })
    }
}

#[async_trait]
impl BibleContentProvider for HttpBibleContentProvider {
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let cache_path = resolve_cache_path(url);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..3 {
            match self.client.get(url).send().await {
                Ok(response) => match response.error_for_status() {
                    Ok(success) => match success.bytes().await {
                        Ok(bytes) => {
                            let bytes = bytes.to_vec();
                            if let Some(path) = cache_path.as_ref() {
                                if let Some(parent) = path.parent() {
                                    if let Err(err) = fs::create_dir_all(parent) {
                                        warn!(?err, "failed to create bible cache directory");
                                    }
                                }
                                if let Err(err) = fs::write(path, &bytes) {
                                    warn!(?err, path = %path.display(), "failed to write bible cache");
                                }
                            }
                            return Ok(bytes);
                        }
                        Err(err) => {
                            last_error = Some(err.into());
                        }
                    },
                    Err(err) => {
                        last_error = Some(err.into());
                    }
                },
                Err(err) => {
                    last_error = Some(err.into());
                }
            }

            if attempt < 2 {
                let backoff = Duration::from_secs(2_u64.pow(attempt as u32));
                tokio::time::sleep(backoff).await;
            }
        }

        if let Some(path) = cache_path.as_ref() {
            if path.exists() {
                warn!(
                    path = %path.display(),
                    error = ?last_error,
                    "using cached bible archive after download failure"
                );
                return fs::read(path).with_context(|| {
                    format!(
                        "failed to read cached bible archive from {}",
                        path.display()
                    )
                });
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("failed to download bible archive")))
    }
}

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
    fn book_name(&self, code: &str) -> Option<String> {
        match self {
            BibleSourceFormat::UsfmZip {
                book_name_overrides,
            } => {
                if let Some(name) = book_name_overrides.get(code) {
                    return Some(name.clone());
                }
                parsers::default_book_name(code).map(|name| name.to_string())
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

impl BibleSource {
    /// Returns true if this source can be fetched (URL sources always available,
    /// LocalFile sources only available if the env var is set).
    pub fn is_available(&self) -> bool {
        match self {
            BibleSource::Url { .. } => true,
            BibleSource::LocalFile { env_var, .. } => std::env::var(env_var).is_ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleImportSummary {
    pub translation_code: String,
    pub passage_count: usize,
}

pub struct BibleScraper<P: BibleContentProvider> {
    provider: P,
}

impl<P: BibleContentProvider> BibleScraper<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Fetch bytes for `spec` (URL or LocalFile) and return the parsed batch.
    pub async fn scrape(
        &self,
        spec: &BibleTranslationSpec,
    ) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
        let bytes = self.fetch_spec_bytes(spec).await?;
        self.scrape_with_bytes(spec, &bytes)
    }

    /// Parse bytes that the caller has already fetched. Used by the ingest
    /// binary so it can hash the bytes once and pass them in.
    pub fn scrape_with_bytes(
        &self,
        spec: &BibleTranslationSpec,
        bytes: &[u8],
    ) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
        parsers::build_ingestion_batch(bytes, spec)
    }

    async fn fetch_spec_bytes(&self, spec: &BibleTranslationSpec) -> Result<Vec<u8>> {
        match &spec.source {
            BibleSource::Url { url } => self
                .provider
                .fetch_bytes(url)
                .await
                .with_context(|| format!("failed to fetch bible archive from {}", url)),
            BibleSource::LocalFile { env_var, .. } => {
                read_local_archive_file(env_var, &spec.translation.code)
            }
        }
    }
}

/// Read a `LocalFile` spec's archive bytes from the path held in `env_var`.
/// Used by both `BibleScraper::fetch_spec_bytes` and the pub `read_local_file_bytes`
/// free function so error messages and lookup logic stay consistent.
fn read_local_archive_file(env_var: &str, translation_code: &str) -> anyhow::Result<Vec<u8>> {
    let path = std::env::var(env_var).with_context(|| {
        format!(
            "environment variable {} must be set to the bible archive for {}",
            env_var, translation_code
        )
    })?;
    std::fs::read(&path).with_context(|| {
        format!(
            "failed to read bible archive for {} from {}",
            translation_code, path
        )
    })
}

/// Read the bytes of a `LocalFile` spec's archive without going through a
/// `BibleContentProvider`. Returns `Err` for `Url` specs — callers that need
/// URL support should use `BibleScraper::scrape` or fetch bytes themselves.
pub fn read_local_file_bytes(spec: &BibleTranslationSpec) -> anyhow::Result<Vec<u8>> {
    match &spec.source {
        BibleSource::LocalFile { env_var, .. } => {
            read_local_archive_file(env_var, &spec.translation.code)
        }
        BibleSource::Url { .. } => Err(anyhow!(
            "read_local_file_bytes called on URL spec {}",
            spec.translation.code
        )),
    }
}

pub fn default_translation_specs() -> Vec<BibleTranslationSpec> {
    vec![
        king_james_spec(),
        slovak_ekumenicky_spec(),
        slovak_rohacek_spec(),
        slovak_evangelicky_spec(),
        slovak_milost_spec(),
    ]
}

fn king_james_spec() -> BibleTranslationSpec {
    let mut overrides = HashMap::new();
    overrides.insert("PSA".to_string(), "Psalms".to_string());
    overrides.insert("SNG".to_string(), "Song of Songs".to_string());

    BibleTranslationSpec {
        translation: BibleTranslation::new("eng-kjv", "King James Version", "en")
            .with_source("local USFM file"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_KJV".to_string(),
            hint: "/opt/presenter/bibles/kjv.usfm.zip".to_string(),
        },
        format: BibleSourceFormat::UsfmZip {
            book_name_overrides: overrides,
        },
    }
}

fn slovak_ekumenicky_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk")
            .with_source("local MySword file"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_SEB".to_string(),
            hint: "/opt/presenter/bibles/seb.bbl.mybible.zip".to_string(),
        },
        format: BibleSourceFormat::MySwordSqlite {
            book_names: slovak_ekumenicky_book_names(),
        },
    }
}

fn slovak_rohacek_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-roh", "Roháčkov preklad", "sk")
            .with_source("local MySword file"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_ROHACEK".to_string(),
            hint: "/opt/presenter/bibles/rohacek.bbl.mybible.zip".to_string(),
        },
        format: BibleSourceFormat::MySwordSqlite {
            book_names: slovak_rohacek_book_names(),
        },
    }
}

fn slovak_evangelicky_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-sevp", "Slovenský evanjelický preklad", "sk")
            .with_source("local Obohu file"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_SEVP".to_string(),
            hint: "/opt/presenter/bibles/sevp.obohu.mybible.zip".to_string(),
        },
        format: BibleSourceFormat::ObohuSqlite,
    }
}

fn slovak_milost_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-mil", "Preklad Milosť", "sk")
            .with_source("local MySword file"),
        source: BibleSource::LocalFile {
            env_var: "PRESENTER_BIBLE_MILOST".to_string(),
            hint: "/opt/presenter/bibles/milost.bbl.mybible.zip".to_string(),
        },
        format: BibleSourceFormat::MySwordSqlite {
            book_names: slovak_milost_book_names(),
        },
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

pub const KING_JAMES_SOURCE: &str = "https://ebible.org/Scriptures/eng-kjv_usfm.zip";
pub const SLOVAK_ECUMENICKY_SOURCE: &str =
    "https://mysword-bible.info/download/getfile.php?file=SlovakEcumenicalTranslation.bbl.mybible.zip";
pub const SLOVAK_ROHACEK_SOURCE: &str = "https://obohu.cz/attachments/article/112/ROH-AV.zip";
pub const SLOVAK_EVANGELICKY_SOURCE: &str = "https://obohu.cz/attachments/article/112/SEVP.zip";

const SLOVAK_ECUMENICKY_BOOKS: [&str; 66] = [
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

fn slovak_ekumenicky_book_names() -> Vec<String> {
    SLOVAK_ECUMENICKY_BOOKS
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

/// Book names for Roháček MySword translation
fn slovak_rohacek_book_names() -> Vec<String> {
    SLOVAK_ROHACEK_BOOKS
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

/// Book names for Milosť MySword translation
fn slovak_milost_book_names() -> Vec<String> {
    SLOVAK_MILOST_BOOKS
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

/// Slovak book names for Roháček translation (MySword format)
const SLOVAK_ROHACEK_BOOKS: [&str; 66] = [
    "1. Mojžišova",
    "2. Mojžišova",
    "3. Mojžišova",
    "4. Mojžišova",
    "5. Mojžišova",
    "Jozua",
    "Sudcovia",
    "Rút",
    "1. Samuelova",
    "2. Samuelova",
    "1. Kráľov",
    "2. Kráľov",
    "1. Kronická",
    "2. Kronická",
    "Ezdráš",
    "Nehemiáš",
    "Ester",
    "Jób",
    "Žalmy",
    "Príslovia",
    "Kazateľ",
    "Pieseň",
    "Izaiáš",
    "Jeremiáš",
    "Plač",
    "Ezechiel",
    "Daniel",
    "Hozeáš",
    "Joel",
    "Ámos",
    "Abdiáš",
    "Jonáš",
    "Micheáš",
    "Náhum",
    "Abakuk",
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
    "1. Korinťanom",
    "2. Korinťanom",
    "Galaťanom",
    "Efezanom",
    "Filipanom",
    "Kolosenským",
    "1. Tesaloničanom",
    "2. Tesaloničanom",
    "1. Timoteovi",
    "2. Timoteovi",
    "Títovi",
    "Filemonovi",
    "Židom",
    "Jakobov",
    "1. Petrov",
    "2. Petrov",
    "1. Jánov",
    "2. Jánov",
    "3. Jánov",
    "Júdov",
    "Zjavenie",
];

/// Slovak book names for Milosť translation (MySword format)
const SLOVAK_MILOST_BOOKS: [&str; 66] = [
    "Genezis",
    "Exodus",
    "Levitikus",
    "Numeri",
    "Deuteronómium",
    "Józua",
    "Sudcovia",
    "Rút",
    "1. Samuelova",
    "2. Samuelova",
    "1. Kráľov",
    "2. Kráľov",
    "1. Kroník",
    "2. Kroník",
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
    "Aggeus",
    "Zachariáš",
    "Malachiáš",
    "Matúš",
    "Marek",
    "Lukáš",
    "Ján",
    "Skutky",
    "Rimanom",
    "1. Korintským",
    "2. Korintským",
    "Galatským",
    "Efezským",
    "Filipským",
    "Kolosenským",
    "1. Tesalonickým",
    "2. Tesalonickým",
    "1. Timotejovi",
    "2. Timotejovi",
    "Títovi",
    "Filemonovi",
    "Židom",
    "Jakub",
    "1. Petra",
    "2. Petra",
    "1. Jána",
    "2. Jána",
    "3. Jána",
    "Júda",
    "Zjavenie",
];

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;
    use zip::write::FileOptions;

    use std::collections::HashSet;
    use std::io::{Cursor, Write};

    struct StubProvider {
        payload: Vec<u8>,
    }

    #[async_trait]
    impl BibleContentProvider for StubProvider {
        async fn fetch_bytes(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(self.payload.clone())
        }
    }

    fn sample_translation() -> BibleTranslation {
        BibleTranslation::new("en-test", "Test Translation", "en")
    }

    fn make_sample_archive() -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            let content =
                "\\id JHN John\n\\c 3\n\\v 16 For God so loved the world\n\\v 17 that He gave\n";
            writer.start_file("JHN.usfm", options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_mysword_archive() -> Vec<u8> {
        let temp = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute(
            "CREATE TABLE Bible (Book INTEGER, Chapter INTEGER, Verse INTEGER, Scripture TEXT)",
            [],
        )
        .unwrap();
        // Include variant marker to test stripping
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (
                1,
                1,
                1,
                "Na počiatku stvoril Boh nebo a zem. <f>[++]</f> [ Var.: + vám.]",
            ),
        )
        .unwrap();
        let bytes = std::fs::read(temp.path()).unwrap();

        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file("SEB.bbl.mybible", options).unwrap();
            writer.write_all(&bytes).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_obohu_archive(filename: &str) -> Vec<u8> {
        let temp = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute(
            "CREATE TABLE books (book_number INTEGER PRIMARY KEY, long_name TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE verses (book_number INTEGER, chapter INTEGER, verse INTEGER, text TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (book_number, long_name) VALUES (?1, ?2)",
            (10, "Genezis"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verses (book_number, chapter, verse, text) VALUES (?1, ?2, ?3, ?4)",
            (10, 1, 1, "V počiatku stvoril Boh nebo a zem."),
        )
        .unwrap();
        let bytes = std::fs::read(temp.path()).unwrap();

        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file(filename, options).unwrap();
            writer.write_all(&bytes).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    #[tokio::test]
    async fn parses_usfm_archive_into_passages() {
        let archive = make_sample_archive();
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let translation = sample_translation();
        let spec = BibleTranslationSpec {
            translation: translation.clone(),
            source: BibleSource::Url {
                url: "memory".to_string(),
            },
            format: BibleSourceFormat::UsfmZip {
                book_name_overrides: HashMap::new(),
            },
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "en-test");
        assert_eq!(summary.passage_count, 2);
        assert_eq!(batch.translation(), &translation);
        let passages = batch.passages();
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0].reference.book, "John");
        assert_eq!(passages[0].reference.chapter, 3);
        assert_eq!(passages[0].reference.verse_start, 16);
        assert_eq!(passages[0].text, "For God so loved the world");
        assert_eq!(passages[1].text, "that He gave");
    }

    #[tokio::test]
    async fn strips_strongs_concordance_markup_from_usfm() {
        let mut archive = Vec::new();
        {
            let cursor = Cursor::new(&mut archive);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            let content = "\\id 1PE 1 Peter\n\\c 1\n\\v 1 \\w Peter|strong=\"G4074\"\\w*, an \\w apostle|strong=\"G0652\"\\w* of \\w Jesus|strong=\"G2424\"\\w* \\w Christ|strong=\"G5547\"\\w*\n";
            writer.start_file("1PE.usfm", options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let translation = sample_translation();
        let spec = BibleTranslationSpec {
            translation: translation.clone(),
            source: BibleSource::Url {
                url: "memory".to_string(),
            },
            format: BibleSourceFormat::UsfmZip {
                book_name_overrides: HashMap::new(),
            },
        };

        let (batch, _) = scraper.scrape(&spec).await.unwrap();
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].text, "Peter, an apostle of Jesus Christ");
    }

    #[test]
    fn default_specs_include_expected_translations() {
        let codes: HashSet<_> = default_translation_specs()
            .into_iter()
            .map(|spec| spec.translation.code.clone())
            .collect();
        assert!(codes.contains("eng-kjv"));
        assert!(codes.contains("slk-seb"));
        assert!(codes.contains("slk-roh"));
        assert!(codes.contains("slk-sevp"));
        assert!(codes.contains("slk-mil"));
    }

    #[tokio::test]
    async fn default_spec_overrides_psalms_label() {
        let mut spec = default_translation_specs()
            .into_iter()
            .find(|spec| spec.translation.code == "eng-kjv")
            .expect("eng-kjv spec");
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let mut archive = Vec::new();
        {
            use zip::write::FileOptions;
            let cursor = Cursor::new(&mut archive);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            let content = "\\id PSA Psalms\n\\c 23\n\\v 1 The Lord is my shepherd\n";
            writer.start_file("PSA.usfm", options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let (batch, _) = scraper.scrape(&spec).await.unwrap();
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Psalms");
        assert_eq!(passages[0].reference.chapter, 23);
        assert_eq!(passages[0].reference.verse_start, 1);
    }

    #[tokio::test]
    async fn parses_mysword_archive_into_passages() {
        let archive = make_mysword_archive();
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let mut spec = slovak_ekumenicky_spec();
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "slk-seb");
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Genezis");
        assert_eq!(passages[0].reference.chapter, 1);
        assert_eq!(passages[0].reference.verse_start, 1);
        assert_eq!(passages[0].text, "Na počiatku stvoril Boh nebo a zem.");
    }

    #[tokio::test]
    async fn parses_obohu_archive_into_passages() {
        let archive = make_obohu_archive("SEVP.SQLite3");
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let mut spec = slovak_evangelicky_spec();
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "slk-sevp");
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Genezis");
        assert_eq!(passages[0].reference.chapter, 1);
        assert_eq!(passages[0].reference.verse_start, 1);
        assert_eq!(passages[0].text, "V počiatku stvoril Boh nebo a zem.");
    }
}
