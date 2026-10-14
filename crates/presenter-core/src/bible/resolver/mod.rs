//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5): [`parse`] turns text into book candidates and a
//! chapter/verse part, [`resolve`] binds them to the MAIN translation's
//! structure, [`lookup`] does both. The name table is generated
//! (`scripts/dev/gen_bible_book_names.py`); the importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
mod grammar;
mod hint;
#[rustfmt::skip]
pub mod names_table;
mod rank;
mod types;
mod versification;

pub use fold::{fold_book_text, FoldedBook};
pub use hint::{
    chapters_hint, counted, segment_label, verses_hint, MISSING_BOOK_HINT, NO_BOOK_HINT,
};
pub use types::{
    BookCandidate, BookRef, MatchKind, ParsedQuery, Passage, RawSpec, Resolution, Segment, Tier,
};
pub use versification::{BookStructure, Step, Versification};

/// Pure text analysis: book candidates (best first), chapter/verse part, leftover words.
pub fn parse(input: &str) -> ParsedQuery {
    let split = grammar::split_input(input);
    let folded = fold_book_text(&split.book_text);
    let candidates = match rank::no_fold_candidate(&split.book_text) {
        Some(candidate) => vec![candidate],
        None => rank::rank(&folded, &[]),
    };
    ParsedQuery {
        input: input.to_string(),
        book_text: split.book_text,
        ordinal: folded.ordinal,
        key: folded.key,
        bare_ordinal: split.bare_ordinal,
        candidates,
        spec: split.spec,
        leftover: split.leftover,
    }
}

/// `parse` + `resolve`.
pub fn lookup(input: &str, structure: &Versification) -> Resolution {
    resolve(&parse(input), structure)
}

/// The best book for a name alone, no structure (the server choke point, §5.8).
pub fn resolve_book_name(name: &str) -> Option<BookCandidate> {
    parse(name).candidates.into_iter().next()
}

/// `KJV` → `eng-kjv`, `seb` → `slk-seb`: an exact installed code (any case),
/// else the one installed code whose part after the last `-` matches.
pub fn resolve_translation_code(input: &str, installed: &[&str]) -> Option<String> {
    let wanted = input.trim();
    if wanted.is_empty() {
        return None;
    }
    if let Some(code) = installed
        .iter()
        .find(|code| code.eq_ignore_ascii_case(wanted))
    {
        return Some((*code).to_string());
    }
    let mut by_short = installed.iter().filter(|code| {
        code.rsplit('-')
            .next()
            .is_some_and(|short| short.eq_ignore_ascii_case(wanted))
    });
    let first = by_short.next()?;
    by_short.next().is_none().then(|| (*first).to_string())
}

/// Bind a parsed input to the MAIN translation's structure (spec §5.6, §5.7).
pub fn resolve(query: &ParsedQuery, structure: &Versification) -> Resolution {
    let typed_ordinal = if query.key.is_empty() {
        query.ordinal
    } else {
        None
    };
    if let Some(ordinal) = query.bare_ordinal.or(typed_ordinal) {
        return incomplete(ordinal, structure);
    }
    if query.key.is_empty() && query.candidates.is_empty() {
        let hint = query.spec.as_ref().map(|_| NO_BOOK_HINT.to_string());
        return no_book(query, hint);
    }
    let books = candidates_in(query, structure);
    let Some(((best, best_book), others)) = books.split_first() else {
        let hint = (!query.candidates.is_empty()).then(|| MISSING_BOOK_HINT.to_string());
        return no_book(query, hint);
    };
    let Some(spec) = &query.spec else {
        return Resolution::BookOnly {
            book: book_ref(best_book),
            alternatives: others.iter().map(|(_, book)| book_ref(book)).collect(),
        };
    };
    let fits: Vec<(&BookCandidate, Passage)> = books
        .iter()
        .filter_map(|(candidate, book)| {
            let segments = versification::bind(spec, book)?;
            Some((candidate, passage(candidate, book, segments)))
        })
        .collect();
    let Some(((default, default_passage), rest)) = fits.split_first() else {
        let failure = versification::out_of_range(spec, best_book);
        let fix = (!failure.fix.is_empty()).then(|| passage(best, best_book, failure.fix));
        return Resolution::OutOfRange {
            book: book_ref(best_book),
            requested: failure.requested,
            hint: failure.hint,
            fix,
        };
    };
    let alternatives: Vec<Passage> = rest.iter().map(|(_, p)| p.clone()).collect();
    let ties = rest
        .iter()
        .any(|(other, _)| (other.tier, other.kind) <= (default.tier, default.kind));
    if ties {
        Resolution::Ambiguous {
            default: default_passage.clone(),
            alternatives,
        }
    } else {
        Resolution::Passage {
            passage: default_passage.clone(),
            alternatives,
        }
    }
}

/// Candidates re-ranked with the structure's stored names (a new translation's
/// own names work without regenerating the table), kept only when the main
/// translation has the book.
fn candidates_in<'s>(
    query: &ParsedQuery,
    structure: &'s Versification,
) -> Vec<(BookCandidate, &'s BookStructure)> {
    let folded = FoldedBook {
        ordinal: query.ordinal,
        key: query.key.clone(),
    };
    let ranked = match rank::no_fold_candidate(&query.book_text) {
        Some(candidate) => vec![candidate],
        None => rank::rank(&folded, structure.stored_names()),
    };
    ranked
        .into_iter()
        .filter_map(|candidate| {
            let book = structure.book(&candidate.code)?;
            Some((candidate, book))
        })
        .collect()
}

/// The main translation's books that have a name under `ordinal` (table forms
/// and its own stored names), in canonical order.
fn incomplete(ordinal: u8, structure: &Versification) -> Resolution {
    let numbered = rank::books_with_ordinal(ordinal, structure.stored_names());
    Resolution::Incomplete {
        ordinal,
        books: structure
            .books()
            .iter()
            .filter(|book| {
                rank::index_of(&book.code)
                    .is_some_and(|index| numbered.get(usize::from(index)) == Some(&true))
            })
            .map(book_ref)
            .collect(),
    }
}

fn no_book(query: &ParsedQuery, hint: Option<String>) -> Resolution {
    Resolution::NoBook {
        segments: query
            .spec
            .as_ref()
            .map(RawSpec::unbound_segments)
            .unwrap_or_default(),
        hint,
    }
}

fn book_ref(book: &BookStructure) -> BookRef {
    BookRef {
        code: book.code.clone(),
        number: book.number,
        name: book.name.clone(),
    }
}

fn passage(candidate: &BookCandidate, book: &BookStructure, segments: Vec<Segment>) -> Passage {
    Passage {
        book: book_ref(book),
        matched_form: candidate.matched_form.clone(),
        segments,
    }
}

#[cfg(test)]
mod tests;
