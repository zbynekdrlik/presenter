//! One Gemini Live connection as a task (#999 L1): connect within
//! [`CONNECT_TIMEOUT`] (the Soniox transport seam: TLS in production, a duplex
//! pipe in tests), send the setup, then write the session's messages in order
//! and turn every frame from Gemini into a [`ConnEvent::Message`]; a close or a
//! read / write error ends it with ONE [`ConnEvent::Closed`]. Never a client
//! ping (tungstenite answers the server's).

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::header::USER_AGENT;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::Message;

use super::config::GeminiEnv;
use super::wire::{parse_server, ServerMessage};
use super::TARGET;
use crate::listen::soniox::conn::{ConnectError, Connector, WsStream, CONNECT_TIMEOUT};

/// The header the Google API reads the key from (the SDK's own; never the
/// `?key=` query, which would put the key into every URL log).
const API_KEY_HEADER: &str = "x-goog-api-key";
const USER_AGENT_VALUE: &str = concat!("presenter-listen/", env!("CARGO_PKG_VERSION"));

/// What a connection tells its session.
#[derive(Debug)]
pub enum ConnEvent {
    /// The upgrade failed (`status` = the HTTP status of a refusal).
    Failed {
        conn: u32,
        status: Option<u16>,
        error: String,
    },
    Message {
        conn: u32,
        message: ServerMessage,
    },
    Closed {
        conn: u32,
        code: Option<u16>,
        reason: String,
    },
}

/// The upgrade request: the key ONLY in `x-goog-api-key` (marked sensitive).
pub fn request(env: &GeminiEnv) -> Result<Request, ConnectError> {
    let key = env.key.as_deref().ok_or(ConnectError::NoKey)?;
    let mut req = env
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| ConnectError::Url(e.to_string()))?;
    let mut value = HeaderValue::from_str(key).map_err(|_| ConnectError::Header)?;
    value.set_sensitive(true);
    let headers = req.headers_mut();
    headers.insert(HeaderName::from_static(API_KEY_HEADER), value);
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    Ok(req)
}

/// Spawns connection `conn`; `setup` is its first frame. The message channel is
/// unbounded: a stuck socket must not stall the session (its `NO_ANSWER`
/// rule drops a connection that answers nothing for 10 s while audio goes
/// out, which bounds the queue). Dropping the sender ends the task.
pub fn spawn(
    conn: u32,
    connector: Arc<dyn Connector>,
    req: Request,
    setup: String,
    events: mpsc::Sender<ConnEvent>,
) -> (mpsc::UnboundedSender<String>, AbortHandle) {
    let (messages, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(conn, connector, req, setup, rx, events));
    (messages, task.abort_handle())
}

async fn run(
    conn: u32,
    connector: Arc<dyn Connector>,
    req: Request,
    setup: String,
    messages: mpsc::UnboundedReceiver<String>,
    events: mpsc::Sender<ConnEvent>,
) {
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(req)).await;
    let mut ws = match connected.unwrap_or(Err(ConnectError::Timeout)) {
        Ok(ws) => ws,
        Err(error) => {
            let failed = ConnEvent::Failed {
                conn,
                status: error.status(),
                error: error.to_string(),
            };
            // A send error = the session is gone; nothing left to tell.
            let _ = events.send(failed).await;
            return;
        }
    };
    if let Err(error) = ws.send(Message::text(setup)).await {
        let _ = events.send(closed(conn, None, error.to_string())).await;
        return;
    }
    if let Some(end) = pump(conn, ws, messages, &events).await {
        let _ = events.send(end).await;
    }
}

/// Writes messages and reads frames until the connection ends; returns the
/// closing event (`None` = the session dropped the connection).
async fn pump(
    conn: u32,
    mut ws: WsStream,
    mut messages: mpsc::UnboundedReceiver<String>,
    events: &mpsc::Sender<ConnEvent>,
) -> Option<ConnEvent> {
    let mut unreadable = 0_u64;
    loop {
        tokio::select! {
            message = messages.recv() => {
                let message = message?;
                if let Err(error) = ws.send(Message::text(message)).await {
                    return Some(closed(conn, None, error.to_string()));
                }
            }
            frame = ws.next() => match frame {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Some(event) = read(conn, &bytes, &mut unreadable) {
                        events.send(event).await.ok()?;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Some(event) = read(conn, text.as_bytes(), &mut unreadable) {
                        events.send(event).await.ok()?;
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame.map_or((None, String::new()), |f| {
                        (Some(u16::from(f.code)), f.reason.to_string())
                    });
                    let _ = ws.flush().await;
                    return Some(closed(conn, code, reason));
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Some(closed(conn, None, error.to_string())),
                None => return Some(closed(conn, None, "the stream ended".to_string())),
            },
        }
    }
}

fn read(conn: u32, bytes: &[u8], unreadable: &mut u64) -> Option<ConnEvent> {
    match parse_server(bytes) {
        Ok(message) => Some(ConnEvent::Message { conn, message }),
        Err(error) => {
            *unreadable += 1;
            if crate::listen::should_log_event(*unreadable) {
                tracing::warn!(target: TARGET, conn, bytes = bytes.len(), count = *unreadable, error = %error, "unreadable Gemini message ignored");
            }
            None
        }
    }
}

fn closed(conn: u32, code: Option<u16>, reason: String) -> ConnEvent {
    ConnEvent::Closed { conn, code, reason }
}
