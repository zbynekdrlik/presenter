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

fn voiced_frame() -> crate::listen::MicFrame {
    (0..1_600)
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
            name: "Deutsch".into(),
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
    let frames = state.listen_service().frame_sender("dante-mic-1");
    let mut speech = 0;
    let mut texts = Vec::new();
    for _ in 0..3 {
        let _ = frames.send(voiced_frame());
        tokio::time::sleep(Duration::from_millis(30)).await;
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
