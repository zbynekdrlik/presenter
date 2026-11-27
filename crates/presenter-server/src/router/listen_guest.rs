//! The guests' live translation (#999 L2/L3). On the guests' OWN hostname
//! (`PRESENTER_GUEST_HOST`, ROZHODNUTÉ 27.11) [`guest_routes`] is the whole
//! surface (`guest_host.rs` dispatches by `Host`): `/` = the WASM shell (the
//! page is `presenter-ui`'s `pages::guest`), its manifest, service worker,
//! icons and `/ui-pkg` assets, `/healthz` (the version), the language list
//! `GET /listen/guest/languages` and the stream `GET /listen/guest/ws?lang=xx`.
//! The normal hosts ([`routes`]) carry the same two guest endpoints and the
//! operator's QR page `/ui/guest-qr` (its code from `GET /listen/guest/qr`,
//! encoding `https://<guest host>/`).
//!
//! The stream (core `listen::guest::GuestWsMessage`): ONE `hello`, the
//! session's `state` (again on every change), then the translated speech as
//! BINARY s16le 24 kHz chunks and the text as `text` / `original` / `turn`
//! messages; a refusal is one `refused` message and a close. A WS Ping every [`KEEPALIVE`] keeps Cloudflare's idle
//! cut away; a client silent for two intervals is dropped.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, MissedTickBehavior};

use super::integrations::extract_client_ip;
use crate::listen::translate::wire::OUTPUT_RATE;
use crate::listen::translate::{GuestPacket, Listener, TARGET};
use crate::live::{close_frame_reason, WsEndReason};
use crate::state::AppState;
use presenter_core::listen::guest::{GuestLanguages, GuestWsMessage};

const SERVICE_WORKER: &str = include_str!("../ui/guest/sw.js");
const ICON_192: &[u8] = include_bytes!("../ui/guest/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../ui/guest/icon-512.png");

const KEEPALIVE_PRODUCTION: Duration = Duration::from_secs(30);
const _: () = assert!(KEEPALIVE_PRODUCTION.as_secs() * 2 < 100);
#[cfg(not(test))]
const KEEPALIVE: Duration = KEEPALIVE_PRODUCTION;
#[cfg(test)]
const KEEPALIVE: Duration = Duration::from_secs(1);
/// Revalidated on every load, so a deploy reaches every guest at once.
const NO_CACHE: &str = "no-cache";

/// The normal hosts: the guest endpoints (an operator can try them) and the
/// operator's QR page. The guest PAGE is never served here.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/ui/guest-qr", get(super::wasm_ui::wasm_ui_shell))
        .route("/listen/guest/qr", get(qr))
        .route("/listen/guest/languages", get(languages))
        .route("/listen/guest/ws", get(guest_websocket))
}

/// Everything the guest hostname serves; any other path is a 404 (that host
/// has no login in front of it).
pub(super) fn guest_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(super::wasm_ui::wasm_ui_shell))
        .route("/sw.js", get(|| async { js(SERVICE_WORKER) }))
        .route("/manifest.json", get(manifest))
        .route("/icon-192.png", get(|| async { png(ICON_192) }))
        .route("/icon-512.png", get(|| async { png(ICON_512) }))
        .route("/ui-pkg/{*path}", get(super::wasm_ui::wasm_ui_asset))
        .route("/healthz", get(super::health))
        .route("/listen/guest/languages", get(languages))
        .route("/listen/guest/ws", get(guest_websocket))
}

fn no_cache() -> [(header::HeaderName, &'static str); 1] {
    [(header::CACHE_CONTROL, NO_CACHE)]
}

fn js(body: &'static str) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        body,
    )
        .into_response()
}

fn png(bytes: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, "image/png")], bytes).into_response()
}

async fn manifest() -> impl IntoResponse {
    let manifest = serde_json::json!({
        "id": "/",
        "name": "Sermon translation",
        "short_name": "Translation",
        "description": "The Slovak sermon, live in your language",
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "orientation": "portrait",
        "background_color": "#0f172a",
        "theme_color": "#0f172a",
        "icons": [
            { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
            { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
            { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
        ]
    });
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        Json(manifest),
    )
}

/// `GET /listen/guest/languages`: what the page offers and whether it works.
async fn languages(State(state): State<AppState>) -> impl IntoResponse {
    let status = state.guest_status();
    let active = status
        .active
        .iter()
        .map(|code| (*code).to_string())
        .collect();
    (
        no_cache(),
        Json(GuestLanguages::offered(
            status.refusal,
            active,
            status.max_languages,
        )),
    )
}

/// `GET /listen/guest/qr` (operator): the guest page's address
/// `https://<guest host>/` and its QR code (an SVG the `/ui/guest-qr` page
/// shows big, for the operator to put on a screen); `url: null` when this
/// box has no guest hostname.
async fn qr(State(state): State<AppState>) -> Response {
    let Some(host) = state.guest_host() else {
        return (
            no_cache(),
            Json(serde_json::json!({ "url": null, "svg": null })),
        )
            .into_response();
    };
    let url = format!("https://{host}/");
    match qr_svg(&url) {
        Ok(svg) => (
            no_cache(),
            Json(serde_json::json!({ "url": url, "svg": svg })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(target: TARGET, url = url.as_str(), error = %error, "QR code not made");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "QR kód sa nepodarilo vytvoriť",
            )
                .into_response()
        }
    }
}

pub(super) fn qr_svg(url: &str) -> Result<String, qrcode::types::QrError> {
    Ok(qrcode::QrCode::new(url.as_bytes())?
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build())
}

#[derive(Debug, Deserialize)]
pub(super) struct GuestQuery {
    #[serde(default)]
    lang: String,
}

async fn guest_websocket(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<GuestQuery>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let peer = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let client_ip = extract_client_ip(&headers, peer);
    let lang = query.lang.trim().to_ascii_lowercase();
    ws.on_upgrade(move |socket| serve_guest(state, socket, lang, client_ip))
}

async fn serve_guest(state: AppState, mut socket: WebSocket, lang: String, client_ip: String) {
    let mut listener = match state.guest_join(&lang) {
        Ok(listener) => listener,
        Err(refusal) => {
            tracing::info!(target: TARGET, client_ip = client_ip.as_str(), language = lang.as_str(), refusal = ?refusal, "guest stream refused");
            let _ = socket
                .send(text(&GuestWsMessage::Refused { reason: refusal }))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    tracing::info!(target: TARGET, client_ip = client_ip.as_str(), language = lang.as_str(), "guest stream connected");
    let connected_at = Instant::now();
    let mut stats = StreamStats::default();
    let reason = forward(&mut socket, &mut listener, &mut stats).await;
    tracing::info!(
        target: TARGET,
        client_ip = client_ip.as_str(),
        language = lang.as_str(),
        reason = reason.to_string().as_str(),
        audio_chunks = stats.audio_chunks,
        skipped_packets = stats.skipped,
        connected_ms = u64::try_from(connected_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        "guest stream disconnected"
    );
}

#[derive(Default)]
struct StreamStats {
    audio_chunks: u64,
    skipped: u64,
}

fn text(message: &GuestWsMessage) -> Message {
    Message::text(serde_json::to_string(message).unwrap_or_default())
}

fn packet_message(packet: GuestPacket) -> Message {
    match packet {
        GuestPacket::Audio(pcm) => Message::Binary(pcm.to_vec().into()),
        GuestPacket::Text(text) => self::text(&GuestWsMessage::Text { text }),
        GuestPacket::Original(text) => self::text(&GuestWsMessage::Original { text }),
        GuestPacket::TurnComplete => self::text(&GuestWsMessage::Turn),
    }
}

async fn forward(
    socket: &mut WebSocket,
    listener: &mut Listener,
    stats: &mut StreamStats,
) -> WsEndReason {
    let hello = GuestWsMessage::Hello {
        language: listener.language.code.to_string(),
        name: listener.language.name.to_string(),
        sample_rate: OUTPUT_RATE,
    };
    if let Err(err) = socket.send(text(&hello)).await {
        return WsEndReason::SendFailed(err.to_string());
    }
    let current = listener.state.borrow_and_update().clone();
    if let Err(err) = socket
        .send(text(&GuestWsMessage::State { session: current }))
        .await
    {
        return WsEndReason::SendFailed(err.to_string());
    }
    let mut last_heard = Instant::now();
    let mut keepalive = tokio::time::interval_at(last_heard + KEEPALIVE, KEEPALIVE);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            packet = listener.packets.recv() => match packet {
                Ok(packet) => {
                    if matches!(packet, GuestPacket::Audio(_)) {
                        stats.audio_chunks += 1;
                    }
                    if let Err(err) = socket.send(packet_message(packet)).await {
                        return WsEndReason::SendFailed(err.to_string());
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    stats.skipped += missed;
                    tracing::info!(target: TARGET, language = listener.language.code, missed, "guest stream fell behind — packets skipped");
                }
                Err(RecvError::Closed) => return WsEndReason::HubClosed,
            },
            changed = listener.state.changed() => {
                if changed.is_err() {
                    return WsEndReason::HubClosed;
                }
                let current = listener.state.borrow_and_update().clone();
                if let Err(err) = socket.send(text(&GuestWsMessage::State { session: current })).await {
                    return WsEndReason::SendFailed(err.to_string());
                }
            }
            incoming = socket.recv() => match incoming {
                None => return WsEndReason::StreamEnded,
                Some(Err(err)) => return WsEndReason::ReadError(err.to_string()),
                Some(Ok(Message::Close(frame))) => return close_frame_reason(frame),
                Some(Ok(_)) => last_heard = Instant::now(),
            },
            _ = keepalive.tick() => {
                if last_heard.elapsed() >= 2 * KEEPALIVE {
                    return WsEndReason::ClientSilent;
                }
                if let Err(err) = socket.send(Message::Ping(Default::default())).await {
                    return WsEndReason::SendFailed(err.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
