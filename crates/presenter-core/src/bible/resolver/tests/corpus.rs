//! The 100-case corpus (#850 spec §2 S1, §10) against slk-seb, the main
//! translation of SNV and dev. A row's `alternatives` are books a picker should
//! offer: those that fit the structure must open `alternatives` in corpus order;
//! those that do not (2 Ján has no chapter 3) must be parse candidates that never fit.

use serde::Deserialize;

use super::seb;
use crate::bible::resolver::versification::bind;
use crate::bible::resolver::{
    lookup, parse, resolve, segment_label, BookRef, MatchKind, ParsedQuery, Passage, Resolution,
    Segment, Versification,
};

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<CorpusCase>,
}

#[derive(Deserialize)]
struct CorpusCase {
    id: u32,
    input: String,
    outcome: String,
    expected: Option<Expected>,
    #[serde(default)]
    alternatives: Vec<String>,
    #[serde(default)]
    hint: Option<String>,
    #[serde(default)]
    verses: Vec<u16>,
    #[serde(default)]
    end_chapter: Option<u16>,
    #[serde(default, rename = "match")]
    match_kind: Option<String>,
}

#[derive(Deserialize)]
struct Expected {
    book: Option<String>,
    chapter: Option<u16>,
    verse_start: Option<u16>,
    verse_end: Option<u16>,
}

fn corpus() -> Vec<CorpusCase> {
    let corpus: Corpus =
        serde_json::from_str(include_str!("../testdata/corpus.json")).expect("corpus parses");
    corpus.cases
}

fn codes(passages: &[Passage]) -> Vec<&str> {
    passages.iter().map(|p| p.book.code.as_str()).collect()
}

/// A list = one segment per verse; a cross-chapter range = to the end of its
/// first chapter, the whole middle chapters, then from verse 1.
fn expected_segments(case: &CorpusCase, structure: &Versification) -> Vec<Segment> {
    let Some(expected) = &case.expected else {
        return Vec::new();
    };
    let chapter = expected.chapter.unwrap_or(0);
    match case.outcome.as_str() {
        "list" => case
            .verses
            .iter()
            .map(|&v| Segment::verses(chapter, v, v))
            .collect(),
        "cross_chapter" => {
            let book = expected
                .book
                .as_deref()
                .and_then(|code| structure.book(code));
            let first = book.and_then(|b| b.verse_count(chapter)).unwrap_or(0);
            let end = case.end_chapter.unwrap_or(chapter);
            let mut segments = vec![Segment::verses(
                chapter,
                expected.verse_start.unwrap_or(1),
                first,
            )];
            segments.extend((chapter + 1..end).map(Segment::whole));
            segments.push(Segment::verses(end, 1, expected.verse_end.unwrap_or(1)));
            segments
        }
        _ => vec![Segment {
            chapter,
            verse_start: expected.verse_start,
            verse_end: expected.verse_end,
        }],
    }
}

fn alternative_problems(
    case: &CorpusCase,
    parsed: &ParsedQuery,
    got: &[&str],
    structure: &Versification,
) -> Vec<String> {
    let mut problems = Vec::new();
    let fitting: Vec<&str> = case
        .alternatives
        .iter()
        .map(String::as_str)
        .filter(|code| got.contains(code))
        .collect();
    if got.get(..fitting.len()) != Some(fitting.as_slice()) {
        problems.push(format!(
            "alternatives {got:?} do not start with {fitting:?}"
        ));
    }
    for code in &case.alternatives {
        if !parsed.candidates.iter().any(|c| c.code == *code) {
            problems.push(format!("{code} is not a parse candidate"));
        }
        let fits = parsed
            .spec
            .as_ref()
            .zip(structure.book(code))
            .is_some_and(|(spec, book)| bind(spec, book).is_some());
        if fits && !got.contains(&code.as_str()) {
            problems.push(format!("{code} fits but is missing from the alternatives"));
        }
    }
    problems
}

fn book_problem(case: &CorpusCase, code: &str) -> Option<String> {
    let want = case.expected.as_ref().and_then(|e| e.book.as_deref());
    (Some(code) != want).then(|| format!("book {code} != {want:?}"))
}

fn passage_problems(
    case: &CorpusCase,
    passage: &Passage,
    structure: &Versification,
) -> Vec<String> {
    let mut problems: Vec<String> = book_problem(case, &passage.book.code).into_iter().collect();
    let want = expected_segments(case, structure);
    if passage.segments != want {
        problems.push(format!("segments {:?} != {want:?}", passage.segments));
    }
    problems
}

fn out_of_range_problems(
    case: &CorpusCase,
    book: &BookRef,
    requested: &Segment,
    hint: &str,
    structure: &Versification,
) -> Vec<String> {
    let want = expected_segments(case, structure);
    let mut problems: Vec<String> = book_problem(case, &book.code).into_iter().collect();
    if want.first() != Some(requested) {
        problems.push(format!("requested {requested:?} != {want:?}"));
    }
    if Some(hint) != case.hint.as_deref() {
        problems.push(format!("hint {hint:?} != {:?}", case.hint));
    }
    problems
}

fn resolution_problems(
    case: &CorpusCase,
    parsed: &ParsedQuery,
    resolution: &Resolution,
    structure: &Versification,
) -> Vec<String> {
    let whole_chapter_book_only =
        case.outcome == "book_only" && case.expected.as_ref().and_then(|e| e.chapter).is_some();
    match (case.outcome.as_str(), resolution) {
        (
            "resolved" | "list" | "cross_chapter" | "book_only",
            Resolution::Passage {
                passage,
                alternatives,
            },
        )
        | (
            "ambiguous",
            Resolution::Ambiguous {
                default: passage,
                alternatives,
            },
        ) if case.outcome != "book_only" || whole_chapter_book_only => {
            let mut problems = passage_problems(case, passage, structure);
            problems.extend(alternative_problems(
                case,
                parsed,
                &codes(alternatives),
                structure,
            ));
            problems
        }
        (
            "out_of_range",
            Resolution::OutOfRange {
                book,
                requested,
                hint,
                ..
            },
        ) => out_of_range_problems(case, book, requested, hint, structure),
        ("book_only", Resolution::BookOnly { book, alternatives }) => {
            let got: Vec<&str> = alternatives.iter().map(|b| b.code.as_str()).collect();
            let mut problems: Vec<String> = book_problem(case, &book.code).into_iter().collect();
            problems.extend(alternative_problems(case, parsed, &got, structure));
            problems
        }
        ("incomplete", Resolution::Incomplete { ordinal, books })
            if *ordinal == 1
                && !books.is_empty()
                && books.iter().all(|b| b.code.starts_with('1')) =>
        {
            Vec::new()
        }
        ("no_book", Resolution::NoBook { segments, hint })
            if *segments == expected_segments(case, structure) && *hint == case.hint =>
        {
            Vec::new()
        }
        (outcome, other) => vec![format!("outcome {outcome}, got {other:?}")],
    }
}

fn case_problems(case: &CorpusCase, structure: &Versification) -> Vec<String> {
    let parsed = parse(&case.input);
    let resolution = resolve(&parsed, structure);
    let mut problems = resolution_problems(case, &parsed, &resolution, structure);
    let first_kind = parsed.candidates.first().map(|c| c.kind);
    if case.match_kind.as_deref() == Some("fuzzy") && first_kind != Some(MatchKind::Fuzzy) {
        problems.push(format!("not a fuzzy match: {first_kind:?}"));
    }
    problems
        .into_iter()
        .map(|problem| format!("#{} {:?}: {problem}", case.id, case.input))
        .collect()
}

#[test]
fn corpus_100_of_100_on_slk_seb() {
    let structure = seb();
    let cases = corpus();
    assert_eq!(cases.len(), 100);
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|case| case_problems(case, &structure))
        .collect();
    assert!(
        failures.is_empty(),
        "{} corpus problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_17_incident_spellings_of_hebrews_11_1_resolve_to_heb() {
    let structure = seb();
    for case in corpus().iter().take(17) {
        let book = match lookup(&case.input, &structure) {
            Resolution::Passage { passage, .. } => passage.book.code,
            Resolution::OutOfRange { book, .. } => book.code,
            other => panic!("{:?} → {other:?}", case.input),
        };
        assert_eq!(book, "HEB", "{:?}", case.input);
    }
}

#[test]
fn zid_11_1_shows_the_stored_name_and_the_matched_form() {
    let Resolution::Passage {
        passage,
        alternatives,
    } = lookup("zid 11 1", &seb())
    else {
        panic!("zid 11 1 must be a passage");
    };
    assert_eq!(
        (passage.book.name.as_str(), passage.book.number),
        ("Hebrejom", 58)
    );
    assert_eq!(passage.matched_form, "Žid");
    assert_eq!(passage.segments, [Segment::verses(11, 1, 1)]);
    assert!(alternatives.is_empty());
    assert_eq!(
        segment_label(&passage.book.name, &passage.segments[0]),
        "Hebrejom 11:1"
    );
}
