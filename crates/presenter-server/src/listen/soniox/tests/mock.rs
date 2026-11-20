//! A scripted Soniox server over `tokio::io::duplex` (#852 C3-5, plan Task
//! 5.5): every connection the session opens takes the next script, the
//! upgrade headers are kept, and every frame the client sends is logged
//! (text / binary / ping) with the paused clock's time.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::io::DuplexStream;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as UpgradeRequest, Response as UpgradeResponse,
};
use tokio_tungstenite::tungstenite::http::{HeaderMap, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use tokio_tungstenite::WebSocketStream;

use super::super::conn::{handshake, ConnectError, Connector, WsStream};

/// One step of a connection's script.
#[derive(Debug, Clone)]
pub enum Step {
    /// Refuse the upgrade with this HTTP status (must be the first step).
    RefuseUpgrade(u16),
    /// Read the first frame (the config).
    ExpectConfig,
    /// Read until this much audio arrived on the connection.
    ExpectAudio {
        ms: u64,
    },
    Reply(String),
    /// Read until a `finalize` arrives, then reply.
    ReplyOnFinalize(String),
    /// Read until the empty TEXT frame arrives.
    ExpectEnd,
    Wait(Duration),
    Close(u16),
    /// End the stream without a close frame (an abnormal close, 1006).
    Drop,
    /// An error response (`error_code`, `error_type`), then a close.
    Error(u16, &'static str),
    /// Read and log, never answer.
    GoSilent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close,
}

#[derive(Debug, Clone)]
pub struct Logged {
    /// 0-based index of the connection.
    pub conn: usize,
    pub at: Instant,
    pub frame: Frame,
}

#[derive(Default)]
pub struct MockSoniox {
    scripts: Mutex<VecDeque<Vec<Step>>>,
    log: Mutex<Vec<Logged>>,
    headers: Mutex<Vec<HeaderMap>>,
    connects: AtomicUsize,
}

impl MockSoniox {
    pub fn new(scripts: Vec<Vec<Step>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            ..Self::default()
        })
    }

    pub fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }

    pub fn log(&self) -> Vec<Logged> {
        self.log.lock().unwrap().clone()
    }

    pub fn headers(&self) -> Vec<HeaderMap> {
        self.headers.lock().unwrap().clone()
    }

    /// Ping frames the client sent over every connection.
    pub fn pings(&self) -> usize {
        self.log().iter().filter(|l| l.frame == Frame::Ping).count()
    }

    /// The TEXT frames of connection `conn`, with their times.
    pub fn texts(&self, conn: usize) -> Vec<(Instant, String)> {
        self.log()
            .into_iter()
            .filter(|l| l.conn == conn)
            .filter_map(|l| match l.frame {
                Frame::Text(text) => Some((l.at, text)),
                _ => None,
            })
            .collect()
    }

    /// The BINARY frames of connection `conn`, with their times.
    pub fn audio(&self, conn: usize) -> Vec<(Instant, Vec<u8>)> {
        self.log()
            .into_iter()
            .filter(|l| l.conn == conn)
            .filter_map(|l| match l.frame {
                Frame::Binary(bytes) => Some((l.at, bytes)),
                _ => None,
            })
            .collect()
    }
}

/// Hands the client end of a fresh duplex pipe to the session; the server
/// end runs the next script.
pub struct DuplexConnector(pub Arc<MockSoniox>);

#[async_trait]
impl Connector for DuplexConnector {
    async fn connect(&self, req: Request) -> Result<WsStream, ConnectError> {
        let conn = self.0.connects.fetch_add(1, Ordering::SeqCst);
        let steps = self
            .0
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![Step::ExpectConfig, Step::GoSilent]);
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(serve(Arc::clone(&self.0), conn, server, steps));
        handshake(req, Box::new(client)).await
    }
}

async fn serve(mock: Arc<MockSoniox>, conn: usize, io: DuplexStream, steps: Vec<Step>) {
    let refuse = match steps.first() {
        Some(Step::RefuseUpgrade(status)) => Some(*status),
        _ => None,
    };
    let headers = Arc::clone(&mock);
    // tungstenite's `Callback` fixes the error type (an http response).
    #[allow(clippy::result_large_err)]
    let callback = move |req: &UpgradeRequest, resp: UpgradeResponse| {
        headers.headers.lock().unwrap().push(req.headers().clone());
        match refuse {
            Some(status) => {
                let mut refused = ErrorResponse::new(Some("refused".to_string()));
                *refused.status_mut() = StatusCode::from_u16(status).unwrap();
                Err(refused)
            }
            None => Ok(resp),
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(io, callback).await else {
        return;
    };
    let mut server = Server {
        mock,
        conn,
        ws,
        audio_bytes: 0,
    };
    for step in steps {
        if !server.step(step).await {
            return;
        }
    }
    while server.read().await.is_some() {}
}

struct Server {
    mock: Arc<MockSoniox>,
    conn: usize,
    ws: WebSocketStream<DuplexStream>,
    audio_bytes: u64,
}

impl Server {
    /// `false` = the connection is over.
    async fn step(&mut self, step: Step) -> bool {
        match step {
            Step::RefuseUpgrade(_) => true,
            Step::ExpectConfig => self.read().await.is_some(),
            Step::ExpectAudio { ms } => {
                while self.audio_bytes < ms * 32 {
                    if self.read().await.is_none() {
                        return false;
                    }
                }
                true
            }
            Step::Reply(json) => self.ws.send(Message::text(json)).await.is_ok(),
            Step::ReplyOnFinalize(json) => {
                self.read_until(|f| matches!(f, Frame::Text(t) if t.contains("finalize")))
                    .await
                    && self.ws.send(Message::text(json)).await.is_ok()
            }
            Step::ExpectEnd => {
                self.read_until(|f| matches!(f, Frame::Text(t) if t.is_empty()))
                    .await
            }
            Step::Wait(d) => {
                tokio::time::sleep(d).await;
                true
            }
            Step::Close(code) => self.close(code).await,
            Step::Drop => false,
            Step::Error(code, kind) => {
                let json = format!(
                    r#"{{"error_code":{code},"error_type":"{kind}","error_message":"test"}}"#
                );
                self.ws.send(Message::text(json)).await.is_ok() && self.close(1000).await
            }
            Step::GoSilent => true,
        }
    }

    async fn close(&mut self, code: u16) -> bool {
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: "".into(),
        };
        self.ws.close(Some(frame)).await.is_ok()
    }

    async fn read_until(&mut self, done: impl Fn(&Frame) -> bool) -> bool {
        while let Some(frame) = self.read().await {
            if done(&frame) {
                return true;
            }
        }
        false
    }

    /// The next frame from the client, logged; `None` = the stream ended.
    async fn read(&mut self) -> Option<Frame> {
        let frame = match self.ws.next().await? {
            Ok(Message::Text(text)) => Frame::Text(text.to_string()),
            Ok(Message::Binary(bytes)) => {
                self.audio_bytes += bytes.len() as u64;
                Frame::Binary(bytes.to_vec())
            }
            Ok(Message::Ping(_)) => Frame::Ping,
            Ok(Message::Pong(_)) => Frame::Pong,
            Ok(Message::Close(_)) => Frame::Close,
            Ok(Message::Frame(_)) | Err(_) => return None,
        };
        self.mock.log.lock().unwrap().push(Logged {
            conn: self.conn,
            at: Instant::now(),
            frame: frame.clone(),
        });
        Some(frame)
    }
}
