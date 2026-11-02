//! Study reference data (#851 D3a, D3b, spec §5.4, §5.5): Strong's dictionaries
//! and cross-references, parsed by `presenter-bible`, stored by the repository.

/// One imported file; `source_digest` decides whether a run replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudySource {
    pub code: String,
    pub name: String,
    pub license: String,
    pub source_digest: String,
}

/// One Strong's entry. `related` lists the Strong's ids the entry names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LexiconEntry {
    pub strongs: String,
    pub lemma: String,
    pub transliteration: String,
    pub pronunciation: String,
    pub definition: String,
    pub derivation: String,
    pub kjv_renderings: String,
    pub related: Vec<String>,
}

/// A verse pointing at a verse or a range, in the KJV's numbering. A range may
/// cross chapters and, 16 times in the source, books (2 John into 3 John).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossReference {
    pub from_book: i32,
    pub from_chapter: i32,
    pub from_verse: i32,
    pub to_book: i32,
    pub to_chapter: i32,
    pub to_verse: i32,
    pub to_book_end: i32,
    pub to_chapter_end: i32,
    pub to_verse_end: i32,
    pub votes: i32,
}
