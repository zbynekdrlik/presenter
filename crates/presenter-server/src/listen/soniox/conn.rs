//! One Soniox WebSocket connection as a task (#852 C3-5, plan Task 5.3):
//! connect within 10 s → [`Report::Connected`] / [`Report::ConnectFailed`]
//! (with the HTTP status of a refused upgrade); then it writes the session's
//! commands in order (audio as BINARY s16 little-endian frames, the config
//! and the controls as TEXT, the end as an EMPTY text frame) and turns every
//! TEXT frame from Soniox into a [`Report::Response`]; a close or a read /
//! write error ends it with ONE [`Report::Closed`]. It never sends a
//! WebSocket ping (Soniox does not answer them, spec §22.4); tungstenite
//! answers a ping from the server by itself.
//!
//! The transport is a seam ([`Connector`]): production opens TCP, the tests
//! a `tokio::io::duplex` pipe; both go through the one [`handshake`], so the
//! upgrade headers the tests read are the ones production sends.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, USER_AGENT};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::config::SonioxEnv;
use super::machine::Report;
use super::wire::{control_message, parse_response, Control};
use crate::listen::MicFrame;

/// The longest wait for the TCP + TLS + upgrade of one connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Our own User-Agent: Cloudflare in front of Soniox refuses a default
/// library agent (songplayer `dubbing-eval.md`, error 1010).
const USER_AGENT_VALUE: &str = concat!("presenter-listen/", env!("CARGO_PKG_VERSION"));
const TARGET: &str = "presenter::listen::asr";

/// A byte stream a connection runs over.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type WsStream = WebSocketStream<MaybeTlsStream<Box<dyn Io>>>;

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no Soniox API key")]
    NoKey,
    #[error("invalid Soniox URL: {0}")]
    Url(String),
    #[error("the API key is not a valid header value")]
    Header,
    #[error("Soniox refused the upgrade with HTTP {0}")]
    Refused(u16),
    #[error("no connection within {} s", CONNECT_TIMEOUT.as_secs())]
    Timeout,
    #[error("{0}")]
    Transport(String),
}

impl ConnectError {
    /// The HTTP status of a refused upgrade (401 / 402 / 403 / 400 stop).
    /// A URL or key that cannot make a request is our config error, never
    /// a transient one: 400, so the session stops instead of retrying.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Refused(status) => Some(*status),
            Self::Url(_) | Self::Header => Some(400),
            Self::NoKey | Self::Timeout | Self::Transport(_) => None,
        }
    }
}

// `#[async_trait]` marks the method `#[must_use]`, and the boxed future it
// returns is already must-use — clippy::double_must_use (as `AdbRunner`).
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self, req: Request) -> Result<WsStream, ConnectError>;
}

/// Production: TCP to the URL's host, TLS for `wss://` (rustls, webpki
/// roots), the upgrade.
pub struct TlsConnector;

#[async_trait]
impl Connector for TlsConnector {
    async fn connect(&self, req: Request) -> Result<WsStream, ConnectError> {
        let uri = req.uri();
        let host = uri
            .host()
            .ok_or_else(|| ConnectError::Url("no host".to_string()))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let secure = uri.scheme_str() == Some("wss");
        let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| ConnectError::Transport(e.to_string()))?;
        // Audio frames are small and latency matters more than packing.
        if let Err(error) = tcp.set_nodelay(true) {
            tracing::debug!(target: TARGET, error = %error, "TCP_NODELAY not set");
        }
        handshake(req, Box::new(tcp)).await
    }
}

/// The WebSocket upgrade over `io` (TLS first when the URL is `wss://`).
pub async fn handshake(req: Request, io: Box<dyn Io>) -> Result<WsStream, ConnectError> {
    match tokio_tungstenite::client_async_tls_with_config(req, io, None, None).await {
        Ok((ws, _response)) => Ok(ws),
        Err(WsError::Http(response)) => Err(ConnectError::Refused(response.status().as_u16())),
        Err(error) => Err(ConnectError::Transport(error.to_string())),
    }
}

/// The upgrade request: the key ONLY in `Authorization: Bearer` (marked
/// sensitive), our own User-Agent. Built without logging any header.
pub fn request(env: &SonioxEnv) -> Result<Request, ConnectError> {
    let key = env.key.as_deref().ok_or(ConnectError::NoKey)?;
    let mut req = env
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| ConnectError::Url(e.to_string()))?;
    let mut auth =
        HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| ConnectError::Header)?;
    auth.set_sensitive(true);
    let headers = req.headers_mut();
    headers.insert(AUTHORIZATION, auth);
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    Ok(req)
}

/// What the session asks a connection to write, in order.
#[derive(Debug, Clone)]
pub enum ConnCommand {
    /// The config JSON: the first frame of every connection.
    Config(String),
    Audio(MicFrame),
    Control(Control),
    /// The empty TEXT frame: Soniox answers the rest, `finished`, closes.
    End,
}

/// Spawns connection `conn`. Commands are unbounded on purpose: a socket
/// that stops taking writes must not stall the session task (whose stall
/// rule drops it after 15 s, bounding the queue). Dropping the sender ends
/// the task without a report; the abort handle kills it at once.
pub fn spawn_connection(
    conn: u32,
    connector: Arc<dyn Connector>,
    req: Request,
    reports: mpsc::Sender<Report>,
) -> (mpsc::UnboundedSender<ConnCommand>, AbortHandle) {
    let (commands, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_connection(conn, connector, req, rx, reports));
    (commands, task.abort_handle())
}

async fn run_connection(
    conn: u32,
    connector: Arc<dyn Connector>,
    req: Request,
    commands: mpsc::UnboundedReceiver<ConnCommand>,
    reports: mpsc::Sender<Report>,
) {
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(req)).await;
    let ws = match connected.unwrap_or(Err(ConnectError::Timeout)) {
        Ok(ws) => ws,
        Err(error) => {
            let failed = Report::ConnectFailed {
                conn,
                status: error.status(),
                error: error.to_string(),
                at: now(),
            };
            // A send error = the session is gone; nothing left to tell.
            let _ = reports.send(failed).await;
            return;
        }
    };
    if reports
        .send(Report::Connected { conn, at: now() })
        .await
        .is_err()
    {
        return;
    }
    if let Some(end) = pump(conn, ws, commands, &reports).await {
        let _ = reports.send(end).await;
    }
}

/// Writes commands and reads frames until the connection ends; returns the
/// closing report (`None` = the session dropped the connection).
async fn pump(
    conn: u32,
    mut ws: WsStream,
    mut commands: mpsc::UnboundedReceiver<ConnCommand>,
    reports: &mpsc::Sender<Report>,
) -> Option<Report> {
    let mut unreadable = 0_u64;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let command = command?;
                if let Err(error) = ws.send(to_message(command)).await {
                    return Some(closed(conn, None, error.to_string()));
                }
            }
            frame = ws.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    if let Some(report) = response(conn, text.as_str(), &mut unreadable) {
                        reports.send(report).await.ok()?;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    tracing::debug!(target: TARGET, conn, bytes = bytes.len(), "binary frame from Soniox ignored");
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame.map_or((None, String::new()), |f| {
                        (Some(u16::from(f.code)), f.reason.to_string())
                    });
                    // Best effort: send tungstenite's queued close reply.
                    let _ = ws.flush().await;
                    return Some(closed(conn, code, reason));
                }
                // Ping / pong: tungstenite answers a server ping itself.
                Some(Ok(_)) => {}
                Some(Err(error)) => return Some(closed(conn, None, error.to_string())),
                None => return Some(closed(conn, None, "the stream ended".to_string())),
            },
        }
    }
}

fn response(conn: u32, text: &str, unreadable: &mut u64) -> Option<Report> {
    match parse_response(text) {
        Ok(resp) => Some(Report::Response {
            conn,
            resp,
            at: now(),
            received_at: Utc::now(),
        }),
        Err(error) => {
            // Never Soniox's normal output; its text is not logged (size
            // only); WARN on power-of-two counts (#484 gate).
            *unreadable += 1;
            if crate::listen::should_log_event(*unreadable) {
                tracing::warn!(target: TARGET, conn, bytes = text.len(), count = *unreadable, error = %error, "unreadable Soniox response ignored");
            }
            None
        }
    }
}

fn to_message(command: ConnCommand) -> Message {
    match command {
        ConnCommand::Config(json) => Message::text(json),
        ConnCommand::Audio(pcm) => Message::binary(pcm_le_bytes(&pcm)),
        ConnCommand::Control(control) => Message::text(control_message(control)),
        ConnCommand::End => Message::text(String::new()),
    }
}

/// `pcm_s16le`: every sample as two little-endian bytes.
pub fn pcm_le_bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

fn closed(conn: u32, code: Option<u16>, reason: String) -> Report {
    Report::Closed {
        conn,
        code,
        reason,
        at: now(),
    }
}

/// The machine's clock: tokio's, so a paused test clock drives it too.
pub fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}
