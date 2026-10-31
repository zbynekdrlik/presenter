//! #851 P4a–P4d: the sermon API over HTTP (spec §11, §12): status codes, the
//! JSON shape (camelCase) and the author / client-IP wiring. The write
//! semantics are pinned at the state level (`state/sermons/tests.rs`).

use crate::router::build_router;
use crate::state::sermons::local_today;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse, VerseEdits};
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use presenter_persistence::{NewSermon, SermonItemRecord, SermonWrite};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;
use uuid::Uuid;

const HEB_11_1: &str = "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,";

/// An in-memory state with SEB Heb 11:1.
async fn seeded_state() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    let reference =
        BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, 1, 1).expect("reference");
    let passage = BiblePassage::new(reference, seb.clone(), HEB_11_1.to_string());
    let batch = BibleIngestionBatch::new(seb, vec![passage]).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
    state
}

async fn app() -> Router {
    build_router(seeded_state().await)
}

/// The app with one sermon dated today holding Heb 11:1 cut to `1a` with
/// "Viera" emphasized, its slides generated; returns the sermon id.
async fn app_with_sermon() -> (Router, String) {
    let state = seeded_state().await;
    let new = NewSermon {
        title: "Kázeň".to_string(),
        service_date: local_today(),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: None,
        author_email: None,
    };
    let created = state
        .repository()
        .create_sermon(&new)
        .await
        .expect("create");
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: vec![0],
        },
    );
    let item = SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: 1,
        verse_end: 1,
        source: vec![SourceVerse {
            verse: 1,
            text: HEB_11_1.to_string(),
        }],
        edits,
        slide_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let write = SermonWrite {
        expected_revision: 0,
        title: "Kázeň".to_string(),
        service_date: local_today(),
        secondary_translation_code: None,
        items: vec![item],
        slides: Vec::new(),
    };
    state
        .repository()
        .save_sermon_change(created.id, &write)
        .await
        .expect("seed write");
    state.regenerate_future_sermons(local_today()).await;
    (build_router(state), created.id.to_string())
}

/// Send `method uri` (JSON `body`, extra `headers`); the status and the JSON
/// body (`Value::Null` for an empty one).
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn the_reads_answer_camel_case_json() {
    let (app, id) = app_with_sermon().await;

    let (status, list) = call(&app, "GET", "/bible/sermons", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["today"], json!(local_today()));
    assert!(list["defaultServiceDate"].is_string());
    assert_eq!(list["sermons"][0]["id"], id.as_str());
    assert_eq!(list["sermons"][0]["itemCount"], 1);
    assert_eq!(list["sermons"][0]["slideCount"], 1);
    let (status, sermon) = call(&app, "GET", &format!("/bible/sermons/{id}"), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sermon["revision"], 2);
    assert_eq!(sermon["defaultTranslation"], "slk-seb");
    assert_eq!(sermon["secondaryTranslation"], Value::Null);
    let item = &sermon["items"][0];
    assert_eq!(item["label"], "Hebrejom 11:1a (SEB)");
    assert_eq!(item["book"], "Hebrejom");
    assert_eq!(item["source"][0]["text"], HEB_11_1);
    assert_eq!(item["edits"]["1"]["deleted"], json!([7, 8, 9, 10, 11]));
    assert_eq!(
        item["renderedText"],
        "1. VIERA je zárukou toho, v čo dúfame"
    );
    assert_eq!(item["sourceChanged"], false);
    let slide = &sermon["slides"][0];
    assert_eq!(slide["mainText"], "1. VIERA je zárukou toho, v čo dúfame");
    assert_eq!(slide["mainReference"], "Hebrejom 11:1a (SEB)");
    assert_eq!(slide["itemId"], item["id"]);
    assert_eq!(item["slideIds"][0], slide["id"]);
    let (status, today) = call(&app, "GET", "/bible/sermons/today", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(today[0]["id"], id.as_str());
}

#[tokio::test]
async fn an_unknown_sermon_is_404_and_a_bad_id_400() {
    let app = app().await;

    let unknown = format!("/bible/sermons/{}", Uuid::new_v4());
    let (status, _) = call(&app, "GET", &unknown, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/bible/sermons/not-a-uuid", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, today) = call(&app, "GET", "/bible/sermons/today", None, &[]).await;
    assert_eq!((status, today), (StatusCode::OK, json!([])));
}

#[tokio::test]
async fn identity_trusts_the_access_email_only_from_the_tunnel() {
    let app = app().await;
    let header = [("cf-access-authenticated-user-email", "preacher@example.org")];

    let (status, tunnel) = call(&app, "GET", "/bible/study/identity", None, &header).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tunnel, json!({ "email": "preacher@example.org" }));
    let mut lan = Request::builder()
        .uri("/bible/study/identity")
        .header(header[0].0, header[0].1)
        .body(Body::empty())
        .expect("request");
    let peer: SocketAddr = "192.0.2.7:40000".parse().expect("peer");
    lan.extensions_mut().insert(ConnectInfo(peer));
    let response = app.clone().oneshot(lan).await.expect("response");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(body, json!({ "email": null }));
}

// ---- #851 P4b: create, delete, the slide guard, `sermonDate` -------------------

#[tokio::test]
async fn create_records_the_author_and_delete_is_204_then_404() {
    let app = app().await;
    let headers = [
        ("cf-access-authenticated-user-email", "Preacher@Example.org"),
        ("x-forwarded-for", "203.0.113.9"),
    ];
    let body = json!({ "serviceDate": local_today(), "defaultTranslation": "slk-seb",
        "secondaryTranslation": null });

    let (status, created) = call(&app, "POST", "/bible/sermons", Some(body), &headers).await;

    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["authorEmail"], "preacher@example.org");
    assert_eq!(created["revision"], 0);
    assert_eq!(created["items"], json!([]));
    let uri = format!("/bible/sermons/{}", created["id"].as_str().expect("id"));
    let (status, _) = call(&app, "DELETE", &uri, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", &uri, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let nlt = json!({ "defaultTranslation": "eng-nlt" });
    let (status, refused) = call(&app, "POST", "/bible/sermons", Some(nlt), &[]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused["message"],
        "NLT sa do kázne neukladá — vyber iný preklad"
    );
}

#[tokio::test]
async fn a_sermons_slides_refuse_the_presentation_slide_routes() {
    let (app, id) = app_with_sermon().await;
    let (_, sermon) = call(&app, "GET", &format!("/bible/sermons/{id}"), None, &[]).await;
    let slide_id = sermon["slides"][0]["id"]
        .as_str()
        .expect("slide id")
        .to_string();
    let slide_uri = format!("/bible/presentations/{id}/slides/{slide_id}");
    let refused = [
        (
            "POST",
            format!("/bible/presentations/{id}/append"),
            Some(json!({ "slides": [{ "bibleMain": "x", "bibleTranslation": "" }] })),
        ),
        (
            "PATCH",
            slide_uri.clone(),
            Some(json!({ "bibleMain": "x", "bibleTranslation": "",
                "bibleMainReference": "", "bibleTranslationReference": "" })),
        ),
        ("DELETE", slide_uri, None),
        (
            "POST",
            format!("/bible/presentations/{id}/slides/reorder"),
            Some(json!({ "slideIds": [slide_id] })),
        ),
    ];

    for (method, uri, body) in refused {
        let (status, answer) = call(&app, method, &uri, body, &[]).await;
        assert_eq!(status, StatusCode::CONFLICT, "{method} {uri}: {answer}");
        assert_eq!(
            answer["message"],
            "Slajdy kázne sa generujú z jej položiek — upravuje sa v /ui/study"
        );
    }
    let rename = json!({ "name": "Premenovaná" });
    let uri = format!("/bible/presentations/{id}");
    let (status, _) = call(&app, "PATCH", &uri, Some(rename), &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "rename stays allowed");
    let (_, list) = call(&app, "GET", "/bible/presentations", None, &[]).await;
    assert_eq!(list[0]["name"], "Premenovaná");
    assert_eq!(list[0]["sermonDate"], json!(local_today()));
    let (_, detail) = call(&app, "GET", &uri, None, &[]).await;
    assert_eq!(detail["sermonDate"], json!(local_today()));
}

// ---- #851 P4c: add and edit items ----------------------------------------------

async fn new_sermon(app: &Router) -> Value {
    let body = json!({ "serviceDate": local_today(), "defaultTranslation": "slk-seb" });
    let (status, sermon) = call(app, "POST", "/bible/sermons", Some(body), &[]).await;
    assert_eq!(status, StatusCode::OK, "{sermon}");
    sermon
}

#[tokio::test]
async fn add_and_edit_an_item_over_http() {
    let app = app().await;
    let id = new_sermon(&app).await["id"]
        .as_str()
        .expect("id")
        .to_string();
    let items = format!("/bible/sermons/{id}/items");
    let item = json!({ "expectedRevision": 0, "translation": "slk-seb", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });

    let (status, added) = call(&app, "POST", &items, Some(item), &[]).await;
    assert_eq!(status, StatusCode::OK, "{added}");
    assert_eq!(added["items"][0]["label"], "Hebrejom 11:1 (SEB)");
    let item_id = added["items"][0]["id"]
        .as_str()
        .expect("item id")
        .to_string();
    let edits = json!({ "expectedRevision": 1,
        "verses": { "1": { "deleted": [7, 8, 9, 10, 11], "emphasized": [0] } } });
    let uri = format!("/bible/sermons/{id}/items/{item_id}/edits");
    let (status, edited) = call(&app, "PUT", &uri, Some(edits), &[]).await;

    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["items"][0]["label"], "Hebrejom 11:1a (SEB)");
    assert_eq!(
        edited["slides"][0]["mainText"],
        "1. VIERA je zárukou toho, v čo dúfame"
    );
}

#[tokio::test]
async fn item_refusals_are_422_409_and_404() {
    let app = app().await;
    let id = new_sermon(&app).await["id"]
        .as_str()
        .expect("id")
        .to_string();
    let items = format!("/bible/sermons/{id}/items");
    let nlt = json!({ "expectedRevision": 0, "translation": "eng-nlt", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });
    let stale = json!({ "expectedRevision": 7, "translation": "slk-seb", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });

    let (status, body) = call(&app, "POST", &items, Some(nlt), &[]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["message"],
        "NLT sa do kázne neukladá — vyber iný preklad"
    );
    let (status, body) = call(&app, "POST", &items, Some(stale), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], "sermon changed on another device");
    let edits = json!({ "expectedRevision": 0, "verses": {} });
    let missing = format!("/bible/sermons/{id}/items/{}/edits", Uuid::new_v4());
    let (status, _) = call(&app, "PUT", &missing, Some(edits), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
