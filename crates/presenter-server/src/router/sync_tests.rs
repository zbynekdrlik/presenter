//! #555 route-shape tests for the sync + trash HTTP surface.
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use crate::state::AppState;

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body");
    let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn sync_manifest_and_status_answer() {
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state);

    let (status, json) = get_json(app.clone(), "/sync/manifest").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array(), "manifest is a JSON array: {json}");

    let (status, json) = get_json(app, "/integrations/sync/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("enabled"),
        Some(&Value::Bool(false)),
        "no peer configured in tests → disabled: {json}"
    );
}

#[tokio::test]
async fn trash_route_is_not_shadowed_and_restore_round_trips() {
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state.clone());

    // Static route answers (not swallowed by /presentations/{id}).
    let (status, json) = get_json(app.clone(), "/presentations/trash").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "trash route must answer — a 404/400 means {{id}} swallowed it"
    );
    assert_eq!(json, Value::Array(vec![]), "empty trash on a fresh DB");

    // Create + delete a song, see it in the trash, restore it, trash empties.
    let library = state.create_library("Songs").await.unwrap();
    let (_, _, pres, _) = state
        .create_presentation(library.id, "Trash Me", None)
        .await
        .unwrap();
    state.delete_presentation(pres.id).await.unwrap();

    let (status, json) = get_json(app.clone(), "/presentations/trash").await;
    assert_eq!(status, StatusCode::OK);
    let rows = json.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Trash Me");
    assert_eq!(rows[0]["libraryName"], "Songs");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/presentations/{}/restore", pres.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let (_, json) = get_json(app, "/presentations/trash").await;
    assert_eq!(json, Value::Array(vec![]), "restored song left the trash");
}

#[tokio::test]
async fn sync_presentation_content_serves_full_slides_by_sync_id() {
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state.clone());

    let library = state.create_library("Songs").await.unwrap();
    state
        .create_presentation(library.id, "Wire Song", None)
        .await
        .unwrap();

    let (_, manifest) = get_json(app.clone(), "/sync/manifest").await;
    let sync_id = manifest.as_array().expect("array")[0]["syncId"]
        .as_str()
        .expect("syncId")
        .to_string();

    let (status, json) = get_json(app.clone(), &format!("/sync/presentations/{sync_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["name"], "Wire Song");
    assert_eq!(json["libraryName"], "Songs");
    assert!(json["slides"].is_array());
    assert!(
        json["updatedAt"].is_string(),
        "camelCase timestamps: {json}"
    );

    let (status, _) = get_json(app, "/sync/presentations/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sync_presentation_content_serves_an_oversize_legacy_stored_slide_without_500ing() {
    // #558 round-4 U1(a): `fetch_sync_presentation` built its wire `Slide`
    // through `to_domain_slide`, which re-validates the STORED text via
    // `SlideText::new` (a 4000-char cap). A legacy/wire-path row that
    // already exceeds that cap (or arrived over the wire before/without
    // that validation ever running -- the APPLY side already accepts an
    // oversize slide unchecked, since `Slide`'s plain `Deserialize` impl
    // never validates either) then 500s this song's serve endpoint
    // FOREVER, on every future sync cycle. FIX: the serve path builds the
    // wire DTO from raw column strings, never re-validated.
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state.clone());

    let library = state.create_library("Songs").await.unwrap();
    state
        .create_presentation(library.id, "Oversize Song", None)
        .await
        .unwrap();

    let (_, manifest) = get_json(app.clone(), "/sync/manifest").await;
    let sync_id = manifest.as_array().expect("array")[0]["syncId"]
        .as_str()
        .expect("syncId")
        .to_string();

    // Corrupt the stored slide's worship_main directly, bypassing
    // `SlideText::new`'s validation entirely -- simulating a legacy row
    // (a normal repository call would reject it before it ever reaches
    // the DB).
    let oversize = "x".repeat(5000);
    use presenter_persistence::entities::slide as slide_entity;
    use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
    let slide_row = slide_entity::Entity::find()
        .one(state.repository().connection())
        .await
        .unwrap()
        .expect("the newly created song has one slide");
    slide_entity::Entity::update_many()
        .col_expr(
            slide_entity::Column::WorshipMain,
            Expr::value(oversize.clone()),
        )
        .filter(slide_entity::Column::Id.eq(slide_row.id.clone()))
        .exec(state.repository().connection())
        .await
        .unwrap();

    let (status, json) = get_json(app, &format!("/sync/presentations/{sync_id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an oversize legacy stored slide must serve OK, never 500: {json}"
    );
    assert_eq!(
        json["slides"][0]["content"]["main"]["value"],
        Value::String(oversize),
        "the oversize text is served verbatim, unchanged"
    );
}
