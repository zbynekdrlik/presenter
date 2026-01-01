use serde::{Deserialize, Serialize};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::{LibraryId, PresentationId, SlideId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchResultKind {
    Library,
    Presentation,
    Slide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMatchField {
    LibraryName,
    PresentationName,
    MainText,
    TranslationText,
    StageText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub kind: SearchResultKind,
    pub library_id: LibraryId,
    pub library_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<PresentationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<SlideId>,
    pub match_field: SearchMatchField,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// Produce a lowercase, accent-free representation for search matching.
pub fn normalise_for_search(input: &str) -> String {
    let stripped: String = input.nfd().filter(|ch| !is_combining_mark(*ch)).collect();
    stripped.to_lowercase()
}

fn tokens_from_normalised(normalised: &str) -> Vec<String> {
    normalised
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Return whitespace separated search tokens for storage.
pub fn fold_query(input: &str) -> String {
    tokens_from_normalised(&normalise_for_search(input)).join(" ")
}

/// Produce token list suitable for query matching.
pub fn query_tokens(input: &str) -> Vec<String> {
    tokens_from_normalised(&normalise_for_search(input))
}
