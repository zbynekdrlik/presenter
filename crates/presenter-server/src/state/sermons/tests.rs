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
        .generate_sermon_slides(&sermon(None), std::slice::from_ref(&whole))
        .await
        .expect("main only");
    let with_kjv = state
        .generate_sermon_slides(&sermon(Some("eng-kjv")), std::slice::from_ref(&whole))
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

// ---- #851 P4a: the sermon reads ---------------------------------------------

fn date(text: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

#[tokio::test]
async fn the_detail_names_items_and_owns_their_slides() {
    let state = hebrews().await;
    let cut = item(1..=1, &[(1, &[7, 8, 9, 10, 11])]);
    let id = stored_sermon(&state, local_today(), vec![cut, item(2..=3, &[])]).await;
    state.regenerate_future_sermons(local_today()).await;

    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");

    assert_eq!(detail.sermon.revision, 2);
    let labels: Vec<&str> = detail
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(labels, ["Hebrejom 11:1a (SEB)", "Hebrejom 11:2-3 (SEB)"]);
    assert_eq!(
        detail.items[0].rendered_text,
        "1. Viera je zárukou toho, v čo dúfame"
    );
    let owners: Vec<Uuid> = detail.slides.iter().map(|slide| slide.item_id).collect();
    assert_eq!(
        owners,
        [detail.items[0].record.id, detail.items[1].record.id]
    );
    assert_eq!(detail.items[0].slide_ids, [detail.slides[0].slide.id]);
    assert!(detail.items.iter().all(|item| !item.source_changed));
    assert!(detail.warnings.is_empty());
    let unknown = state
        .sermon_detail(BiblePresentationId::new())
        .await
        .expect("detail");
    assert!(unknown.is_none());
}

#[tokio::test]
async fn a_changed_database_text_flags_the_item_and_keeps_its_snapshot() {
    let state = hebrews().await;
    let id = stored_sermon(&state, local_today(), vec![item(1..=2, &[])]).await;

    seed(
        &state,
        "slk-seb",
        "sk",
        "Hebrejom",
        &[SEB[0], "Iné znenie druhého verša.", SEB[2]],
    )
    .await;

    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert!(detail.items[0].source_changed);
    assert_eq!(
        detail.items[0].record.source[1].text, SEB[1],
        "the snapshot stays"
    );
}

#[tokio::test]
async fn the_list_carries_the_server_dates_and_today_lists_todays_sermons() {
    let state = hebrews().await;
    let today = local_today();
    let tomorrow = today.checked_add_days(Days::new(1)).expect("tomorrow");
    let todays = stored_sermon(&state, today, Vec::new()).await;
    stored_sermon(&state, tomorrow, Vec::new()).await;

    let list = state.list_sermons(date("2026-10-14")).await.expect("list");
    let on: Vec<BiblePresentationId> = state
        .sermons_on(today)
        .await
        .expect("today")
        .into_iter()
        .map(|detail| detail.sermon.id)
        .collect();

    assert_eq!(list.today, date("2026-10-14"));
    assert_eq!(list.default_service_date, date("2026-10-18"));
    assert_eq!(list.sermons.len(), 2);
    assert_eq!(on, [todays]);
}

// ---- #851 P4b: create, delete, the slide guard -------------------------------

fn preacher() -> SermonOrigin {
    SermonOrigin {
        author: Some("preacher@example.org".to_string()),
        client_ip: "203.0.113.9".to_string(),
    }
}

fn refusal(err: &anyhow::Error) -> Option<String> {
    err.downcast_ref::<SermonRefusal>()
        .map(|refusal| refusal.0.clone())
}

/// A new SEB sermon for today with no secondary line.
async fn new_sermon(state: &AppState) -> SermonDetail {
    let draft = SermonDraft {
        service_date: Some(local_today()),
        default_translation: Some("slk-seb".to_string()),
        secondary_translation: Some(None),
        ..SermonDraft::default()
    };
    state
        .create_sermon(draft, local_today(), &preacher())
        .await
        .expect("create")
}

#[tokio::test]
async fn create_takes_the_server_dates_and_the_bible_preferences() {
    let state = hebrews().await;
    state
        .set_bible_preferences(
            BiblePreferences::default()
                .with_main_translation(Some("slk-seb".to_string()))
                .with_secondary_translation(Some("eng-kjv".to_string())),
        )
        .await
        .expect("preferences");

    let created = state
        .create_sermon(SermonDraft::default(), date("2026-10-14"), &preacher())
        .await
        .expect("create");

    let sermon = &created.sermon;
    assert_eq!(sermon.service_date, date("2026-10-18"));
    assert_eq!(sermon.title, "Kázeň 18. 10. 2026");
    assert_eq!(sermon.default_translation_code, "slk-seb");
    assert_eq!(
        sermon.secondary_translation_code.as_deref(),
        Some("eng-kjv")
    );
    assert_eq!(sermon.author_email.as_deref(), Some("preacher@example.org"));
    assert_eq!(sermon.revision, 0);
    let nlt = SermonDraft {
        default_translation: Some(NLT_CODE.to_string()),
        ..SermonDraft::default()
    };
    let err = state
        .create_sermon(nlt, date("2026-10-14"), &preacher())
        .await
        .expect_err("the NLT is never an item translation");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("NLT sa do kázne neukladá — vyber iný preklad")
    );
}

#[tokio::test]
async fn deleting_only_deletes_sermons_and_logs_with_the_author() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let plain = state
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain presentation");
    let capture = Capture::install();

    let err = state
        .delete_sermon(plain.id, &preacher())
        .await
        .expect_err("a plain presentation is no sermon");
    state.delete_sermon(id, &preacher()).await.expect("delete");

    assert!(matches!(
        err.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::NotFound(_))
    ));
    assert!(state.sermon_detail(id).await.expect("detail").is_none());
    let writes: Vec<_> = capture
        .events("presenter::bible::sermon")
        .into_iter()
        .filter(|line| line.field("message") == Some("sermon write"))
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].field("op"), Some(quoted("delete").as_str()));
    assert_eq!(
        writes[0].field("author"),
        Some(quoted("preacher@example.org").as_str())
    );
    assert_eq!(
        writes[0].field("client_ip"),
        Some(quoted("203.0.113.9").as_str())
    );
    assert_eq!(writes[0].field("sermon_id"), Some(id.to_string().as_str()));
}

#[tokio::test]
async fn sermon_slides_are_read_only_on_the_presentation_paths() {
    let state = hebrews().await;
    let id = stored_sermon(&state, local_today(), vec![item(1..=3, &[])]).await;
    state.regenerate_future_sermons(local_today()).await;
    let slides = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon")
        .slides;
    let slide_id = slides[0].slide.id;

    let refusals = [
        state
            .append_bible_presentation_slides(id, Vec::new())
            .await
            .map(|_| ()),
        state.delete_bible_slide(id, slide_id).await.map(|_| ()),
        state
            .reorder_bible_slides(id, vec![slide_id])
            .await
            .map(|_| ()),
        state
            .update_bible_slide(
                id,
                slide_id,
                "x".into(),
                "y".into(),
                String::new(),
                String::new(),
                None,
            )
            .await
            .map(|_| ()),
    ];

    for refused in refusals {
        let err = refused.expect_err("a sermon's slides are generated");
        assert!(matches!(
            err.downcast_ref::<presenter_persistence::RepositoryError>(),
            Some(presenter_persistence::RepositoryError::Conflict(
                super::guard::SERMON_SLIDES_REFUSAL
            ))
        ));
    }
    state
        .rename_bible_presentation(id, "Nový názov")
        .await
        .expect("rename stays allowed");
    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!(detail.sermon.title, "Nový názov");
    assert_eq!(detail.slides.len(), slides.len());
}

// ---- #851 P4c: add and edit items --------------------------------------------

fn range(translation: &str, book_code: &str, verses: std::ops::RangeInclusive<u16>) -> ItemRange {
    ItemRange {
        translation: translation.to_string(),
        book_code: book_code.to_string(),
        chapter: 11,
        verse_start: *verses.start(),
        verse_end: *verses.end(),
    }
}

#[tokio::test]
async fn an_added_item_snapshots_the_text_and_owns_its_slides() {
    let state = hebrews().await;
    let sermon = new_sermon(&state).await;

    let detail = state
        .add_sermon_item(
            sermon.sermon.id,
            0,
            None,
            &range("slk-seb", "HEB", 1..=3),
            &preacher(),
        )
        .await
        .expect("add");

    assert_eq!(detail.sermon.revision, 1);
    let item = &detail.items[0];
    let texts: Vec<&str> = item.record.source.iter().map(|v| v.text.as_str()).collect();
    assert_eq!(texts, SEB);
    assert_eq!(item.record.book_name, "Hebrejom");
    assert_eq!(item.label, "Hebrejom 11:1-3 (SEB)");
    assert!(!item.source_changed);
    assert_eq!(detail.slides.len(), 1);
    assert_eq!(detail.slides[0].item_id, item.record.id);
    assert_eq!(item.slide_ids, [detail.slides[0].slide.id]);
    assert!(item.rendered_text.starts_with("1. Viera je zárukou"));
}

#[tokio::test]
async fn item_refusals_name_the_problem_in_slovak() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let cases = [
        (
            range(NLT_CODE, "HEB", 1..=1),
            "NLT sa do kázne neukladá — vyber iný preklad",
        ),
        (
            range("slk-seb", "HEB", 1..=61),
            "Položka môže mať najviac 60 veršov",
        ),
        (
            range("slk-seb", "HEB", 1..=5),
            "HEB 11:1-5 nie je celé v preklade SEB",
        ),
        (range("slk-seb", "XYZ", 1..=1), "Neznáma kniha XYZ"),
        (
            range("slk-xyz", "HEB", 1..=1),
            "Preklad slk-xyz nie je nainštalovaný",
        ),
        (
            ItemRange {
                verse_start: 3,
                verse_end: 2,
                ..range("slk-seb", "HEB", 1..=1)
            },
            "Neplatný rozsah veršov",
        ),
    ];

    for (bad, message) in cases {
        let err = state
            .add_sermon_item(id, 0, None, &bad, &preacher())
            .await
            .expect_err("refused");
        assert_eq!(refusal(&err).as_deref(), Some(message), "{bad:?}");
    }
    let unchanged = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!(unchanged.sermon.revision, 0);
}

#[tokio::test]
async fn edits_are_normalized_and_a_bad_index_is_refused() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let added = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add");
    let item_id = added.items[0].record.id;
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![11, 10, 9, 8, 7],
            emphasized: vec![0, 7],
        },
    );

    let edited = state
        .set_sermon_item_edits(id, item_id, 1, &edits, &preacher())
        .await
        .expect("edit");

    let stored = &edited.items[0];
    assert_eq!(
        stored.record.edits.verse(1),
        Some(&VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: vec![0],
        })
    );
    assert_eq!(stored.label, "Hebrejom 11:1a (SEB)");
    assert_eq!(
        edited.slides[0].slide.main.value(),
        "1. VIERA je zárukou toho, v čo dúfame"
    );
    let mut bad = ItemEdits::default();
    bad.0.insert(
        1,
        VerseEdits {
            deleted: vec![40],
            emphasized: Vec::new(),
        },
    );
    let err = state
        .set_sermon_item_edits(id, item_id, 2, &bad, &preacher())
        .await
        .expect_err("index past the verse");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("Verš 1 nemá slovo s indexom 40")
    );
}

#[tokio::test]
async fn a_stale_revision_is_refused_before_anything_changes() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add");

    let err = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 2..=2), &preacher())
        .await
        .expect_err("stale");

    assert!(matches!(
        err.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::Conflict(
            presenter_persistence::SERMON_CONFLICT
        ))
    ));
    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!((detail.sermon.revision, detail.items.len()), (1, 1));
}

// ---- #851 P4d: item upkeep, reorder, update, regenerate ----------------------

/// A sermon with HEB 11:1 (verse 1 cut to `1a`) and HEB 11:2-3; revision 3.
async fn sermon_with_two_items(state: &AppState) -> SermonDetail {
    let id = new_sermon(state).await.sermon.id;
    let first = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add first");
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: Vec::new(),
        },
    );
    state
        .set_sermon_item_edits(id, first.items[0].record.id, 1, &edits, &preacher())
        .await
        .expect("edit");
    state
        .add_sermon_item(id, 2, None, &range("slk-seb", "HEB", 2..=3), &preacher())
        .await
        .expect("add second")
}

#[tokio::test]
async fn changing_the_translation_resnapshots_and_clears_the_edits() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let item_id = sermon.items[0].record.id;

    let changed = state
        .change_sermon_item_translation(sermon.sermon.id, item_id, 3, "eng-kjv", &preacher())
        .await
        .expect("translation");

    let item = &changed.items[0];
    assert_eq!(
        item.record.id, item_id,
        "the item keeps its id and its slide ids"
    );
    assert_eq!(item.record.translation_code, "eng-kjv");
    assert_eq!(item.record.book_name, "Hebrews");
    assert_eq!(item.record.edits, ItemEdits::default());
    assert_eq!(item.label, "Hebrews 11:1 (KJV)");
    assert_eq!(item.record.source[0].text, KJV[0]);
}

#[tokio::test]
async fn refreshing_keeps_the_edits_of_unchanged_verses_only() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let added = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=2), &preacher())
        .await
        .expect("add");
    let item_id = added.items[0].record.id;
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![11],
            emphasized: Vec::new(),
        },
    );
    edits.0.insert(
        2,
        VerseEdits {
            deleted: vec![0],
            emphasized: Vec::new(),
        },
    );
    state
        .set_sermon_item_edits(id, item_id, 1, &edits, &preacher())
        .await
        .expect("edit");
    seed(
        &state,
        "slk-seb",
        "sk",
        "Hebrejom",
        &[SEB[0], "Iné znenie druhého verša.", SEB[2]],
    )
    .await;

    let refreshed = state
        .refresh_sermon_item_source(id, item_id, 2, &preacher())
        .await
        .expect("refresh");

    let item = &refreshed.items[0];
    assert!(!item.source_changed);
    assert_eq!(item.record.source[1].text, "Iné znenie druhého verša.");
    assert_eq!(item.record.edits.0.keys().copied().collect::<Vec<_>>(), [1]);
}

#[tokio::test]
async fn deleting_and_reordering_items() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let id = sermon.sermon.id;
    let (first, second) = (sermon.items[0].record.id, sermon.items[1].record.id);

    let reordered = state
        .reorder_sermon_items(id, 3, &[second, first], &preacher())
        .await
        .expect("reorder");
    let order: Vec<_> = reordered.items.iter().map(|item| item.record.id).collect();
    assert_eq!(order, [second, first]);
    assert_eq!(
        reordered.slides[0].item_id, second,
        "slides follow the items"
    );

    let stale_set = state
        .reorder_sermon_items(id, 4, &[second], &preacher())
        .await
        .expect_err("a different item set is a stale view");
    assert!(matches!(
        stale_set.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::Conflict(_))
    ));

    let deleted = state
        .delete_sermon_item(id, second, 4, &preacher())
        .await
        .expect("delete item");
    let left: Vec<_> = deleted.items.iter().map(|item| item.record.id).collect();
    assert_eq!(left, [first]);
    assert_eq!(deleted.slides.len(), 1);
}

#[tokio::test]
async fn update_and_regenerate() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let id = sermon.sermon.id;
    let patch = SermonPatch {
        title: Some("  Viera  ".to_string()),
        service_date: Some(date("2026-10-25")),
        secondary_translation: Some(Some("eng-kjv".to_string())),
    };

    let updated = state
        .update_sermon(id, 3, patch, &preacher())
        .await
        .expect("update");

    assert_eq!(updated.sermon.title, "Viera");
    assert_eq!(updated.sermon.service_date, date("2026-10-25"));
    let presentation = state
        .bible_presentation_detail(id)
        .await
        .expect("detail")
        .expect("presentation");
    assert_eq!(presentation.name, "Viera");
    assert_eq!(
        presentation.slides[0].secondary_reference,
        "Hebrews 11:1 (KJV)"
    );
    let empty = SermonPatch {
        title: Some("  ".to_string()),
        ..SermonPatch::default()
    };
    let err = state
        .update_sermon(id, 4, empty, &preacher())
        .await
        .expect_err("empty title");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("Názov kázne nesmie byť prázdny")
    );
    let regenerated = state
        .regenerate_sermon(id, 4, &preacher())
        .await
        .expect("regenerate");
    assert_eq!(regenerated.sermon.revision, 5);
}
