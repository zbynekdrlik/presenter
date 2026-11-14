//! #915: the bookmark API over HTTP — status codes, the camelCase shape and
//! whose list a request reaches. The rules are pinned in
//! `state/study_bookmarks/tests.rs`.

use crate::access::test_support;
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
use std::sync::LazyLock;
use tower::ServiceExt;

/// A verified Access login of the preacher (#915: the e-mail header alone counts for nothing).
static TOKEN: LazyLock<String> = LazyLock::new(|| test_support::token("Preacher@Example.org"));

fn email() -> (&'static str, &'static str) {
    ("cf-access-jwt-assertion", TOKEN.as_str())
}

async fn app() -> Router {
    let mut state = AppState::in_memory().await.expect("state");
    state.set_access_identity(test_support::identity());
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

async fn list(app: &Router, headers: &[(&str, &str)], lan: bool) -> Value {
    let (status, list) = call(app, "GET", "/bible/study/bookmarks", None, headers, lan).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list
}

fn verses(list: &Value) -> Vec<i64> {
    list.as_array()
        .expect("array")
        .iter()
        .map(|row| row["verse"].as_i64().expect("verse"))
        .collect()
}

#[tokio::test]
async fn save_list_reorder_and_delete_answer_camel_case_newest_first() {
    let app = app().await;
    let (status, made) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(4)),
        &[email()],
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
    // A write answers no text; the list carries it.
    assert_eq!(made["text"], Value::Null);
    let mut ids = vec![made["id"].as_i64().expect("id")];
    for verse in [1, 2] {
        let (_, row) = call(
            &app,
            "POST",
            "/bible/study/bookmarks",
            Some(heb(verse)),
            &[email()],
            false,
        )
        .await;
        ids.push(row["id"].as_i64().expect("id"));
    }
    let shown = list(&app, &[email()], false).await;
    assert_eq!(verses(&shown), [2, 1, 4]);
    assert_eq!(shown[2]["text"], "verš 4");
    assert_eq!(shown[2]["id"], made["id"]);

    // Reorder: the whole list, top-first.
    let order = json!({ "ids": [ids[0], ids[2], ids[1]] });
    let (status, _) = call(
        &app,
        "PUT",
        "/bible/study/bookmarks/order",
        Some(order),
        &[email()],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(verses(&list(&app, &[email()], false).await), [4, 2, 1]);
    // A stale set: 409 with the Slovak sentence, nothing changed.
    let (status, answer) = call(
        &app,
        "PUT",
        "/bible/study/bookmarks/order",
        Some(json!({ "ids": [ids[0], ids[1]] })),
        &[email()],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        answer["message"],
        "Záložky sa medzitým zmenili — skús znova"
    );

    // Delete two at once: each with its old index, ascending.
    let (status, deleted) = call(
        &app,
        "POST",
        "/bible/study/bookmarks/delete",
        Some(json!({ "ids": [ids[1], ids[0]] })),
        &[email()],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted[0]["index"], 0);
    assert_eq!(deleted[0]["bookmark"]["verse"], 4);
    assert_eq!(deleted[1]["index"], 2);
    assert_eq!(deleted[1]["bookmark"]["verse"], 1);
    assert_eq!(verses(&list(&app, &[email()], false).await), [2]);
    let (status, _) = call(
        &app,
        "POST",
        "/bible/study/bookmarks/delete",
        Some(json!({ "ids": [ids[0]] })),
        &[email()],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The rename / move route is gone (#951).
    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/bible/study/bookmarks/{}", ids[2]),
        Some(json!({ "name": "x" })),
        &[email()],
        false,
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "{status}"
    );
}

#[tokio::test]
async fn the_lan_has_its_own_shared_list_and_cannot_reach_a_tunnel_list() {
    let app = app().await;
    let (_, mine) = call(
        &app,
        "POST",
        "/bible/study/bookmarks",
        Some(heb(1)),
        &[email()],
        false,
    )
    .await;
    // The LAN may send the header itself: it is not trusted from a LAN peer.
    assert_eq!(list(&app, &[email()], true).await, json!([]));
    let ids = json!({ "ids": [mine["id"]] });
    let (status, _) = call(
        &app,
        "POST",
        "/bible/study/bookmarks/delete",
        Some(ids.clone()),
        &[],
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        "PUT",
        "/bible/study/bookmarks/order",
        Some(ids),
        &[],
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
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
    let lan = list(&app, &[], true).await;
    assert_eq!(lan[0]["id"], shared["id"]);
    assert_eq!(lan.as_array().map(Vec::len), Some(1));
    let tunnel = list(&app, &[email()], false).await;
    assert_eq!(tunnel[0]["id"], mine["id"]);
    assert_eq!(tunnel.as_array().map(Vec::len), Some(1));
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
    let (status, answer) = call(
        &app,
        "POST",
        "/bible/study/bookmarks/delete",
        Some(json!({ "ids": [] })),
        &[],
        true,
    )
    .await;
    assert_eq!(
        (status, answer["message"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("Nie je čo zmazať"))
    );
}
