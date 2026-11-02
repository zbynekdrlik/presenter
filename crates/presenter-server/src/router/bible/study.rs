//! The study page's read API (#851 spec §11). Query parameters are snake_case,
//! JSON camelCase, like the other Bible routes. `GET /bible/study/identity`
//! lives with the sermons; every verse read goes through `state/bible_source`.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use presenter_core::bible::{normalize_strongs, BookScope, LexiconEntry, VerseMarkup};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use crate::state::bible_source::study::{
    ChapterPlace, StudyChapter, StudyCrossReference, StudyVerse,
};
use crate::state::AppState;

const MAX_OCCURRENCES_PAGE: u32 = 100;
const MAX_CROSS_REFERENCES: usize = 500;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/bible/study/chapter", get(chapter))
        .route("/bible/study/lexicon/{strongs}", get(lexicon))
        .route("/bible/study/strongs/{strongs}", get(strongs))
        .route("/bible/study/crossrefs", get(crossrefs))
}

/// The `scope` parameter of the Strong's view and `/bible/search`.
pub(super) fn parse_scope(scope: Option<&str>) -> Result<BookScope, AppError> {
    BookScope::parse(scope.unwrap_or_default())
        .ok_or_else(|| AppError::bad_request_message("Neznámy rozsah hľadania"))
}

fn parse_strongs(raw: &str) -> Result<String, AppError> {
    normalize_strongs(raw).ok_or_else(|| AppError::bad_request_message("Neplatné Strongovo číslo"))
}

#[derive(Debug, Deserialize)]
struct ChapterQuery {
    translation: String,
    book_code: String,
    chapter: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChapterPlaceDto {
    book_code: String,
    chapter: u16,
}

impl From<ChapterPlace> for ChapterPlaceDto {
    fn from(place: ChapterPlace) -> Self {
        Self {
            book_code: place.book_code,
            chapter: place.chapter,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StudyVerseDto {
    verse: u16,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    markup: Option<VerseMarkup>,
}

impl From<StudyVerse> for StudyVerseDto {
    fn from(verse: StudyVerse) -> Self {
        Self {
            verse: verse.verse,
            text: verse.text,
            markup: verse.markup,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StudyChapterDto {
    translation: String,
    book_code: String,
    book_number: u16,
    book: String,
    chapter: u16,
    verse_count: u16,
    prev: Option<ChapterPlaceDto>,
    next: Option<ChapterPlaceDto>,
    verses: Vec<StudyVerseDto>,
}

impl StudyChapterDto {
    fn new(translation: String, chapter: StudyChapter) -> Self {
        Self {
            translation,
            book_code: chapter.book_code,
            book_number: chapter.book_number,
            book: chapter.book,
            chapter: chapter.chapter,
            verse_count: chapter.verse_count,
            prev: chapter.prev.map(Into::into),
            next: chapter.next.map(Into::into),
            verses: chapter.verses.into_iter().map(Into::into).collect(),
        }
    }
}

#[instrument(skip(state))]
async fn chapter(
    State(state): State<AppState>,
    Query(query): Query<ChapterQuery>,
) -> Result<Json<StudyChapterDto>, AppError> {
    let chapter = state
        .study_chapter(&query.translation, &query.book_code, query.chapter)
        .await?
        .ok_or_else(|| AppError::not_found("Preklad alebo kniha neexistuje"))?;
    Ok(Json(StudyChapterDto::new(query.translation, chapter)))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LexiconDto {
    strongs: String,
    lemma: String,
    transliteration: String,
    pronunciation: String,
    definition: String,
    derivation: String,
    kjv_renderings: String,
    related: Vec<String>,
    occurrence_count: u64,
}

impl LexiconDto {
    fn new(entry: LexiconEntry, occurrence_count: u64) -> Self {
        Self {
            strongs: entry.strongs,
            lemma: entry.lemma,
            transliteration: entry.transliteration,
            pronunciation: entry.pronunciation,
            definition: entry.definition,
            derivation: entry.derivation,
            kjv_renderings: entry.kjv_renderings,
            related: entry.related,
            occurrence_count,
        }
    }
}

#[instrument(skip(state))]
async fn lexicon(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<LexiconDto>, AppError> {
    let strongs = parse_strongs(&raw)?;
    let (entry, count) = state
        .study_lexicon(&strongs)
        .await?
        .ok_or_else(|| AppError::not_found("Heslo v slovníku neexistuje"))?;
    Ok(Json(LexiconDto::new(entry, count)))
}

#[derive(Debug, Deserialize)]
struct StrongsQuery {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BookCountDto {
    book_code: String,
    count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StrongsItemDto {
    book_code: String,
    book: String,
    chapter: u16,
    verse: u16,
    text: String,
    markup: Option<VerseMarkup>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StrongsPageDto {
    strongs: String,
    total: u64,
    per_book: Vec<BookCountDto>,
    items: Vec<StrongsItemDto>,
}

#[instrument(skip(state))]
async fn strongs(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Query(query): Query<StrongsQuery>,
) -> Result<Json<StrongsPageDto>, AppError> {
    let strongs = parse_strongs(&raw)?;
    let scope = parse_scope(query.scope.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(MAX_OCCURRENCES_PAGE)
        .clamp(1, MAX_OCCURRENCES_PAGE);
    let page = state
        .study_strongs(&strongs, scope, query.offset, limit)
        .await?;
    Ok(Json(StrongsPageDto {
        strongs,
        total: page.total,
        per_book: page
            .per_book
            .into_iter()
            .map(|(book_code, count)| BookCountDto { book_code, count })
            .collect(),
        items: page
            .items
            .into_iter()
            .map(|verse| StrongsItemDto {
                book_code: verse.book_code,
                book: verse.book,
                chapter: verse.chapter,
                verse: verse.verse,
                text: verse.text,
                markup: verse.markup,
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
struct CrossrefsQuery {
    translation: String,
    book_code: String,
    chapter: u16,
    verse: u16,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrossReferenceDto {
    label: String,
    book_code: String,
    chapter: u16,
    verse: u16,
    chapter_end: u16,
    verse_end: u16,
    votes: i32,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_translation: Option<String>,
}

impl From<StudyCrossReference> for CrossReferenceDto {
    fn from(item: StudyCrossReference) -> Self {
        Self {
            label: item.label,
            book_code: item.book_code,
            chapter: item.chapter,
            verse: item.verse,
            chapter_end: item.chapter_end,
            verse_end: item.verse_end,
            votes: item.votes,
            text: item.text,
            fallback_translation: item.fallback_translation,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrossReferencesDto {
    total: usize,
    items: Vec<CrossReferenceDto>,
}

#[instrument(skip(state))]
async fn crossrefs(
    State(state): State<AppState>,
    Query(query): Query<CrossrefsQuery>,
) -> Result<Json<CrossReferencesDto>, AppError> {
    let limit = query.limit.unwrap_or(10).clamp(1, MAX_CROSS_REFERENCES);
    let (total, items) = state
        .study_crossrefs(
            &query.translation,
            &query.book_code,
            query.chapter,
            query.verse,
            limit,
        )
        .await?
        .ok_or_else(|| AppError::not_found("Preklad alebo kniha neexistuje"))?;
    Ok(Json(CrossReferencesDto {
        total,
        items: items.into_iter().map(Into::into).collect(),
    }))
}

#[cfg(test)]
mod tests;
