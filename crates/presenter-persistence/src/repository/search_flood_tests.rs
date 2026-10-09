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

/// #833 review: the slide phase's LIMIT counted slide ROWS. Songs whose TITLE
/// holds every token match on all their slides, and those rows (or a second
/// slide of a song already emitted) filled the window, so a lyric match in
/// another song was cut off.
#[tokio::test]
async fn many_slides_of_title_matches_do_not_crowd_out_a_lyric_match() {
    let repo = repo().await;
    for title in ["Víťazím A", "Víťazím B"] {
        let slides = (0..30).map(|p| slide(p, "iný text")).collect();
        let presentation = Presentation::new(title, slides).unwrap();
        let library = Library::new(format!("Knižnica {title}"), vec![presentation]).unwrap();
        repo.upsert_library(&library).await.unwrap();
    }
    let slides = (0..6)
        .map(|p| {
            slide(
                p,
                if p == 5 {
                    "už víťazím v Ňom"
                } else {
                    "verš"
                },
            )
        })
        .collect();
    let target = Presentation::new("Pieseň Z", slides).unwrap();
    repo.upsert_library(&Library::new("Zbor", vec![target]).unwrap())
        .await
        .unwrap();

    let results = repo.search_presenter("vitazim", 5).await.unwrap();

    assert!(
        names(&results).iter().any(|n| n == "Pieseň Z"),
        "the lyric match must not be crowded out by title matches, got {:?}",
        names(&results)
    );
}

/// #833 review: a very long query (a pasted verse) cost 4 LIKEs per token on
/// every slide. Only the first `MAX_SEARCH_TOKENS` words are matched, so a
/// long paste still finds the song its first words come from.
#[tokio::test]
async fn a_long_pasted_query_matches_on_its_first_words() {
    let repo = repo().await;
    let target = Presentation::new(
        "Pieseň L",
        vec![slide(0, "jeden dva tri styri pat sest sedem osem devat")],
    )
    .unwrap();
    repo.upsert_library(&Library::new("Zbor", vec![target]).unwrap())
        .await
        .unwrap();

    let results = repo
        .search_presenter(
            "jeden dva tri styri pat sest sedem osem xenon yttrium zirkon wolfram",
            10,
        )
        .await
        .unwrap();

    assert!(
        names(&results).iter().any(|n| n == "Pieseň L"),
        "a long query must match on its first words, got {:?}",
        names(&results)
    );
}
