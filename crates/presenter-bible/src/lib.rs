use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use presenter_core::bible::{
    canonical_book_by_code, canonical_book_by_name, canonical_book_by_number, BibleBookCanonical,
    BibleIngestionBatch,
};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use rusqlite::{types::ValueRef, Connection};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::str;
use std::time::Duration;
use tempfile::NamedTempFile;
use tracing::warn;
use zip::read::ZipArchive;

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
    fn book_name<'a>(&'a self, code: &str) -> Option<String> {
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

pub struct BibleScraper<P: BibleContentProvider> {
    provider: P,
}

impl<P: BibleContentProvider> BibleScraper<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    pub async fn scrape(
        &self,
        spec: &BibleTranslationSpec,
    ) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
        match (&spec.source, &spec.format) {
            (BibleSource::Url { url }, BibleSourceFormat::UsfmZip { .. })
            | (BibleSource::Url { url }, BibleSourceFormat::MySwordSqlite { .. })
            | (BibleSource::Url { url }, BibleSourceFormat::ObohuSqlite) => {
                let bytes = self
                    .provider
                    .fetch_bytes(url)
                    .await
                    .with_context(|| format!("failed to fetch bible archive from {}", url))?;
                build_ingestion_batch(&bytes, spec)
            }
            (
                BibleSource::LocalFile { env_var, .. },
                BibleSourceFormat::UsfmZip { .. }
                | BibleSourceFormat::MySwordSqlite { .. }
                | BibleSourceFormat::ObohuSqlite,
            ) => {
                let path = std::env::var(env_var).with_context(|| {
                    format!(
                        "environment variable {} must be set to the bible archive for {}",
                        env_var, spec.translation.code
                    )
                })?;
                let bytes = std::fs::read(&path).with_context(|| {
                    format!(
                        "failed to read bible archive for {} from {}",
                        spec.translation.code, path
                    )
                })?;
                build_ingestion_batch(&bytes, spec)
            }
        }
    }
}

pub fn default_translation_specs() -> Vec<BibleTranslationSpec> {
    vec![
        king_james_spec(),
        slovak_ekumenicky_spec(),
        slovak_rohacek_spec(),
        slovak_evangelicky_spec(),
    ]
}

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

fn slovak_ekumenicky_spec() -> BibleTranslationSpec {
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

fn slovak_rohacek_spec() -> BibleTranslationSpec {
    BibleTranslationSpec {
        translation: BibleTranslation::new("slk-roh", "Roháčkov preklad", "sk")
            .with_source(SLOVAK_ROHACEK_SOURCE),
        source: BibleSource::Url {
            url: SLOVAK_ROHACEK_SOURCE.to_string(),
        },
        format: BibleSourceFormat::ObohuSqlite,
    }
}

fn slovak_evangelicky_spec() -> BibleTranslationSpec {
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

fn build_ingestion_batch(
    bytes: &[u8],
    spec: &BibleTranslationSpec,
) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
    let passages = match &spec.format {
        BibleSourceFormat::UsfmZip { .. } => {
            parse_usfm_zip(bytes, &spec.translation, &spec.format)?
        }
        BibleSourceFormat::MySwordSqlite { book_names } => {
            parse_mysword_sqlite_zip(bytes, &spec.translation, book_names)?
        }
        BibleSourceFormat::ObohuSqlite => parse_obohu_sqlite_zip(bytes, &spec.translation)?,
    };
    let passage_count = passages.len();
    let batch = BibleIngestionBatch::new(spec.translation.clone(), passages)?;
    let summary = BibleImportSummary {
        translation_code: spec.translation.code.clone(),
        passage_count,
    };
    Ok((batch, summary))
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

fn parse_usfm_zip(
    bytes: &[u8],
    translation: &BibleTranslation,
    format: &BibleSourceFormat,
) -> Result<Vec<BiblePassage>> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).context("failed to read bible archive as zip")?;
    let mut passages = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if !file.is_file() {
            continue;
        }
        let name = file.name().to_string();
        if !name.ends_with(".usfm") {
            continue;
        }
        let mut contents = String::new();
        use std::io::Read;
        file.read_to_string(&mut contents)?;
        let mut doc_passages = parse_usfm_document(&contents, translation, format)?;
        passages.append(&mut doc_passages);
    }

    Ok(passages)
}

fn parse_mysword_sqlite_zip(
    bytes: &[u8],
    translation: &BibleTranslation,
    book_names: &[String],
) -> Result<Vec<BiblePassage>> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).context("failed to read MySword archive as zip")?;
    let mut sqlite_bytes: Option<Vec<u8>> = None;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if !file.is_file() {
            continue;
        }
        let name = file.name().to_ascii_lowercase();
        if name.ends_with(".bbl.mybible") {
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            sqlite_bytes = Some(data);
            break;
        }
    }

    let sqlite_bytes = sqlite_bytes
        .ok_or_else(|| anyhow!("MySword archive did not contain a *.bbl.mybible database"))?;

    let mut temp =
        NamedTempFile::new().context("failed to create temp file for MySword database")?;
    temp.write_all(&sqlite_bytes)
        .context("failed to write MySword database to temp file")?;

    let conn = Connection::open(temp.path())
        .context("failed to open MySword SQLite database from archive")?;
    let mut stmt = conn.prepare(
        "SELECT Book, Chapter, Verse, Scripture FROM Bible ORDER BY Book, Chapter, Verse",
    )?;

    let mut rows = stmt.query([])?;
    let mut passages = Vec::new();

    while let Some(row) = rows.next()? {
        let book_index = match row.get_ref(0)? {
            ValueRef::Integer(value) => value,
            ValueRef::Real(value) => value as i64,
            ValueRef::Text(text) => {
                let raw = str::from_utf8(text)?.trim();
                match raw.parse::<i64>() {
                    Ok(value) => value,
                    Err(_) => {
                        warn!(
                            book = raw,
                            "skipping MySword row with non-numeric book code"
                        );
                        continue;
                    }
                }
            }
            ValueRef::Null => {
                warn!("skipping MySword row without book code");
                continue;
            }
            other => {
                warn!(data_type = ?other.data_type(), "skipping MySword row with unsupported book column type");
                continue;
            }
        };

        let chapter = read_u16(&row, 1, "Chapter")?;
        let verse = read_u16(&row, 2, "Verse")?;
        let scripture: String = row.get(3)?;

        if book_index < 1 {
            warn!(book_index, "skipping MySword row with invalid book index");
            continue;
        }
        let book_vec_index = usize::try_from(book_index - 1)
            .context("MySword book index does not fit into usize")?;
        let book = book_names.get(book_vec_index).cloned().ok_or_else(|| {
            anyhow!(
                "book index {} is not mapped for translation {}",
                book_index,
                translation.code
            )
        })?;

        let canonical_number = match u16::try_from(book_index) {
            Ok(value) if value >= 1 => Some(value),
            _ => {
                warn!(
                    book_index,
                    "skipping MySword row with unsupported book index"
                );
                None
            }
        };
        let canonical_meta = canonical_book_by_name(&book)
            .or_else(|| canonical_number.and_then(canonical_book_by_number));

        let text = sanitize_mysword_text(&scripture);
        if text.is_empty() {
            continue;
        }

        let reference = if let Some(meta) = canonical_meta {
            BibleReference::new_with_code(
                book.clone(),
                meta.code,
                meta.number,
                chapter,
                verse,
                verse,
            )?
        } else {
            warn!(
                book_index,
                "canonical mapping missing for MySword book index"
            );
            BibleReference::new(book.clone(), chapter, verse, verse)?
        };
        let passage = BiblePassage::new(reference, translation.clone(), text);
        passages.push(passage);
    }

    Ok(passages)
}

fn canonical_number_from_dataset(code: i64) -> Option<u16> {
    if code <= 0 {
        return None;
    }
    let mut value = code;
    while value > 66 && value % 10 == 0 {
        value /= 10;
    }
    if (1..=66).contains(&value) {
        return u16::try_from(value).ok();
    }
    None
}

fn parse_obohu_sqlite_zip(
    bytes: &[u8],
    translation: &BibleTranslation,
) -> Result<Vec<BiblePassage>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .context("failed to read Slovak SQLite archive as zip")?;

    let mut primary_bytes: Option<Vec<u8>> = None;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if !file.is_file() {
            continue;
        }
        let name = file.name().to_ascii_lowercase();
        if !name.ends_with(".sqlite3") {
            continue;
        }
        if name.contains("subheadings") || name.contains("commentaries") {
            continue;
        }
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        primary_bytes = Some(data);
        break;
    }

    let primary_bytes = primary_bytes
        .ok_or_else(|| anyhow!("Slovak archive missing primary *.SQLite3 bible file"))?;

    let mut temp =
        NamedTempFile::new().context("failed to create temp file for Slovak SQLite database")?;
    temp.write_all(&primary_bytes)
        .context("failed to write Slovak SQLite database to temp file")?;

    let conn =
        Connection::open(temp.path()).context("failed to open Slovak SQLite bible database")?;

    let mut book_stmt =
        conn.prepare("SELECT book_number, long_name FROM books ORDER BY book_number")?;
    let mut book_rows = book_stmt.query([])?;
    let mut book_map = HashMap::new();
    while let Some(row) = book_rows.next()? {
        let code: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        book_map.insert(code, name);
    }

    let mut verse_stmt = conn.prepare(
        "SELECT book_number, chapter, verse, text FROM verses ORDER BY book_number, chapter, verse",
    )?;
    let mut verse_rows = verse_stmt.query([])?;
    let mut passages = Vec::new();

    while let Some(row) = verse_rows.next()? {
        let code = read_i64(&row, 0, "book_number")?;
        let chapter = read_u16(&row, 1, "chapter")?;
        let verse = read_u16(&row, 2, "verse")?;
        let text: String = row.get(3)?;

        let Some(book) = book_map.get(&code) else {
            warn!(code, "skipping verse with unmapped book code");
            continue;
        };
        let content = sanitize_markup_text(&text);
        if content.is_empty() {
            continue;
        }

        let canonical_meta = canonical_book_by_name(book)
            .or_else(|| canonical_number_from_dataset(code).and_then(canonical_book_by_number));
        let reference = if let Some(meta) = canonical_meta {
            BibleReference::new_with_code(
                book.clone(),
                meta.code,
                meta.number,
                chapter,
                verse,
                verse,
            )?
        } else {
            warn!(code, "canonical mapping missing for Slovak book number");
            BibleReference::new(book.clone(), chapter, verse, verse)?
        };

        let passage = BiblePassage::new(reference, translation.clone(), content);
        passages.push(passage);
    }

    Ok(passages)
}

fn parse_usfm_document(
    contents: &str,
    translation: &BibleTranslation,
    format: &BibleSourceFormat,
) -> Result<Vec<BiblePassage>> {
    let mut book_name: Option<String> = None;
    let mut book_meta: Option<BibleBookCanonical> = None;
    let mut current_chapter: Option<u16> = None;
    let mut current_builder: Option<PassageBuilder> = None;
    let mut passages = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("\\id ") {
            flush_current_passage(
                &mut passages,
                &mut current_builder,
                &book_name,
                &book_meta,
                current_chapter,
                translation,
            )?;
            current_chapter = None;
            let mut parts = rest.split_whitespace();
            let code = parts
                .next()
                .ok_or_else(|| anyhow!("USFM document missing book code in \\id marker"))?;
            book_meta = canonical_book_by_code(code);
            match format.book_name(code) {
                Some(name) => {
                    if book_meta.is_none() {
                        if let Some(meta) = canonical_book_by_name(&name) {
                            book_meta = Some(meta);
                        } else {
                            warn!(%code, "USFM book code lacks canonical mapping; alignment may be degraded");
                        }
                    }
                    book_name = Some(name);
                }
                None => {
                    warn!(%code, "skipping unsupported USFM book code");
                    book_name = None;
                    book_meta = None;
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\c ") {
            flush_current_passage(
                &mut passages,
                &mut current_builder,
                &book_name,
                &book_meta,
                current_chapter,
                translation,
            )?;
            if book_name.is_none() {
                continue;
            }
            let chapter_num: u16 = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| anyhow!("missing chapter number after \\c marker"))?
                .parse()
                .context("chapter value is not a number")?;
            current_chapter = Some(chapter_num);
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\v ") {
            if book_name.is_none() {
                continue;
            }
            flush_current_passage(
                &mut passages,
                &mut current_builder,
                &book_name,
                &book_meta,
                current_chapter,
                translation,
            )?;
            let (verse_range, text) = rest
                .split_once(' ')
                .ok_or_else(|| anyhow!("missing verse text after \\v marker"))?;
            let (start, end) = parse_verse_range(verse_range)?;
            let clean_text = sanitize_text(text);
            current_builder = Some(PassageBuilder::new(start, end, clean_text));
            continue;
        }

        if let Some(builder) = current_builder.as_mut() {
            let clean_text = sanitize_text(line);
            if !clean_text.is_empty() {
                builder.push_text(&clean_text);
            }
        }
    }

    flush_current_passage(
        &mut passages,
        &mut current_builder,
        &book_name,
        &book_meta,
        current_chapter,
        translation,
    )?;

    Ok(passages)
}

fn read_i64(row: &rusqlite::Row<'_>, index: usize, column: &str) -> Result<i64> {
    match row.get_ref(index)? {
        ValueRef::Integer(value) => Ok(value),
        ValueRef::Real(value) => Ok(value as i64),
        ValueRef::Text(text) => {
            let s = str::from_utf8(text)?;
            s.trim()
                .parse::<i64>()
                .with_context(|| format!("column {column} contained '{s}' which is not an integer"))
        }
        ValueRef::Null => Err(anyhow!("column {column} was NULL")),
        other => Err(anyhow!(
            "column {column} has unsupported type {:?}",
            other.data_type()
        )),
    }
}

fn read_u16(row: &rusqlite::Row<'_>, index: usize, column: &str) -> Result<u16> {
    let value = read_i64(row, index, column)?;
    u16::try_from(value)
        .with_context(|| format!("column {column} value {value} does not fit in u16"))
}

fn flush_current_passage(
    passages: &mut Vec<BiblePassage>,
    builder: &mut Option<PassageBuilder>,
    book_name: &Option<String>,
    book_meta: &Option<BibleBookCanonical>,
    chapter: Option<u16>,
    translation: &BibleTranslation,
) -> Result<()> {
    if let Some(current) = builder.take() {
        let Some(book) = book_name.clone() else {
            warn!("dropping passage without book context");
            return Ok(());
        };
        let Some(chapter) = chapter else {
            warn!(%book, "dropping passage without chapter context");
            return Ok(());
        };
        let reference = if let Some(meta) = book_meta {
            BibleReference::new_with_code(
                book.clone(),
                meta.code,
                meta.number,
                chapter,
                current.start,
                current.end,
            )?
        } else {
            warn!(book = %book, "canonical mapping missing; falling back to name-only reference");
            BibleReference::new(book.clone(), chapter, current.start, current.end)?
        };
        let passage = BiblePassage::new(reference, translation.clone(), current.text);
        passages.push(passage);
    }
    Ok(())
}

fn parse_verse_range(token: &str) -> Result<(u16, u16)> {
    let token = token.trim();
    if let Some((start, end)) = token.split_once('-') {
        let start = parse_verse_number(start)?;
        let end = parse_verse_number(end)?;
        Ok((start, end))
    } else {
        let num = parse_verse_number(token)?;
        Ok((num, num))
    }
}

fn parse_verse_number(value: &str) -> Result<u16> {
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(anyhow!("verse identifier '{value}' did not contain digits"));
    }
    digits
        .parse::<u16>()
        .context("failed to parse verse number into u16")
}

fn sanitize_text(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // consume marker name
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                chars.next();
            }
            // consume trailing whitespace between marker and content
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            continue;
        }
        result.push(ch);
    }
    result
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_mysword_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        if starts_with_ci(rest, "<f") {
            if let Some(end) = rest.find("</f>") {
                i += end + 4;
                continue;
            }
        }
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                i += end + 1;
                continue;
            } else {
                break;
            }
        }
        let ch = rest.chars().next().unwrap();
        output.push(ch);
        i += ch.len_utf8();
    }

    sanitize_markup_text(&output)
}

fn sanitize_markup_text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut inside_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if inside_tag => {}
            _ => result.push(ch),
        }
    }

    let replaced = result
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"");

    replaced
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn starts_with_ci(haystack: &str, needle: &str) -> bool {
    let h_bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    if h_bytes.len() < n_bytes.len() {
        return false;
    }
    for (hb, nb) in h_bytes.iter().zip(n_bytes.iter()) {
        if hb.to_ascii_lowercase() != nb.to_ascii_lowercase() {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone)]
struct PassageBuilder {
    start: u16,
    end: u16,
    text: String,
}

impl PassageBuilder {
    fn new(start: u16, end: u16, text: String) -> Self {
        Self { start, end, text }
    }

    fn push_text(&mut self, text: &str) {
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(text);
    }
}

fn default_book_name(code: &str) -> Option<&'static str> {
    canonical_book_by_code(code).map(|meta| meta.english_name)
}

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
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (1, 1, 1, "Na počiatku stvoril Boh nebo a zem. <f>[++]</f>"),
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
        let archive = make_obohu_archive("ROH-AV.SQLite3");
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let mut spec = slovak_rohacek_spec();
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "slk-roh");
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Genezis");
        assert_eq!(passages[0].reference.chapter, 1);
        assert_eq!(passages[0].reference.verse_start, 1);
        assert_eq!(passages[0].text, "V počiatku stvoril Boh nebo a zem.");
    }
}
