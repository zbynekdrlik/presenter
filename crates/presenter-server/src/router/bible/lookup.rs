//! `GET /bible/lookup?q=&translation=` (#850 §5.8): what a typed reference
//! means against a translation. The console resolves in the browser; this is
//! the same resolver for everything that is not a browser.

use std::time::Instant;

use axum::extract::{Query, State};
use axum::Json;
use presenter_core::bible::resolver::{segment_label, BookRef, Passage, Resolution, Segment};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use super::resolve::LOOKUP_QUERY_MAX_CHARS;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct BibleLookupQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    translation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupSegment {
    chapter: u16,
    verse_start: Option<u16>,
    verse_end: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupPassage {
    book_code: String,
    book_number: u16,
    book: String,
    matched: Option<String>,
    segments: Vec<LookupSegment>,
    label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleLookupResponse {
    query: String,
    translation: String,
    kind: &'static str,
    default: Option<LookupPassage>,
    alternatives: Vec<LookupPassage>,
    hint: Option<String>,
    fix: Option<LookupPassage>,
}

fn dto_segment(segment: &Segment) -> LookupSegment {
    LookupSegment {
        chapter: segment.chapter,
        verse_start: segment.verse_start,
        verse_end: segment.verse_end,
    }
}

/// `Matúš 5:3, 5:5`; a book alone is its name.
fn label(book: &BookRef, segments: &[Segment]) -> String {
    let mut parts = segments.iter().map(|segment| segment_label("", segment));
    match parts.next() {
        Some(first) => {
            let rest: Vec<String> = parts.map(|part| part.trim().to_string()).collect();
            let head = format!("{}{first}", book.name);
            std::iter::once(head)
                .chain(rest)
                .collect::<Vec<_>>()
                .join(", ")
        }
        None => book.name.clone(),
    }
}

fn dto_passage(book: &BookRef, matched: Option<&str>, segments: &[Segment]) -> LookupPassage {
    LookupPassage {
        book_code: book.code.clone(),
        book_number: book.number,
        book: book.name.clone(),
        matched: matched.map(str::to_string),
        segments: segments.iter().map(dto_segment).collect(),
        label: label(book, segments),
    }
}

fn from_passage(passage: &Passage) -> LookupPassage {
    dto_passage(
        &passage.book,
        Some(passage.matched_form.as_str()),
        &passage.segments,
    )
}

fn response(query: String, translation: String, resolution: &Resolution) -> BibleLookupResponse {
    let passages = |list: &[Passage]| list.iter().map(from_passage).collect::<Vec<_>>();
    let books = |list: &[BookRef]| {
        list.iter()
            .map(|book| dto_passage(book, None, &[]))
            .collect::<Vec<_>>()
    };
    let (kind, default, alternatives, hint, fix) = match resolution {
        Resolution::Passage {
            passage,
            alternatives,
        } => (
            "passage",
            Some(from_passage(passage)),
            passages(alternatives),
            None,
            None,
        ),
        Resolution::Ambiguous {
            default,
            alternatives,
        } => (
            "ambiguous",
            Some(from_passage(default)),
            passages(alternatives),
            None,
            None,
        ),
        Resolution::OutOfRange {
            book,
            requested,
            hint,
            fix,
        } => (
            "outOfRange",
            Some(dto_passage(book, None, std::slice::from_ref(requested))),
            Vec::new(),
            Some(hint.clone()),
            fix.as_ref().map(from_passage),
        ),
        Resolution::BookOnly { book, alternatives } => (
            "bookOnly",
            Some(dto_passage(book, None, &[])),
            books(alternatives),
            None,
            None,
        ),
        Resolution::Incomplete { books: list, .. } => ("incomplete", None, books(list), None, None),
        Resolution::NoBook { hint, .. } => ("noBook", None, Vec::new(), hint.clone(), None),
    };
    BibleLookupResponse {
        query,
        translation,
        kind,
        default,
        alternatives,
        hint,
        fix,
    }
}

/// `first_verse-last_verse` of the answer's first segment, for the log line.
fn log_verses(passage: Option<&LookupPassage>) -> (Option<u16>, String) {
    let first = passage.and_then(|passage| passage.segments.first());
    let verses = first
        .and_then(|segment| Some((segment.verse_start?, segment.verse_end?)))
        .map_or_else(|| "-".to_string(), |(start, end)| format!("{start}-{end}"));
    (first.map(|segment| segment.chapter), verses)
}

#[instrument(skip_all)]
pub(crate) async fn bible_lookup(
    State(state): State<AppState>,
    Query(params): Query<BibleLookupQuery>,
) -> Result<Json<BibleLookupResponse>, AppError> {
    let query = params.q.trim();
    if query.is_empty() {
        return Err(AppError::bad_request_message("q is required"));
    }
    let started = Instant::now();
    let (translation, resolution) = state
        .lookup_bible_reference(query, params.translation.as_deref())
        .await?;
    let answer = response(query.to_string(), translation, &resolution);
    let logged_query: String = query.chars().take(LOOKUP_QUERY_MAX_CHARS).collect();
    let (chapter, verses) = log_verses(answer.default.as_ref());
    tracing::info!(
        target: "presenter::bible::lookup",
        q = logged_query.as_str(),
        kind = answer.kind,
        book = answer.default.as_ref().map_or("-", |d| d.book_code.as_str()),
        chapter,
        verses = verses.as_str(),
        translation = answer.translation.as_str(),
        ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "bible lookup"
    );
    Ok(Json(answer))
}
