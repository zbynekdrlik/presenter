//! Bible archive parsing: USFM, MySword SQLite, and Obohu SQLite formats.

use anyhow::{anyhow, Context, Result};
use presenter_core::{bible::BibleIngestionBatch, BiblePassage, BibleReference, BibleTranslation};
use regex::Regex;
use rusqlite::{types::ValueRef, Connection};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io::{Cursor, Read, Write};
use std::str;
use std::sync::LazyLock;
use tempfile::NamedTempFile;
use tracing::warn;
use zip::read::ZipArchive;

use crate::{BibleImportSummary, BibleSourceFormat, BibleTranslationSpec};

/// Regex to strip variant markers like `[ Var.: + vám.]` or `[ Var.: vaša.]`
static VARIANT_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\s*Var\.:[^\]]*\]").expect("invalid variant marker regex"));

pub(crate) fn build_ingestion_batch(
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

        let chapter = read_u16(row, 1, "Chapter")?;
        let verse = read_u16(row, 2, "Verse")?;
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

        let text = sanitize_mysword_text(&scripture);
        if text.is_empty() {
            continue;
        }

        // MySword book numbers 1-66 map directly to standard book numbering
        let book_number =
            u16::try_from(book_index).context("MySword book index does not fit into u16")?;
        let book_code = mysword_book_code(book_number);
        let reference =
            BibleReference::new_with_code(&book, book_code, book_number, chapter, verse, verse)?;
        let passage = BiblePassage::new(reference, translation.clone(), text);
        passages.push(passage);
    }

    Ok(passages)
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
        let code = read_i64(row, 0, "book_number")?;
        let chapter = read_u16(row, 1, "chapter")?;
        let verse = read_u16(row, 2, "verse")?;
        let text: String = row.get(3)?;

        let Some(book) = book_map.get(&code) else {
            warn!(code, "skipping verse with unmapped book code");
            continue;
        };
        let content = sanitize_markup_text(&text);
        if content.is_empty() {
            continue;
        }
        let reference = BibleReference::new(book.clone(), chapter, verse, verse)?;
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
                current_chapter,
                translation,
            )?;
            current_chapter = None;
            let mut parts = rest.split_whitespace();
            let code = parts
                .next()
                .ok_or_else(|| anyhow!("USFM document missing book code in \\id marker"))?;
            match format.book_name(code) {
                Some(name) => book_name = Some(name),
                None => {
                    warn!(%code, "skipping unsupported USFM book code");
                    book_name = None;
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\c ") {
            flush_current_passage(
                &mut passages,
                &mut current_builder,
                &book_name,
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
        let reference = BibleReference::new(book, chapter, current.start, current.end)?;
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
            // consume marker name (letters, digits, +, and closing *)
            let mut is_closing = false;
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '+' {
                    chars.next();
                } else if next == '*' {
                    is_closing = true;
                    chars.next();
                } else {
                    break;
                }
            }
            // consume trailing whitespace only for opening markers, not
            // closing markers like \w* where the space is content spacing
            if !is_closing {
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        // Strip USFM word-level attributes: |strong="G1234" etc.
        // These appear between the word text and the closing \w* marker.
        if ch == '|' {
            while let Some(&next) = chars.peek() {
                if next == '\\' {
                    break;
                }
                chars.next();
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
    // First strip variant markers like [ Var.: + vám.] or [ Var.: vaša.]
    let without_variants = VARIANT_MARKER_RE.replace_all(input, "");

    let mut output = String::with_capacity(without_variants.len());
    let mut i = 0;
    while i < without_variants.len() {
        let rest = &without_variants[i..];
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
        // Safety: rest is non-empty since i < without_variants.len() and we slice at valid UTF-8 boundaries
        if let Some(ch) = rest.chars().next() {
            output.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
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
    h_bytes
        .iter()
        .zip(n_bytes.iter())
        .all(|(hb, nb)| hb.eq_ignore_ascii_case(nb))
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

/// Convert MySword book number (1-66) to standard 3-letter book code
fn mysword_book_code(book_number: u16) -> &'static str {
    match book_number {
        1 => "GEN",
        2 => "EXO",
        3 => "LEV",
        4 => "NUM",
        5 => "DEU",
        6 => "JOS",
        7 => "JDG",
        8 => "RUT",
        9 => "1SA",
        10 => "2SA",
        11 => "1KI",
        12 => "2KI",
        13 => "1CH",
        14 => "2CH",
        15 => "EZR",
        16 => "NEH",
        17 => "EST",
        18 => "JOB",
        19 => "PSA",
        20 => "PRO",
        21 => "ECC",
        22 => "SNG",
        23 => "ISA",
        24 => "JER",
        25 => "LAM",
        26 => "EZK",
        27 => "DAN",
        28 => "HOS",
        29 => "JOL",
        30 => "AMO",
        31 => "OBA",
        32 => "JON",
        33 => "MIC",
        34 => "NAM",
        35 => "HAB",
        36 => "ZEP",
        37 => "HAG",
        38 => "ZEC",
        39 => "MAL",
        40 => "MAT",
        41 => "MRK",
        42 => "LUK",
        43 => "JHN",
        44 => "ACT",
        45 => "ROM",
        46 => "1CO",
        47 => "2CO",
        48 => "GAL",
        49 => "EPH",
        50 => "PHP",
        51 => "COL",
        52 => "1TH",
        53 => "2TH",
        54 => "1TI",
        55 => "2TI",
        56 => "TIT",
        57 => "PHM",
        58 => "HEB",
        59 => "JAS",
        60 => "1PE",
        61 => "2PE",
        62 => "1JN",
        63 => "2JN",
        64 => "3JN",
        65 => "JUD",
        66 => "REV",
        _ => "UNK",
    }
}

pub(crate) fn default_book_name(code: &str) -> Option<&'static str> {
    match code {
        "GEN" => Some("Genesis"),
        "EXO" => Some("Exodus"),
        "LEV" => Some("Leviticus"),
        "NUM" => Some("Numbers"),
        "DEU" => Some("Deuteronomy"),
        "JOS" => Some("Joshua"),
        "JDG" => Some("Judges"),
        "RUT" => Some("Ruth"),
        "1SA" => Some("1 Samuel"),
        "2SA" => Some("2 Samuel"),
        "1KI" => Some("1 Kings"),
        "2KI" => Some("2 Kings"),
        "1CH" => Some("1 Chronicles"),
        "2CH" => Some("2 Chronicles"),
        "EZR" => Some("Ezra"),
        "NEH" => Some("Nehemiah"),
        "EST" => Some("Esther"),
        "JOB" => Some("Job"),
        "PSA" => Some("Psalm"),
        "PRO" => Some("Proverbs"),
        "ECC" => Some("Ecclesiastes"),
        "SNG" => Some("Song of Songs"),
        "ISA" => Some("Isaiah"),
        "JER" => Some("Jeremiah"),
        "LAM" => Some("Lamentations"),
        "EZK" => Some("Ezekiel"),
        "DAN" => Some("Daniel"),
        "HOS" => Some("Hosea"),
        "JOL" => Some("Joel"),
        "AMO" => Some("Amos"),
        "OBA" => Some("Obadiah"),
        "JON" => Some("Jonah"),
        "MIC" => Some("Micah"),
        "NAM" => Some("Nahum"),
        "HAB" => Some("Habakkuk"),
        "ZEP" => Some("Zephaniah"),
        "HAG" => Some("Haggai"),
        "ZEC" => Some("Zechariah"),
        "MAL" => Some("Malachi"),
        "MAT" => Some("Matthew"),
        "MRK" => Some("Mark"),
        "LUK" => Some("Luke"),
        "JHN" => Some("John"),
        "ACT" => Some("Acts"),
        "ROM" => Some("Romans"),
        "1CO" => Some("1 Corinthians"),
        "2CO" => Some("2 Corinthians"),
        "GAL" => Some("Galatians"),
        "EPH" => Some("Ephesians"),
        "PHP" => Some("Philippians"),
        "COL" => Some("Colossians"),
        "1TH" => Some("1 Thessalonians"),
        "2TH" => Some("2 Thessalonians"),
        "1TI" => Some("1 Timothy"),
        "2TI" => Some("2 Timothy"),
        "TIT" => Some("Titus"),
        "PHM" => Some("Philemon"),
        "HEB" => Some("Hebrews"),
        "JAS" => Some("James"),
        "1PE" => Some("1 Peter"),
        "2PE" => Some("2 Peter"),
        "1JN" => Some("1 John"),
        "2JN" => Some("2 John"),
        "3JN" => Some("3 John"),
        "JUD" => Some("Jude"),
        "REV" => Some("Revelation"),
        _ => None,
    }
}
