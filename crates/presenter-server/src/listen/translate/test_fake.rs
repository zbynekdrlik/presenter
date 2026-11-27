//! A scripted fake Gemini Live server over `tokio::io::duplex` (#999 L1),
//! shared by the session tests and the guest stream's router tests: every
//! connection takes the next [`Script`], the upgrade headers and the setup are
//! kept, every audio message is counted and answered with 100 ms of 24 kHz
//! audio, a text fragment and a new resumption handle.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use tokio::io::DuplexStream;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::handshake::server::{
    Request as UpgradeRequest, Response as UpgradeResponse,
};
use tokio_tungstenite::tungstenite::http::HeaderMap;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};

use super::config::GeminiEnv;
use super::session::SessionDeps;
use super::stats::DayMeter;
use super::GuestTranslate;
use crate::listen::soniox::conn::{handshake, ConnectError, Connector, WsStream};

/// What one fake connection does.
#[derive(Debug, Clone, Default)]
pub(crate) struct Script {
    /// Close right after the setup with this code + reason (a refused key).
    pub(crate) close_on_setup: Option<(u16, &'static str)>,
    /// After this many audio messages, send a `goAway`.
    pub(crate) go_away_after: Option<usize>,
    /// After this many audio messages, close with code + reason.
    pub(crate) close_after: Option<(usize, u16, &'static str)>,
}

#[derive(Default)]
pub(crate) struct FakeGemini {
    scripts: Mutex<VecDeque<Script>>,
    connects: AtomicUsize,
    pub(crate) headers: Mutex<Vec<HeaderMap>>,
    setups: Mutex<Vec<serde_json::Value>>,
    /// Audio messages per connection (index = connection).
    audio: Mutex<Vec<usize>>,
}

impl FakeGemini {
    pub(crate) fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            ..Self::default()
        })
    }
    pub(crate) fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
    pub(crate) fn setups(&self) -> Vec<serde_json::Value> {
        self.setups.lock().unwrap().clone()
    }
    pub(crate) fn audio(&self) -> Vec<usize> {
        self.audio.lock().unwrap().clone()
    }
}

pub(crate) struct DuplexConnector(pub(crate) Arc<FakeGemini>);

#[async_trait]
impl Connector for DuplexConnector {
    async fn connect(&self, req: Request) -> Result<WsStream, ConnectError> {
        let conn = self.0.connects.fetch_add(1, Ordering::SeqCst);
        self.0.audio.lock().unwrap().push(0);
        let script = self
            .0
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(serve(Arc::clone(&self.0), conn, server, script));
        handshake(req, Box::new(client)).await
    }
}

/// The fake's answer to the `n`-th audio message: 100 ms of 24 kHz audio
/// whose first sample is `n`, the text `w<n> ` and a new handle.
fn answer(conn: usize, n: usize) -> Vec<Message> {
    let mut pcm = vec![0_u8; 4_800];
    pcm[..2].copy_from_slice(&i16::try_from(n).unwrap_or(i16::MAX).to_le_bytes());
    let audio = serde_json::json!({"serverContent": {"modelTurn": {"parts": [
        {"inlineData": {"mimeType": "audio/pcm;rate=24000", "data": BASE64.encode(&pcm)}}]}}});
    let text =
        serde_json::json!({"serverContent": {"outputTranscription": {"text": format!("w{n} ")}}});
    let handle = serde_json::json!({"sessionResumptionUpdate": {"newHandle": format!("h-{conn}-{n}"), "resumable": true}});
    [audio, text, handle]
        .into_iter()
        .map(|v| Message::binary(v.to_string().into_bytes()))
        .collect()
}

async fn serve(fake: Arc<FakeGemini>, conn: usize, io: DuplexStream, script: Script) {
    let headers = Arc::clone(&fake);
    #[allow(clippy::result_large_err)]
    let callback = move |req: &UpgradeRequest, resp: UpgradeResponse| {
        headers.headers.lock().unwrap().push(req.headers().clone());
        Ok(resp)
    };
    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(io, callback).await else {
        return;
    };
    let Some(Ok(Message::Text(setup))) = ws.next().await else {
        return;
    };
    fake.setups
        .lock()
        .unwrap()
        .push(serde_json::from_str(setup.as_str()).unwrap());
    if let Some((code, reason)) = script.close_on_setup {
        let _ = ws.send(close(code, reason)).await;
        return;
    }
    let _ = ws
        .send(Message::binary(b"{\"setupComplete\":{}}".to_vec()))
        .await;
    let mut n = 0;
    while let Some(Ok(frame)) = ws.next().await {
        let Message::Text(text) = frame else { continue };
        if !text.as_str().contains("realtimeInput") {
            continue;
        }
        n += 1;
        fake.audio.lock().unwrap()[conn] += 1;
        for message in answer(conn, n) {
            if ws.send(message).await.is_err() {
                return;
            }
        }
        if script.go_away_after == Some(n) {
            let away = serde_json::json!({"goAway": {"timeLeft": "10s"}});
            let _ = ws
                .send(Message::binary(away.to_string().into_bytes()))
                .await;
        }
        if let Some((after, code, reason)) = script.close_after {
            if after == n {
                let _ = ws.send(close(code, reason)).await;
                return;
            }
        }
    }
}

fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::from(code),
        reason: reason.into(),
    }))
}

/// The test settings: a fake URL, the given key, the source `dante-mic-1`.
pub(crate) fn env(key: Option<&str>) -> GeminiEnv {
    GeminiEnv {
        url: "ws://gemini.test/ws".to_string(),
        key: key.map(str::to_string),
        model: "gemini-test-translate".to_string(),
        source_mic: "dante-mic-1".to_string(),
    }
}

pub(crate) fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 11, 29).unwrap()
}

/// A registry whose sessions talk to `fake`.
pub(crate) fn guest(fake: &Arc<FakeGemini>, key: Option<&str>, meter: DayMeter) -> GuestTranslate {
    GuestTranslate::with_deps(SessionDeps {
        env: env(key),
        connector: Arc::new(DuplexConnector(Arc::clone(fake))),
        meter,
        today,
    })
}
