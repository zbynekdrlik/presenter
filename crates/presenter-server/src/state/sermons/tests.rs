//! #851 P3: sermon slides through the real composer and the commit path
//! (spec §7.4, §7.5, §13). SEB / KJV Hebrews 11:1-3 rows are seeded into the
//! in-memory repository; the NLT comes from a mock API (an external network
//! service, the one thing tests may mock).

use super::generate::stable_slide_id;
use super::*;
use crate::bible_remote::{NltClient, NLT_CODE};
use crate::live::LiveEvent;
use crate::state::AppState;
use crate::tracing_capture::{quoted, Capture};
use chrono::{Days, Utc};
use presenter_core::bible::sermon::{ItemEdits, SourceVerse, VerseEdits};
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{
    BiblePassage, BiblePreferences, BiblePresentationId, BibleReference, BibleTranslation,
};
use presenter_persistence::{NewSermon, SermonItemRecord, SermonRecord, SermonWrite};
use std::time::Duration;
use uuid::Uuid;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// SEB Heb 11:1 (the mockup's verse) and two made-up verses after it.
const SEB: [&str; 3] = [
    "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,",
    "Pre ňu dostali starší dobré svedectvo.",
    "Vierou poznávame, že svety vznikli Božím slovom.",
];
/// KJV Heb 11:1-3 (public domain), shortened at the end of verse 3.
const KJV: [&str; 3] = [
    "Now faith is the substance of things hoped for, the evidence of things not seen.",
    "For by it the elders obtained a good report.",
    "Through faith we understand that the worlds were framed by the word of God.",
];

async fn seed(state: &AppState, code: &str, language: &str, book: &str, texts: &[&str]) {
    let translation = BibleTranslation::new(code, code, language);
    let mut passages = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let verse = index as u16 + 1;
        let reference =
            BibleReference::new_with_code(book, "HEB", 58, 11, verse, verse).expect("reference");
        passages.push(BiblePassage::new(
            reference,
            translation.clone(),
            (*text).to_string(),
        ));
    }
    let batch = BibleIngestionBatch::new(translation, passages).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed translation");
}

async fn hebrews() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed(&state, "slk-seb", "sk", "Hebrejom", &SEB).await;
    seed(&state, "eng-kjv", "en", "Hebrews", &KJV).await;
    state
}

fn sermon(secondary: Option<&str>) -> SermonRecord {
    SermonRecord {
        id: BiblePresentationId::new(),
        title: "Kázeň".to_string(),
        service_date: local_today(),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: secondary.map(str::to_string),
        author_email: None,
        revision: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// An SEB Hebrews 11 item over `verses` (1-based, contiguous) with `edits`.
fn item(verses: std::ops::RangeInclusive<u16>, edits: &[(u16, &[u16])]) -> SermonItemRecord {
    let source = verses
        .clone()
        .map(|verse| SourceVerse {
            verse,
            text: SEB[usize::from(verse - 1)].to_string(),
        })
        .collect();
    let edits = ItemEdits(
        edits
            .iter()
            .map(|(verse, deleted)| {
                let verse_edits = VerseEdits {
                    deleted: deleted.to_vec(),
                    emphasized: Vec::new(),
                };
                (*verse, verse_edits)
            })
            .collect(),
    );
    SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: *verses.start(),
        verse_end: *verses.end(),
        source,
        edits,
        slide_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn set_limit(state: &AppState, limit: u32) {
    state
        .set_bible_preferences(BiblePreferences::default().with_character_limit(limit))
        .await
        .expect("preferences");
}

#[tokio::test]
async fn an_item_becomes_a_slide_with_its_computed_label() {
    let state = hebrews().await;
    let heb = item(1..=1, &[(1, &[7, 8, 9, 10, 11])]);
    let mut emphasized = heb.clone();
    emphasized.edits.0.entry(1).or_default().emphasized = vec![0];

    let generated = state
        .generate_sermon_slides(&sermon(None), &[emphasized.clone()])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [1]);
    let slide = &generated.slides[0];
    assert_eq!(slide.main.value(), "1. VIERA je zárukou toho, v čo dúfame");
    assert_eq!(slide.main_reference, "Hebrejom 11:1a (SEB)");
    assert_eq!(slide.secondary.value(), "");
    assert_eq!(slide.id, stable_slide_id(emphasized.id, 0));
    let metadata = slide.metadata.as_ref().expect("bible metadata");
    assert_eq!(
        (metadata.book_code.as_deref(), metadata.chapter),
        (Some("HEB"), 11)
    );
    assert!(generated.warnings.is_empty());
}

#[tokio::test]
async fn sermon_slides_obey_the_828_limits() {
    let state = hebrews().await;
    let whole = item(1..=3, &[]);
    set_limit(&state, 100).await;

    let main_only = state
        .generate_sermon_slides(&sermon(None), &[whole.clone()])
        .await
        .expect("main only");
    let with_kjv = state
        .generate_sermon_slides(&sermon(Some("eng-kjv")), &[whole.clone()])
        .await
        .expect("with KJV");
    set_limit(&state, 60).await;
    let narrow = state
        .generate_sermon_slides(&sermon(None), &[whole])
        .await
        .expect("narrow");

    // 2 + 3 SEB lines fit 100 chars; their KJV lines (126) do not: the longer decides.
    assert_eq!(main_only.slide_counts, [2]);
    assert_eq!(with_kjv.slide_counts, [3]);
    // A lone verse longer than the limit stays one whole slide.
    assert_eq!(narrow.slide_counts, [3]);
    assert_eq!(narrow.slides[0].main.value().chars().count(), 71);
}

#[tokio::test]
async fn the_secondary_line_has_the_full_kept_verses_and_its_own_book() {
    let state = hebrews().await;
    let cut = item(1..=3, &[(1, &[7, 8, 9, 10, 11]), (2, &[0, 1, 2, 3, 4, 5])]);

    let generated = state
        .generate_sermon_slides(&sermon(Some("eng-kjv")), &[cut])
        .await
        .expect("generate");

    assert_eq!(generated.slides.len(), 1);
    let slide = &generated.slides[0];
    assert_eq!(slide.main_reference, "Hebrejom 11:1a, 3 (SEB)");
    assert_eq!(
        slide.main.value(),
        "1. Viera je zárukou toho, v čo dúfame\n3. Vierou poznávame, že svety vznikli Božím slovom."
    );
    assert_eq!(slide.secondary_reference, "Hebrews 11:1, 3 (KJV)");
    assert_eq!(
        slide.secondary.value(),
        format!("1. {}\n3. {}", KJV[0], KJV[2])
    );
}

#[tokio::test]
async fn slide_ids_are_stable_across_a_regeneration() {
    let state = hebrews().await;
    let items = [item(1..=1, &[]), item(2..=3, &[])];

    let first = state
        .generate_sermon_slides(&sermon(None), &items)
        .await
        .expect("first");
    let second = state
        .generate_sermon_slides(&sermon(None), &items)
        .await
        .expect("second");

    let first_ids: Vec<_> = first.slides.iter().map(|slide| slide.id).collect();
    let second_ids: Vec<_> = second.slides.iter().map(|slide| slide.id).collect();
    assert_eq!(first_ids, second_ids);
    assert_eq!(
        first_ids,
        [
            stable_slide_id(items[0].id, 0),
            stable_slide_id(items[1].id, 0)
        ]
    );
}

#[tokio::test]
async fn an_unavailable_nlt_gives_slides_without_the_line_and_a_warning() {
    let mut state = hebrews().await;
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(5)));

    let generated = state
        .generate_sermon_slides(&sermon(Some(NLT_CODE)), &[item(1..=2, &[])])
        .await
        .expect("a failed secondary never fails the write");

    assert_eq!(generated.slide_counts, [1]);
    assert_eq!(generated.slides[0].secondary.value(), "");
    assert_eq!(generated.slides[0].secondary_reference, "");
    assert_eq!(generated.warnings.len(), 1, "{:?}", generated.warnings);
    assert!(generated.warnings[0].ends_with("sekundárny preklad vynechaný"));
}

#[tokio::test]
async fn an_item_with_every_token_struck_has_no_slides() {
    let state = hebrews().await;
    let struck = item(2..=2, &[(2, &[0, 1, 2, 3, 4, 5])]);

    let generated = state
        .generate_sermon_slides(&sermon(None), &[struck, item(3..=3, &[])])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [0, 1]);
    assert_eq!(generated.slides.len(), 1);
}

#[tokio::test]
async fn an_item_of_a_removed_translation_still_renders_from_its_snapshot() {
    let state = hebrews().await;
    let mut gone = item(1..=1, &[]);
    gone.translation_code = "slk-gone".to_string();

    let generated = state
        .generate_sermon_slides(&sermon(None), &[gone])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [1]);
    assert_eq!(generated.slides[0].main_reference, "Hebrejom 11:1 (GONE)");
}

/// A stored sermon dated `date` holding `items` (revision 1, no slides yet).
async fn stored_sermon(
    state: &AppState,
    date: chrono::NaiveDate,
    items: Vec<SermonItemRecord>,
) -> BiblePresentationId {
    let new = NewSermon {
        title: "Kázeň".to_string(),
        service_date: date,
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: None,
        author_email: None,
    };
    let created = state
        .repository()
        .create_sermon(&new)
        .await
        .expect("create");
    let write = SermonWrite {
        expected_revision: 0,
        title: "Kázeň".to_string(),
        service_date: date,
        secondary_translation_code: None,
        items,
        slides: Vec::new(),
    };
    state
        .repository()
        .save_sermon_change(created.id, &write)
        .await
        .expect("seed write");
    created.id
}

#[tokio::test]
async fn a_new_character_limit_regenerates_future_sermons_only() {
    let state = hebrews().await;
    let today = local_today();
    let past_date = today.checked_sub_days(Days::new(7)).expect("a week ago");
    let future = stored_sermon(&state, today, vec![item(1..=3, &[])]).await;
    let past = stored_sermon(&state, past_date, vec![item(1..=3, &[])]).await;
    let mut events = state.live_hub().subscribe();
    let capture = Capture::install();

    set_limit(&state, 60).await;

    let fetch = |id| {
        let state = state.clone();
        async move {
            state
                .repository()
                .fetch_sermon(id)
                .await
                .expect("fetch")
                .expect("sermon")
        }
    };
    let future_now = fetch(future).await;
    assert_eq!(future_now.sermon.revision, 2);
    assert_eq!(future_now.items[0].slide_count, 3);
    let future_slides = state
        .bible_presentation_detail(future)
        .await
        .expect("detail")
        .expect("presentation")
        .slides;
    assert_eq!(future_slides.len(), 3);
    assert_eq!(fetch(past).await.sermon.revision, 1);
    let mut changed = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LiveEvent::BibleSlidesChanged { presentation_id } = event {
            changed.push(presentation_id);
        }
    }
    assert_eq!(changed, [future.to_string()]);
    let writes: Vec<_> = capture
        .events("presenter::bible::sermon")
        .into_iter()
        .filter(|line| line.field("message") == Some("sermon write"))
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].level, tracing::Level::INFO);
    assert_eq!(writes[0].field("op"), Some(quoted("regenerate").as_str()));
    assert_eq!(
        writes[0].field("client_ip"),
        Some(quoted("system").as_str())
    );
    assert_eq!(writes[0].field("author"), Some(quoted("-").as_str()));
    assert_eq!(writes[0].field("slides"), Some("3"));

    set_limit(&state, 60).await;
    assert_eq!(
        fetch(future).await.sermon.revision,
        2,
        "same limit, no pass"
    );
}
