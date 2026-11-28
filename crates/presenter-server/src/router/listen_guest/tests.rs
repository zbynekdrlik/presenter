//! The guest routes (#999 L2) through a real axum server on loopback and a
//! real `tokio_tungstenite` client, the sessions talking to the scripted fake
//! Gemini (`listen::translate::test_fake`): the stream's hello / state /
//! speech / text, refusals, the language list and the operator's QR code.

use super::*;
use crate::listen::translate::stats::DayMeter;
use crate::listen::translate::test_fake::{guest, FakeGemini};
use crate::router::build_router;
use futures_util::StreamExt;
use presenter_core::listen::guest::{GuestRefusal, GuestSessionState};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A served router whose guest translation talks to `fake` (key set or not)
/// and whose box publishes the Dante mics (`source` = true).
async fn serve(
    fake: &std::sync::Arc<FakeGemini>,
    key: Option<&str>,
    source: bool,
) -> (SocketAddr, AppState) {
    let state = AppState::in_memory().await.unwrap();
    state.set_guest_translate(guest(fake, key, DayMeter::default()));
    state.listen_service().set_dante_mics(source);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, state)
}

async fn connect(addr: SocketAddr, lang: &str) -> Client {
    let url = format!("ws://{addr}/listen/guest/ws?lang={lang}");
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

/// The next frame that is not a ping, within 5 s.
async fn next(client: &mut Client) -> WsMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if !matches!(frame, WsMessage::Ping(_) | WsMessage::Pong(_)) {
            return frame;
        }
    }
}

async fn next_text(client: &mut Client) -> GuestWsMessage {
    loop {
        if let WsMessage::Text(text) = next(client).await {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// A voiced 20 ms chunk, what the mic chain hands the session (#999 L6e).
fn voiced_chunk() -> crate::listen::MicChunk {
    (0..320)
        .map(|i| if i % 20 < 10 { 8_000 } else { -8_000 })
        .collect::<Vec<i16>>()
        .into()
}

#[tokio::test]
async fn a_guest_gets_hello_state_speech_and_text() {
    let fake = FakeGemini::new(vec![]);
    let (addr, state) = serve(&fake, Some("test-key-not-real"), true).await;
    let mut client = connect(addr, "de").await;
    assert_eq!(
        next_text(&mut client).await,
        GuestWsMessage::Hello {
            language: "de".into(),
            name: "German (Deutsch)".into(),
            sample_rate: 24_000
        }
    );
    let GuestWsMessage::State { session } = next_text(&mut client).await else {
        panic!("a state second");
    };
    assert!(
        matches!(
            session,
            GuestSessionState::Connecting | GuestSessionState::Waiting
        ),
        "{session:?}"
    );
    let chunks = state.listen_service().chunk_sender("dante-mic-1");
    let mut speech = 0;
    let mut texts = Vec::new();
    // 300 ms of audio: the fake answers each 100 ms as it starts
    for _ in 0..15 {
        let _ = chunks.send(voiced_chunk());
        tokio::time::sleep(Duration::from_millis(6)).await;
    }
    while speech < 3 || texts.len() < 3 {
        match next(&mut client).await {
            WsMessage::Binary(pcm) => {
                assert_eq!(pcm.len(), 4_800, "100 ms of s16 at 24 kHz");
                speech += 1;
            }
            WsMessage::Text(text) => {
                if let GuestWsMessage::Text { text } = serde_json::from_str(&text).unwrap() {
                    texts.push(text);
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(texts, vec!["w1 ", "w2 ", "w3 "]);
    assert_eq!(fake.connects(), 1);
    // #999 L6a: every speech chunk written to this guest is timed from the
    // moment its Gemini frame was read (recorded right after the send).
    let probe = state.guest_join("de").expect("the same session");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while probe.times.peek().to_guest.count() < 3 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "3 guest sends timed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let to_guest = probe.times.peek().to_guest;
    assert!(
        to_guest.max() < Some(1_000_000),
        "a loopback send takes well under a second: {to_guest:?}"
    );
    drop(probe);
    assert_eq!(
        fake.setups()[0]["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
        "de"
    );
    let list: serde_json::Value = reqwest::get(format!("http://{addr}/listen/guest/languages"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["active"], serde_json::json!(["de"]));
    assert_eq!(list["available"], true);
}

#[tokio::test]
async fn refusals_reach_the_guest_and_the_language_list() {
    let fake = FakeGemini::new(vec![]);
    let (addr, _state) = serve(&fake, None, true).await;
    let mut client = connect(addr, "en").await;
    assert_eq!(
        next_text(&mut client).await,
        GuestWsMessage::Refused {
            reason: GuestRefusal::NoKey
        }
    );
    let list: serde_json::Value = reqwest::get(format!("http://{addr}/listen/guest/languages"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["available"], false);
    assert_eq!(list["refusal"], "noKey");
    assert_eq!(list["maxLanguages"], 4);
    assert_eq!(list["languages"][0]["name"], "English");

    let (addr, _state) = serve(&fake, Some("test-key-not-real"), false).await;
    let mut client = connect(addr, "en").await;
    assert_eq!(
        next_text(&mut client).await,
        GuestWsMessage::Refused {
            reason: GuestRefusal::NoSource
        }
    );
    let mut client = connect(addr, "xx").await;
    assert_eq!(
        next_text(&mut client).await,
        GuestWsMessage::Refused {
            reason: GuestRefusal::UnknownLanguage
        }
    );
    assert_eq!(fake.connects(), 0, "a refusal never reaches Gemini");
}

#[tokio::test]
async fn the_operator_gets_the_qr_code_of_the_guest_host() {
    let fake = FakeGemini::new(vec![]);
    let (addr, _state) = serve(&fake, None, false).await;
    let qr: serde_json::Value = reqwest::get(format!("http://{addr}/listen/guest/qr"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(qr["url"], "https://listen.example.org/");
    assert!(qr["svg"].as_str().unwrap().contains("<svg"));
    assert!(qr_svg("https://listen.example.org/")
        .unwrap()
        .starts_with("<?xml"));
    assert!(!SERVICE_WORKER.contains("\"fetch\"") && !SERVICE_WORKER.contains("caches."));
}

// --- the guest listener's boundary (#999 arch review: fail closed) ---

async fn guest_get(app: &Router, path: &str) -> (StatusCode, String) {
    use tower::ServiceExt;
    let request = axum::http::Request::get(path)
        .header("host", "listen.example.org")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn guest_state() -> AppState {
    let state = AppState::in_memory().await.unwrap();
    let fake = FakeGemini::new(vec![]);
    state.set_guest_translate(guest(&fake, Some("test-key-not-real"), DayMeter::default()));
    state
}

#[tokio::test]
async fn the_guest_listener_serves_only_the_guest_surface() {
    let app = crate::router::build_guest_router(guest_state().await);
    let shell = super::super::wasm_ui::wasm_ui_shell().await;
    let shell_status = shell.status();
    let shell_body = String::from_utf8_lossy(
        &axum::body::to_bytes(shell.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    assert_eq!(
        guest_get(&app, "/").await,
        (shell_status, shell_body),
        "the root is the guest page"
    );
    let (status, body) = guest_get(&app, "/manifest.json").await;
    assert_eq!(status, StatusCode::OK);
    let manifest: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (manifest["start_url"].as_str(), manifest["scope"].as_str()),
        (Some("/"), Some("/"))
    );
    for path in [
        "/sw.js",
        "/icon-192.png",
        "/icon-512.png",
        "/healthz",
        "/listen/guest/languages",
    ] {
        assert_eq!(guest_get(&app, path).await.0, StatusCode::OK, "{path}");
    }
    for path in [
        "/ui/operator",
        "/ui/settings",
        "/settings",
        "/listen/status",
        "/listen/mics",
        "/listen/guest/qr",
        "/ui/guest-qr",
        "/bible/presentations",
        "/libraries/summary",
        "/stage",
        "/live/ws",
        "/nothing",
    ] {
        assert_eq!(
            guest_get(&app, path).await,
            (StatusCode::NOT_FOUND, GUEST_NOT_FOUND.to_string()),
            "{path}"
        );
    }
    // a write is never answered either (only GET routes exist)
    let post = axum::http::Request::post("/listen/mics")
        .body(axum::body::Body::from("[]"))
        .unwrap();
    use tower::ServiceExt;
    assert_ne!(
        app.clone().oneshot(post).await.unwrap().status(),
        StatusCode::OK
    );
    // the guest router's own path parameter (never a 500)
    assert_eq!(
        guest_get(&app, "/ui-pkg/no-such-asset.js").await,
        (StatusCode::NOT_FOUND, "Asset not found".to_string())
    );
    // the public health shows the version, never NDI or the AI backend
    let health: serde_json::Value =
        serde_json::from_str(&guest_get(&app, "/healthz").await.1).unwrap();
    assert_eq!(health["status"], "ok");
    assert!(health["version"].is_string());
    assert!(
        health.get("ndi_pipelines").is_none() && health.get("ai").is_none(),
        "{health}"
    );
}

#[tokio::test]
async fn the_main_port_never_serves_the_guest_page() {
    let app = crate::router::build_router(guest_state().await);
    let (status, home) = guest_get(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    let shell = super::super::wasm_ui::wasm_ui_shell().await;
    let shell_body = String::from_utf8_lossy(
        &axum::body::to_bytes(shell.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    assert_ne!(
        home, shell_body,
        "even with the guest Host the main port's root is the home page"
    );
    assert_eq!(
        guest_get(&app, "/manifest.json").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        guest_get(&app, "/listen/status").await.0,
        StatusCode::OK,
        "the main port is the whole app"
    );
}

#[tokio::test]
async fn the_guest_listener_streams_over_a_real_socket() {
    let fake = FakeGemini::new(vec![]);
    let state = AppState::in_memory().await.unwrap();
    state.set_guest_translate(guest(&fake, Some("test-key-not-real"), DayMeter::default()));
    state.listen_service().set_dante_mics(true);
    let app = crate::router::build_guest_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut client = connect(addr, "hu").await;
    assert!(matches!(
        next_text(&mut client).await,
        GuestWsMessage::Hello { .. }
    ));
    let status = reqwest::get(format!("http://{addr}/ui/operator"))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_guest_listener_serves_a_favicon() {
    // SNV 27.11.2026: every browser asks for /favicon.ico; a 404 there is a
    // console error on the guest page.
    use tower::ServiceExt;
    let app = crate::router::build_guest_router(guest_state().await);
    let request = axum::http::Request::get("/favicon.ico")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
}
