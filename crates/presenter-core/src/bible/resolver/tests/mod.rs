//! Resolver tests (#850 spec §10). Fixtures in `../testdata/`: the fold cases
//! shared with the generator, the 100-case corpus and `/bible/books` snapshots.

mod api;
mod corpus;
mod grammar;
mod hint;
mod rank;
mod table;
mod versification;

use serde::Deserialize;

use super::{BookStructure, Versification};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookDto {
    book: String,
    code: String,
    number: u16,
    chapters: Vec<ChapterDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterDto {
    number: u16,
    verse_count: u16,
}

/// A `GET /bible/books` snapshot as the main translation's structure.
fn structure(json: &str) -> Versification {
    let books: Vec<BookDto> = serde_json::from_str(json).expect("structure fixture parses");
    Versification::new(
        books
            .into_iter()
            .map(|book| {
                let mut chapters = book.chapters;
                chapters.sort_by_key(|chapter| chapter.number);
                BookStructure {
                    code: book.code,
                    number: book.number,
                    name: book.book,
                    verse_counts: chapters.iter().map(|chapter| chapter.verse_count).collect(),
                }
            })
            .collect(),
    )
}

fn seb() -> Versification {
    structure(include_str!("../testdata/structure-slk-seb.json"))
}

fn kjv() -> Versification {
    structure(include_str!("../testdata/structure-eng-kjv.json"))
}

fn sevp() -> Versification {
    structure(include_str!("../testdata/structure-slk-sevp.json"))
}
