//! Šepkár tips REST API (#852 spec §9.4), merged with one
//! `.merge(interpreter_tips::router())` in `router.rs`:
//! `GET|POST /interpreter/tips`, `POST /interpreter/tips/dismiss-all`,
//! `POST /interpreter/tips/{id}/dismiss`, `POST /interpreter/tips/{id}/restore`.
//!
//! Repository refusals (unknown id 404, another day's restore 409) map
//! through the central `From<anyhow::Error> for AppError`; the state-layer
//! `TipRefusal` maps here: 422 with the Slovak rule, 429 with `Retry-After`.
//! The client IP follows the #793 rule (`integrations::extract_client_ip`).

use super::AppError;
use crate::state::interpreter_tips::{TipOrigin, TipRefusal, TipScope};
use crate::state::AppState;
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header::USER_AGENT, HeaderMap, StatusCode},
    routing::{get, post},
    Extension, Json, Router,
};
use presenter_core::interpreter_tips::{
    clean_user_agent, CreateTipResponse, DismissAllResponse, NewTipRequest, TipDto, TipsList,
};
use serde::Deserialize;
use std::net::SocketAddr;
use tracing::instrument;

/// The tips sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/interpreter/tips", get(list_tips).post(create_tip))
        .route("/interpreter/tips/dismiss-all", post(dismiss_all))
        .route("/interpreter/tips/{id}/dismiss", post(dismiss_tip))
        .route("/interpreter/tips/{id}/restore", post(restore_tip))
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct TipsQuery {
    #[serde(default)]
    scope: TipScope,
}

/// The socket peer; optional so a router served without connect-info
/// (tests) still answers.
type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

fn client_ip(headers: &HeaderMap, peer: Peer) -> String {
    super::integrations::extract_client_ip(headers, peer.map(|Extension(ConnectInfo(addr))| addr))
}

fn origin(headers: &HeaderMap, peer: Peer) -> TipOrigin {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .and_then(clean_user_agent);
    TipOrigin {
        client_ip: client_ip(headers, peer),
        user_agent,
    }
}

/// `TipRefusal` → 422 / 429 + `Retry-After`; everything else → the central
/// mapping (404 / 409 / 500).
fn map_tip_error(err: anyhow::Error) -> AppError {
    let Some((message, retry_after)) = err
        .downcast_ref::<TipRefusal>()
        .map(|refusal| (refusal.to_string(), refusal.retry_after_secs()))
    else {
        return AppError::from(err);
    };
    match retry_after {
        None => AppError::unprocessable(message),
        Some(secs) => AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            error: anyhow::anyhow!(message),
            retry_after: Some(secs),
        },
    }
}

#[instrument(skip_all)]
pub(super) async fn list_tips(
    State(state): State<AppState>,
    Query(query): Query<TipsQuery>,
) -> Result<Json<TipsList>, AppError> {
    Ok(Json(state.interpreter_tips_list(query.scope).await?))
}

#[instrument(skip_all)]
pub(super) async fn create_tip(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(request): Json<NewTipRequest>,
) -> Result<(StatusCode, Json<CreateTipResponse>), AppError> {
    let created = state
        .create_interpreter_tip(&request, origin(&headers, peer))
        .await
        .map_err(map_tip_error)?;
    let status = if created.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(CreateTipResponse {
            tip: created.tip,
            duplicate: created.duplicate,
        }),
    ))
}

#[instrument(skip_all)]
pub(super) async fn dismiss_tip(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<i64>,
) -> Result<Json<TipDto>, AppError> {
    let ip = client_ip(&headers, peer);
    Ok(Json(state.dismiss_interpreter_tip(id, &ip).await?))
}

#[instrument(skip_all)]
pub(super) async fn restore_tip(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<i64>,
) -> Result<Json<TipDto>, AppError> {
    let ip = client_ip(&headers, peer);
    Ok(Json(state.restore_interpreter_tip(id, &ip).await?))
}

#[instrument(skip_all)]
pub(super) async fn dismiss_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Json<DismissAllResponse>, AppError> {
    let ip = client_ip(&headers, peer);
    let dismissed = state.dismiss_all_interpreter_tips(&ip).await?;
    Ok(Json(DismissAllResponse { dismissed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::build_router;
    use crate::state::interpreter_tips::test_support::{last_tips_event, ManualTipClock};
    use crate::tracing_capture::{quoted, Capture};
    use axum::body::Body;
    use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
    use axum::http::{Method, Request};
    use presenter_core::interpreter_tips::TipValidationError;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    const IP: &str = "203.0.113.9";

    struct Reply {
        status: StatusCode,
        retry_after: Option<String>,
        body: Value,
    }

    async fn send(app: &Router, method: Method, uri: &str, body: Option<Value>, ip: &str) -> Reply {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-forwarded-for", ip)
            .header(USER_AGENT, "Mozilla/5.0 (iPhone)");
        let request = match body {
            Some(body) => builder
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            retry_after,
            body,
        }
    }

    async fn post_tip(app: &Router, ip: &str, body: Value) -> Reply {
        send(app, Method::POST, "/interpreter/tips", Some(body), ip).await
    }

    async fn get_json(app: &Router, uri: &str) -> Value {
        let reply = send(app, Method::GET, uri, None, IP).await;
        assert_eq!(reply.status, StatusCode::OK, "{uri}");
        reply.body
    }

    async fn app_at(local: &str) -> (AppState, Router, Arc<ManualTipClock>) {
        let state = AppState::in_memory().await.unwrap();
        let clock = ManualTipClock::at(local);
        state.set_interpreter_tip_clock(clock.clone());
        let app = build_router(state.clone());
        (state, app, clock)
    }

    fn tip_ids(list: &Value) -> Vec<i64> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|tip| tip["id"].as_i64().unwrap())
            .collect()
    }

    fn ids_of(tips: &[TipDto]) -> Vec<i64> {
        tips.iter().map(|tip| tip.id).collect()
    }

    #[tokio::test]
    async fn a_pair_a_note_or_both_is_201_with_the_tip() {
        let (_state, app, _clock) = app_at("2026-10-18T19:42:05+02:00").await;
        let pair = post_tip(
            &app,
            IP,
            json!({"senderName": " Peter ", "sourceTerm": "atonement\t", "targetTerm": "zmierenie"}),
        )
        .await;
        assert_eq!(pair.status, StatusCode::CREATED, "{:?}", pair.body);
        assert_eq!(pair.body["serviceDate"], json!("2026-10-18"));
        assert_eq!(pair.body["senderName"], json!("Peter"));
        assert_eq!(pair.body["sourceTerm"], json!("atonement"));
        assert_eq!(pair.body["targetTerm"], json!("zmierenie"));
        assert_eq!(pair.body["note"], Value::Null);
        assert_eq!(pair.body["createdAt"], json!("2026-10-18T17:42:05Z"));
        assert_eq!(pair.body["dismissedAt"], Value::Null);
        assert_eq!(pair.body["duplicate"], json!(false));
        assert!(pair.body.get("senderIp").is_none() && pair.body.get("userAgent").is_none());
        let note = post_tip(
            &app,
            IP,
            json!({"senderName": "Jana", "note": "hovorí o milosti"}),
        )
        .await;
        assert_eq!(note.status, StatusCode::CREATED);
        let both = post_tip(
            &app,
            IP,
            json!({"senderName": "Eva", "sourceTerm": "grace", "targetTerm": "milosť", "note": "nie láskavosť"}),
        )
        .await;
        assert_eq!(both.status, StatusCode::CREATED);
        assert_eq!(
            tip_ids(&get_json(&app, "/interpreter/tips").await["tips"]).len(),
            3
        );
    }

    #[tokio::test]
    async fn invalid_tips_are_422_with_the_slovak_rule() {
        let (_state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        let long = |n: usize| "ž".repeat(n);
        let cases = [
            (
                json!({"sourceTerm": "a", "targetTerm": "b"}),
                TipValidationError::MissingSender,
            ),
            (
                json!({"senderName": "   ", "note": "x"}),
                TipValidationError::MissingSender,
            ),
            (
                json!({"senderName": long(41), "note": "x"}),
                TipValidationError::SenderTooLong,
            ),
            (json!({"senderName": "Peter"}), TipValidationError::Empty),
            (
                json!({"senderName": "Peter", "note": "\u{7}\t\n"}),
                TipValidationError::Empty,
            ),
            (
                json!({"senderName": "Peter", "sourceTerm": "atonement"}),
                TipValidationError::HalfPair,
            ),
            (
                json!({"senderName": "Peter", "sourceTerm": long(61), "targetTerm": "b"}),
                TipValidationError::TermTooLong,
            ),
            (
                json!({"senderName": "Peter", "note": long(201)}),
                TipValidationError::NoteTooLong,
            ),
        ];
        for (body, expected) in cases {
            let reply = post_tip(&app, IP, body.clone()).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(reply.body["message"], json!(expected.to_string()), "{body}");
        }
        assert!(tip_ids(&get_json(&app, "/interpreter/tips?scope=today").await["tips"]).is_empty());
        // Eight refusals from "Peter" used no rate slot: five valid tips still pass.
        for n in 0..5 {
            let reply = post_tip(
                &app,
                IP,
                json!({"senderName": "Peter", "note": format!("ok {n}")}),
            )
            .await;
            assert_eq!(reply.status, StatusCode::CREATED, "valid tip {n}");
        }
    }

    #[tokio::test]
    async fn the_sixth_tip_of_one_sender_within_a_minute_is_429_with_retry_after() {
        let (_state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        for n in 0..5 {
            let reply = post_tip(
                &app,
                IP,
                json!({"senderName": "Peter", "note": format!("tip {n}")}),
            )
            .await;
            assert_eq!(reply.status, StatusCode::CREATED, "tip {n}");
        }
        let refused = post_tip(&app, IP, json!({"senderName": "Peter", "note": "tip 5"})).await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        let secs: u64 = refused
            .retry_after
            .as_deref()
            .expect("Retry-After")
            .parse()
            .expect("seconds");
        assert!((1..=60).contains(&secs), "{secs}");
        let message = refused.body["message"].as_str().unwrap();
        assert!(
            message.starts_with("Pomaly — najviac 5 tipov za minútu."),
            "{message}"
        );
        assert!(message.ends_with(&format!("o {secs} s.")), "{message}");
        let jana = post_tip(&app, IP, json!({"senderName": "Jana", "note": "tip"})).await;
        assert_eq!(
            jana.status,
            StatusCode::CREATED,
            "another name behind the same IP"
        );
        let elsewhere = post_tip(
            &app,
            "192.0.2.7",
            json!({"senderName": "Peter", "note": "iný"}),
        )
        .await;
        assert_eq!(
            elsewhere.status,
            StatusCode::CREATED,
            "the same name from another device"
        );
        let today = get_json(&app, "/interpreter/tips?scope=today").await;
        assert!(!today["tips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tip| tip["note"] == "tip 5"));
    }

    #[tokio::test]
    async fn the_thirty_first_tip_in_a_minute_is_refused_overall() {
        let (_state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        for sender in 0..6 {
            for n in 0..5 {
                let body = json!({"senderName": format!("s{sender}"), "note": format!("tip {sender}-{n}")});
                assert_eq!(post_tip(&app, IP, body).await.status, StatusCode::CREATED);
            }
        }
        let refused = post_tip(
            &app,
            IP,
            json!({"senderName": "s6", "note": "one too many"}),
        )
        .await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(refused.body["message"]
            .as_str()
            .unwrap()
            .starts_with("Tlmočník dostal veľa tipov naraz."));
    }

    #[tokio::test]
    async fn the_same_pair_within_two_minutes_is_200_duplicate_and_no_new_row() {
        let (_state, app, clock) = app_at("2026-10-18T19:42:00+02:00").await;
        let first = post_tip(
            &app,
            IP,
            json!({"senderName": "Peter", "sourceTerm": "atonement", "targetTerm": "zmierenie"}),
        )
        .await;
        assert_eq!(first.status, StatusCode::CREATED);
        let again = post_tip(
            &app,
            "192.0.2.7",
            json!({"senderName": "Jana", "sourceTerm": " Atonement ", "targetTerm": "ZMIERENIE"}),
        )
        .await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(again.body["duplicate"], json!(true));
        assert_eq!(again.body["id"], first.body["id"]);
        assert_eq!(
            again.body["senderName"],
            json!("Peter"),
            "the earlier tip is returned"
        );
        clock.advance(chrono::Duration::seconds(121));
        let later = post_tip(
            &app,
            IP,
            json!({"senderName": "Jana", "sourceTerm": "atonement", "targetTerm": "zmierenie"}),
        )
        .await;
        assert_eq!(later.status, StatusCode::CREATED);
        assert_eq!(
            tip_ids(&get_json(&app, "/interpreter/tips?scope=today").await["tips"]).len(),
            2
        );
    }

    #[tokio::test]
    async fn dismiss_is_idempotent_and_restore_brings_the_tip_back() {
        let (_state, app, clock) = app_at("2026-10-18T19:00:00+02:00").await;
        let id = post_tip(&app, IP, json!({"senderName": "Peter", "note": "a"}))
            .await
            .body["id"]
            .as_i64()
            .unwrap();
        let first = send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{id}/dismiss"),
            None,
            IP,
        )
        .await;
        assert_eq!(first.status, StatusCode::OK);
        assert!(first.body["dismissedAt"].is_string());
        clock.advance(chrono::Duration::seconds(30));
        let again = send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{id}/dismiss"),
            None,
            IP,
        )
        .await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(again.body["dismissedAt"], first.body["dismissedAt"]);
        assert!(get_json(&app, "/interpreter/tips").await["tips"]
            .as_array()
            .unwrap()
            .is_empty());
        let today = get_json(&app, "/interpreter/tips?scope=today").await;
        assert_eq!(tip_ids(&today["tips"]), vec![id]);
        assert!(today["tips"][0]["dismissedAt"].is_string());
        assert_eq!(today["activeIds"], json!([]));
        let restored = send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{id}/restore"),
            None,
            IP,
        )
        .await;
        assert_eq!(restored.status, StatusCode::OK);
        assert_eq!(restored.body["dismissedAt"], Value::Null);
        let active = get_json(&app, "/interpreter/tips").await;
        assert_eq!(tip_ids(&active["tips"]), vec![id]);
        assert_eq!(active["activeIds"], json!([id]));
    }

    #[tokio::test]
    async fn unknown_ids_are_404() {
        let (_state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        for id in ["999999", "99999999999"] {
            for action in ["dismiss", "restore"] {
                let reply = send(
                    &app,
                    Method::POST,
                    &format!("/interpreter/tips/{id}/{action}"),
                    None,
                    IP,
                )
                .await;
                assert_eq!(reply.status, StatusCode::NOT_FOUND, "{action} {id}");
                assert_eq!(reply.body["message"], json!("Tip neexistuje."));
            }
        }
    }

    #[tokio::test]
    async fn restoring_a_tip_of_another_day_is_409() {
        let (_state, app, clock) = app_at("2026-10-18T22:00:00+02:00").await;
        let id = post_tip(&app, IP, json!({"senderName": "Peter", "note": "a"}))
            .await
            .body["id"]
            .as_i64()
            .unwrap();
        send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{id}/dismiss"),
            None,
            IP,
        )
        .await;
        clock.advance(chrono::Duration::hours(3));
        let reply = send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{id}/restore"),
            None,
            IP,
        )
        .await;
        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(
            reply.body["message"],
            json!("Tento tip už nie je z dnešnej bohoslužby.")
        );
    }

    #[tokio::test]
    async fn every_write_publishes_the_full_active_list_newest_first() {
        let (state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        let mut rx = state.live_hub().subscribe();
        let a = post_tip(&app, IP, json!({"senderName": "Peter", "note": "a"}))
            .await
            .body["id"]
            .as_i64()
            .unwrap();
        let (date, tips) = last_tips_event(&mut rx).expect("create publishes");
        assert_eq!(date, "2026-10-18");
        assert_eq!(ids_of(&tips), vec![a]);
        let b = post_tip(&app, IP, json!({"senderName": "Jana", "note": "b"}))
            .await
            .body["id"]
            .as_i64()
            .unwrap();
        assert_eq!(
            ids_of(&last_tips_event(&mut rx).expect("create").1),
            vec![b, a]
        );
        send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{a}/dismiss"),
            None,
            IP,
        )
        .await;
        assert_eq!(
            ids_of(&last_tips_event(&mut rx).expect("dismiss").1),
            vec![b]
        );
        send(
            &app,
            Method::POST,
            &format!("/interpreter/tips/{a}/restore"),
            None,
            IP,
        )
        .await;
        assert_eq!(
            ids_of(&last_tips_event(&mut rx).expect("restore").1),
            vec![b, a]
        );
        let all = send(
            &app,
            Method::POST,
            "/interpreter/tips/dismiss-all",
            None,
            IP,
        )
        .await;
        assert_eq!(all.status, StatusCode::OK);
        assert_eq!(all.body, json!({"dismissed": 2}));
        assert!(last_tips_event(&mut rx).expect("dismiss-all").1.is_empty());
        let c = post_tip(&app, IP, json!({"senderName": "Eva", "note": "c"})).await;
        assert_eq!(c.status, StatusCode::CREATED);
        assert!(last_tips_event(&mut rx).is_some());
        let again = post_tip(&app, IP, json!({"senderName": "Marek", "note": "C"})).await;
        assert_eq!(again.body["duplicate"], json!(true));
        assert!(
            last_tips_event(&mut rx).is_none(),
            "a duplicate changes nothing and publishes nothing"
        );
    }

    #[tokio::test]
    async fn a_created_tip_logs_one_info_line_with_the_audit_fields() {
        let capture = Capture::install();
        let (_state, app, _clock) = app_at("2026-10-18T19:00:00+02:00").await;
        let reply = post_tip(
            &app,
            IP,
            json!({"senderName": "Peter", "sourceTerm": "atonement", "targetTerm": "zmierenie"}),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        let events = capture.events("presenter::interpreter::tip");
        assert_eq!(events.len(), 1, "{events:?}");
        let line = &events[0];
        assert_eq!(line.level, tracing::Level::INFO);
        assert_eq!(line.field("action"), Some(quoted("create").as_str()));
        assert_eq!(
            line.field("id"),
            Some(reply.body["id"].to_string().as_str())
        );
        assert_eq!(line.field("sender_name"), Some(quoted("Peter").as_str()));
        assert_eq!(line.field("client_ip"), Some(quoted(IP).as_str()));
        assert_eq!(line.field("chars"), Some("18"));
        assert_eq!(line.field("duplicate"), Some("false"));
    }
}
