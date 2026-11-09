//! #915: the identity middleware over HTTP — the token sets the remembered
//! device, the cookie alone identifies the reader at church, the LAN and a
//! spoofed header identify nobody.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::SET_COOKIE;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::access::test_support;
use crate::router::build_router;
use crate::state::AppState;

async fn app() -> Router {
    let mut state = AppState::in_memory().await.expect("state");
    state.set_access_identity(test_support::identity());
    build_router(state)
}

/// `GET /bible/study/identity` → (status, body, Set-Cookie values).
async fn identity(
    app: &Router,
    headers: &[(&str, &str)],
    lan: bool,
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder().uri("/bible/study/identity");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::empty()).expect("request");
    if lan {
        let peer: SocketAddr = "192.0.2.7:40000".parse().expect("peer");
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("ascii").to_string())
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).expect("json"),
        cookies,
    )
}

#[tokio::test]
async fn a_login_at_home_is_remembered_for_the_church_bypass() {
    let app = app().await;
    let token = test_support::token("Preacher@Example.org");
    let (status, body, cookies) = identity(
        &app,
        &[
            ("cf-access-jwt-assertion", &token),
            ("x-forwarded-proto", "https"),
        ],
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "email": "preacher@example.org" }));
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert!(
        cookies[0].contains("HttpOnly") && cookies[0].contains("Secure"),
        "{cookies:?}"
    );

    // At church: no token, the browser sends the cookie back.
    let pair = cookies[0].split(';').next().expect("pair").to_string();
    let (_, body, cookies) = identity(&app, &[("cookie", &pair)], false).await;
    assert_eq!(body, json!({ "email": "preacher@example.org" }));
    assert!(cookies.is_empty(), "no new cookie for a remembered device");

    // Plain http through the tunnel: the cookie is not marked Secure.
    let (_, _, cookies) = identity(&app, &[("cf-access-jwt-assertion", &token)], false).await;
    assert!(!cookies[0].contains("Secure"), "{cookies:?}");
}

#[tokio::test]
async fn the_lan_a_spoofed_header_or_a_forged_token_is_nobody() {
    let app = app().await;
    let (_, body, cookies) = identity(
        &app,
        &[("cf-access-authenticated-user-email", "boss@example.org")],
        false,
    )
    .await;
    assert_eq!((body, cookies.len()), (json!({ "email": null }), 0));

    let forged = test_support::forge(&test_support::claims("boss@example.org"));
    let (_, body, cookies) = identity(&app, &[("cf-access-jwt-assertion", &forged)], false).await;
    assert_eq!((body, cookies.len()), (json!({ "email": null }), 0));

    let token = test_support::token("boss@example.org");
    let (_, body, cookies) = identity(&app, &[("cf-access-jwt-assertion", &token)], true).await;
    assert_eq!((body, cookies.len()), (json!({ "email": null }), 0));

    let (_, body, _) = identity(&app, &[], false).await;
    assert_eq!(body, json!({ "email": null }));
}

#[tokio::test]
async fn a_site_without_access_config_identifies_nobody() {
    let app = build_router(AppState::in_memory().await.expect("state"));
    let token = test_support::token("preacher@example.org");
    let (_, body, cookies) = identity(&app, &[("cf-access-jwt-assertion", &token)], false).await;
    assert_eq!((body, cookies.len()), (json!({ "email": null }), 0));
}
