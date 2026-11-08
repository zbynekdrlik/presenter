//! #915: the bookmark API over HTTP — status codes, the camelCase shape and
//! whose list a request reaches. The rules are pinned in
//! `state/study_bookmarks/tests.rs`.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;

const EMAIL: (&str, &str) = ("cf-access-authenticated-user-email", "Preacher@Example.org");

async fn app() -> Router {
    let state = AppState::in_memory().await.expect("state");
    let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    let passages = (1..=4)
        .map(|verse| {
            let reference = BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, verse, verse)
                .expect("reference");
            BiblePassage::new(reference, seb.clone(), format!("verš {verse}"))
        })
        .collect();
    let batch = BibleIngestionBatch::new(seb, passages).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
    build_router(state)
}

/// A request from the tunnel (no peer) or, with `lan`, from a LAN peer.
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
    lan: bool,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    if lan {
        let peer: SocketAddr = "192.0.2.7:40000".parse().expect("peer");
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn heb(verse: u16) -> Value {
    json!({ "name": "Kázeň", "translation": "slk-seb", "bookCode": "HEB", "chapter": 11, "verse": verse })
}

#[tokio::test]
async fn save_list_rename_move_and_delete_answer_camel_case() {
    let app = app().await;
    let (status, made) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(4)),
        &[EMAIL],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["name"], "Kázeň");
    assert_eq!(made["translation"], "slk-seb");
    assert_eq!(made["bookCode"], "HEB");
    assert_eq!(made["book"], "Hebrejom");
    assert_eq!(
        (made["chapter"].clone(), made["verse"].clone()),
        (json!(11), json!(4))
    );
    let id = made["id"].as_i64().expect("id");

    let (status, list) = call(&app, "GET", "/bible/study/bookmarks", None, &[EMAIL], false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list, json!([made]));

    let uri = format!("/bible/study/bookmarks/{id}");
    let (status, renamed) = call(
        &app,
        "PATCH",
        &uri,
        Some(json!({ "name": "Nedeľa" })),
        &[EMAIL],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(
        (renamed["name"].clone(), renamed["verse"].clone()),
        (json!("Nedeľa"), json!(4))
    );
    let place = json!({ "place": { "translation": "slk-seb", "bookCode": "HEB", "chapter": 11, "verse": 2 } });
    let (status, moved) = call(&app, "PATCH", &uri, Some(place), &[EMAIL], false).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(
        (moved["name"].clone(), moved["verse"].clone()),
        (json!("Nedeľa"), json!(2))
    );

    let (status, deleted) = call(&app, "DELETE", &uri, None, &[EMAIL], false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["index"], 0);
    assert_eq!(deleted["bookmark"]["name"], "Nedeľa");
    let (status, _) = call(&app, "DELETE", &uri, None, &[EMAIL], false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, empty) = call(&app, "GET", "/bible/study/bookmarks", None, &[EMAIL], false).await;
    assert_eq!(empty, json!([]));
}

#[tokio::test]
async fn the_lan_has_its_own_shared_list_and_cannot_reach_a_tunnel_list() {
    let app = app().await;
    let (_, mine) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(1)),
        &[EMAIL],
        false,
    )
    .await;
    let mine_uri = format!("/bible/study/bookmarks/{}", mine["id"]);
    // The LAN may send the header itself: it is not trusted from a LAN peer.
    let (status, lan) = call(&app, "GET", "/bible/study/bookmarks", None, &[EMAIL], true).await;
    assert_eq!((status, lan), (StatusCode::OK, json!([])));
    let (status, _) = call(
        &app,
        "PATCH",
        &mine_uri,
        Some(json!({ "name": "x" })),
        &[EMAIL],
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "DELETE", &mine_uri, None, &[], true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, shared) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(2)),
        &[],
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, lan) = call(&app, "GET", "/bible/study/bookmarks", None, &[], true).await;
    assert_eq!(lan, json!([shared]));
    let (_, tunnel) = call(&app, "GET", "/bible/study/bookmarks", None, &[EMAIL], false).await;
    assert_eq!(tunnel, json!([mine]));
}

#[tokio::test]
async fn a_refusal_is_422_with_the_slovak_sentence() {
    let app = app().await;
    let cases = [
        (
            json!({ "name": " ", "translation": "slk-seb", "bookCode": "HEB", "chapter": 11, "verse": 1 }),
            "Záložka potrebuje názov",
        ),
        (
            json!({ "name": "x", "translation": "slk-seb", "bookCode": "HEB", "chapter": 11, "verse": 9 }),
            "Hebrejom 11:9 v preklade SEB nie je",
        ),
        (
            json!({ "name": "x", "translation": "slk-seb", "bookCode": "QQQ", "chapter": 1, "verse": 1 }),
            "Neznáma kniha QQQ",
        ),
    ];
    for (body, message) in cases {
        let (status, answer) = call(
            &app,
            "POST",
            "/bible/study/bookmarks",
            Some(body),
            &[],
            true,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["message"], message);
    }
    let (_, made) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(1)),
        &[],
        true,
    )
    .await;
    let uri = format!("/bible/study/bookmarks/{}", made["id"]);
    let (status, answer) = call(&app, "PATCH", &uri, Some(json!({})), &[], true).await;
    assert_eq!(
        (status, answer["message"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("Nie je čo zmeniť"))
    );
    let (status, _) = call(
        &app,
        "PATCH",
        "/bible/study/bookmarks/999",
        Some(json!({ "name": "x" })),
        &[],
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
