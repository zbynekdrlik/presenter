use crate::spec::{BibleImportSummary, BibleSourceFormat, BibleTranslationSpec};
use anyhow::{anyhow, Context, Result};
use presenter_core::bible::{
    canonical_book_by_code, canonical_book_by_name, canonical_book_by_number, BibleBookCanonical,
    BibleIngestionBatch,
};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use rusqlite::{types::ValueRef, Connection};
use std::convert::TryFrom;
use std::io::{Cursor, Read, Write};
use std::str;
use tempfile::NamedTempFile;
use tracing::warn;
use zip::read::ZipArchive;

pub fn build_ingestion_batch(
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

pub fn parse_usfm_zip(
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

pub fn parse_mysword_sqlite_zip(
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

pub fn parse_obohu_sqlite_zip(
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
    let mut book_map = std::collections::HashMap::new();
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
            warn!(
                code,
                "canonical mapping missing; falling back to name-only reference"
            );
            BibleReference::new(book.clone(), chapter, verse, verse)?
        };
        let passage = BiblePassage::new(reference, translation.clone(), content);
        passages.push(passage);
    }

    Ok(passages)
}

fn parse_usfm_document(
    text: &str,
    translation: &BibleTranslation,
    format: &BibleSourceFormat,
) -> Result<Vec<BiblePassage>> {
    let mut passages = Vec::new();

    let mut book_code: Option<String> = None;
    let mut book_name: Option<String> = None;
    let mut book_meta: Option<BibleBookCanonical> = None;
    let mut current_chapter: Option<u16> = None;
    let mut current_builder: Option<PassageBuilder> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\id ") {
            let mut parts = rest.split_whitespace();
            let code = parts
                .next()
                .ok_or_else(|| anyhow!("missing book code in \\id marker"))?;
            let code = code.trim();
            if code.len() < 3 {
                return Err(anyhow!("invalid USFM book code '{code}'"));
            }
            let trailing_name = parts.collect::<Vec<_>>().join(" ");
            book_code = Some(code.to_string());
            let mut resolved_name = if trailing_name.is_empty() {
                format.book_name(code)
            } else {
                Some(trailing_name)
            };
            if resolved_name.is_none() {
                resolved_name = format.book_name(code);
            }
            book_name = resolved_name;
            book_meta = canonical_book_by_code(code);
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\h ") {
            if let Some(code) = book_code.as_deref() {
                let name = rest.trim();
                if !name.is_empty() {
                    book_name = Some(name.to_string());
                }
                if book_meta.is_none() {
                    book_meta = canonical_book_by_code(code);
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\c ") {
            let c = rest
                .trim()
                .parse::<u16>()
                .context("failed to parse chapter number into u16")?;
            if current_chapter != Some(c) {
                flush_current_passage(
                    &mut passages,
                    &mut current_builder,
                    &book_name,
                    &book_meta,
                    current_chapter,
                    translation,
                )?;
                current_chapter = Some(c);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("\\v ") {
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
