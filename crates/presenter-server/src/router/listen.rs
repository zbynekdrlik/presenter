//! #852 C1a: `GET|PUT /listen/mics` (the whole list; 422 with the Slovak rule)
//! and `GET /listen/status` (polled → a quiet GET in `request_log.rs`).

use super::integrations::extract_actor;
use super::AppError;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use presenter_core::listen::mics::{MicConfig, MicsValidationError};
use presenter_core::listen::status::ListenStatus;
use tracing::instrument;

/// The listen sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/listen/mics", get(get_mics).put(put_mics))
        .route("/listen/status", get(get_status))
}

#[instrument(skip_all)]
async fn get_mics(State(state): State<AppState>) -> Result<Json<Vec<MicConfig>>, AppError> {
    Ok(Json(state.listen_mics().await?))
}

#[instrument(skip_all)]
async fn put_mics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mics): Json<Vec<MicConfig>>,
) -> Result<Json<Vec<MicConfig>>, AppError> {
    let actor = extract_actor(&headers);
    state
        .set_listen_mics(mics, &actor)
        .await
        .map(Json)
        .map_err(map_mics_error)
}

async fn get_status(State(state): State<AppState>) -> Json<ListenStatus> {
    Json(state.listen_status())
}

/// `MicsValidationError` → 422 with its Slovak text; everything else → the
/// central mapping (a corrupt stored list → 500).
fn map_mics_error(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<MicsValidationError>() {
        Some(refusal) => AppError::unprocessable(refusal.to_string()),
        None => AppError::from(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::build_router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use presenter_core::audio::vban::VbanSampleType;
    use presenter_core::audio::vban_encode::{tone, VbanEncoder};
    use presenter_core::listen::mics::LISTEN_MICS_KEY;
    use presenter_core::listen::status::SocketState;
    use serde_json::{json, Value};
    use std::net::SocketAddr;
    use std::time::Duration;
    use tower::ServiceExt;

    /// The spec §5.6 example.
    const SPEC_MICS: &str = r#"[
        {"id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null,
         "channel": "left", "language": "en", "role": "guest"},
        {"id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null,
         "channel": "left", "language": "sk", "role": "interpreter"}]"#;

    async fn send(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-forwarded-for", "203.0.113.9");
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn app() -> (AppState, Router) {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());
        (state, app)
    }

    #[tokio::test]
    async fn a_fresh_server_has_no_mics_and_an_unstarted_receiver() {
        let (_state, app) = app().await;
        assert_eq!(
            send(&app, Method::GET, "/listen/mics", None).await,
            (StatusCode::OK, json!([]))
        );
        let (status, body) = send(&app, Method::GET, "/listen/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["socket"]["state"], "not_started");
        assert_eq!(
            (body["mics"].clone(), body["unmatched"].clone()),
            (json!([]), json!([]))
        );
    }

    #[tokio::test]
    async fn put_stores_normalizes_audits_and_applies_the_list() {
        let (state, app) = app().await;
        let mut mics: Value = serde_json::from_str(SPEC_MICS).unwrap();
        mics[0]["name"] = json!("  Hosť ");
        mics[0]["senderIp"] = json!(" 192.0.2.10 ");
        let (status, saved) = send(&app, Method::PUT, "/listen/mics", Some(mics)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(
            (saved[0]["name"].clone(), saved[0]["senderIp"].clone()),
            (json!("Hosť"), json!("192.0.2.10"))
        );
        assert_eq!(
            send(&app, Method::GET, "/listen/mics", None).await,
            (StatusCode::OK, saved.clone())
        );
        let stored = state
            .repository()
            .get_app_setting(LISTEN_MICS_KEY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), saved);
        let audit = state
            .repository()
            .list_settings_audit(Some("app_settings"), Some(LISTEN_MICS_KEY), None, 10)
            .await
            .unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(
            (audit[0].actor.as_str(), audit[0].before_json.clone()),
            ("203.0.113.9", None)
        );
        assert_eq!(audit[0].after_json, saved);
        let (_, live) = send(&app, Method::GET, "/listen/status", None).await;
        let ids: Vec<&str> = live["mics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["guest", "interp"], "the receiver got the list");
        assert_eq!(live["mics"][0]["state"], "waiting");
    }

    #[tokio::test]
    async fn refused_lists_are_422_with_the_slovak_rule_and_store_nothing() {
        let (state, app) = app().await;
        let base: Value = serde_json::from_str(SPEC_MICS).unwrap();
        let mut duplicate = base.clone();
        duplicate[1]["id"] = json!("guest");
        let mut bad_stream = base.clone();
        bad_stream[0]["streamName"] = json!("HOSŤ_EN");
        let mut ambiguous = base.clone();
        ambiguous[1]["streamName"] = json!("GUEST_EN");
        for (body, needle) in [
            (duplicate, "dvakrát"),
            (bad_stream, "VBAN streamu"),
            (ambiguous, "ten istý VBAN stream"),
        ] {
            let (status, reply) = send(&app, Method::PUT, "/listen/mics", Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reply}");
            assert!(
                reply["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(needle),
                "{reply}"
            );
        }
        assert_eq!(
            state
                .repository()
                .get_app_setting(LISTEN_MICS_KEY)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_corrupt_stored_list_is_a_500_and_the_receiver_stays_idle() {
        let (state, app) = app().await;
        state
            .repository()
            // not JSON (no brace in the literal: fn_length_check.py counts braces)
            .set_app_setting(LISTEN_MICS_KEY, "not a list")
            .await
            .unwrap();
        let (status, reply) = send(&app, Method::GET, "/listen/mics", None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            reply["message"]
                .as_str()
                .unwrap_or_default()
                .contains("poškodené"),
            "{reply}"
        );
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while state.listen_status().socket.state != SocketState::Idle {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{:?}",
                state.listen_status()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            state.listen_status().mics.is_empty(),
            "nothing received from a corrupt list"
        );
    }

    /// The integration test of the unit: real VBAN packets (the ported
    /// songplayer encoder) over real UDP into the server, read back through a
    /// real HTTP `GET /listen/status`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_vban_packets_over_udp_show_in_get_listen_status() {
        let state = AppState::in_memory().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = build_router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();
        let guest = json!([{"id": "guest", "name": "Hosť", "streamName": "GUEST_EN",
            "senderIp": "127.0.0.1", "channel": "left", "language": "en", "role": "guest"}]);
        let saved = http
            .put(format!("{base}/listen/mics"))
            .json(&guest)
            .send()
            .await
            .unwrap();
        assert_eq!(saved.status(), reqwest::StatusCode::OK);
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let status = poll_status(&http, &base, |s| s["socket"]["state"] == "bound").await;
        let vban: SocketAddr = status["socket"]["localAddr"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        tx.send_to(b"hello", vban).await.unwrap(); // not VBAN → rejected
        let mut stray = VbanEncoder::new("Stream1", 48_000, 2, VbanSampleType::Int24);
        let stray_packet = stray.packet(&tone(48_000, 440.0, 0.25, 0, 64, 2)).unwrap();
        tx.send_to(&stray_packet, vban).await.unwrap();
        // 95 counters of 256 frames, counter 10 left out: 94 packets (C1a-3: 1 lost,
        // 24 320 frames = 25 chunks = 8 000 samples = 5 frames at 16 kHz).
        let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16);
        let block = tone(48_000, 1_000.0, 0.5, 0, 95 * 256, 1);
        for (k, chunk) in block.chunks(256).enumerate() {
            if k == 10 {
                encoder.skip(1);
                continue;
            }
            tx.send_to(&encoder.packet(chunk).unwrap(), vban)
                .await
                .unwrap();
            if k % 4 == 3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        let status = poll_status(&http, &base, |s| s["mics"][0]["packets"] == 94).await;
        let mic = &status["mics"][0];
        assert_eq!(mic["state"], "receiving");
        assert_eq!(mic["packets"], json!(94));
        assert_eq!(
            mic["format"],
            json!({"rateHz": 48000, "channels": 1, "sampleType": "int16"})
        );
        assert_eq!(mic["framesPerPacket"], json!(256));
        assert_eq!(mic["sender"], json!(tx.local_addr().unwrap().to_string()));
        assert_eq!(status["unmatched"][0]["streamName"], "Stream1");
        assert_eq!(status["unmatched"][0]["format"]["channels"], json!(2));
    }

    async fn poll_status(
        http: &reqwest::Client,
        base: &str,
        done: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status: Value = http
                .get(format!("{base}/listen/status"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if done(&status) {
                return status;
            }
            assert!(tokio::time::Instant::now() < deadline, "{status:#}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
