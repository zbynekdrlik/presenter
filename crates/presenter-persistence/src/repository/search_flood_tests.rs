//! #833 regression: a lyric search with a short common word ("s tebou sa
//! neda") found NOTHING on PP. Every phase's SQL prefilter was looser than the
//! Rust all-tokens check: a library whose name held ANY token ("s", "sa") was
//! "matched", every presentation of it was OR'd into the presentation and
//! slide queries, and those unrelated rows filled the `.limit(remaining)`
//! window before the real lyric was ever read. Kept in its own file (`tests.rs`
//! is over the file-size cap).
use crate::Repository;
use presenter_core::{Library, Presentation, SearchResult, Slide, SlideContent, SlideText};

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn slide(position: u32, main: &str) -> Slide {
    Slide::new(
        position,
        SlideContent::new(
            SlideText::new(main).unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    )
}

/// 30 libraries whose names contain "s" and "sa" ("Salmy 01" …), each with a
/// song whose only slide sits at position 0 — more rows than the search limit,
/// all sorting before any later slide.
async fn seed_flood(repo: &Repository) {
    for n in 0..30 {
        let presentation =
            Presentation::new(format!("Pieseň {n:02}"), vec![slide(0, "iné slová")]).unwrap();
        let library = Library::new(format!("Salmy {n:02}"), vec![presentation]).unwrap();
        repo.upsert_library(&library).await.unwrap();
    }
}

fn names(results: &[SearchResult]) -> Vec<String> {
    results
        .iter()
        .filter_map(|r| r.presentation_name.clone())
        .collect()
}

#[tokio::test]
async fn lyric_with_short_words_is_found_past_a_flood_of_partial_library_matches() {
    let repo = repo().await;
    seed_flood(&repo).await;
    // The real song: a library and title WITHOUT "sa", the lyric on a later
    // slide (position 2), so only the slide-text phase can find it.
    let target = Presentation::new(
        "Víťazím",
        vec![
            slide(0, "Intro"),
            slide(1, "Verš jeden"),
            slide(2, "S Tebou sa nedá už prehrať"),
        ],
    )
    .unwrap();
    let library = Library::new("Zbor", vec![target]).unwrap();
    repo.upsert_library(&library).await.unwrap();

    let results = repo.search_presenter("s tebou sa neda", 10).await.unwrap();

    assert!(
        names(&results).iter().any(|n| n == "Víťazím"),
        "the song singing \"S Tebou sa nedá už prehrať\" must be found, got {:?}",
        names(&results)
    );
}

#[tokio::test]
async fn a_title_word_plus_a_lyric_word_finds_the_song() {
    let repo = repo().await;
    seed_flood(&repo).await;
    // "vitazim" is only in the TITLE, "prehrat" only in the LYRIC: the slide
    // phase must accept a token found in the presentation name, as the Rust
    // check (library + presentation + slide texts) already does.
    let target = Presentation::new(
        "Víťazím s Tebou",
        vec![slide(0, "Intro"), slide(1, "už sa nedá prehrať")],
    )
    .unwrap();
    let library = Library::new("Zbor", vec![target]).unwrap();
    repo.upsert_library(&library).await.unwrap();

    let results = repo.search_presenter("vitazim prehrat", 10).await.unwrap();

    assert!(
        names(&results).iter().any(|n| n == "Víťazím s Tebou"),
        "a title word plus a lyric word must find the song, got {:?}",
        names(&results)
    );
}

#[tokio::test]
async fn a_library_word_plus_a_lyric_word_still_finds_the_song() {
    let repo = repo().await;
    seed_flood(&repo).await;
    let target = Presentation::new(
        "Pieseň X",
        vec![slide(0, "Intro"), slide(1, "Spasenia skalou mojou")],
    )
    .unwrap();
    let library = Library::new("HERO", vec![target]).unwrap();
    repo.upsert_library(&library).await.unwrap();

    let results = repo.search_presenter("hero spasenia", 10).await.unwrap();

    assert!(
        names(&results).iter().any(|n| n == "Pieseň X"),
        "a library word plus a lyric word must find the song, got {:?}",
        names(&results)
    );
}
