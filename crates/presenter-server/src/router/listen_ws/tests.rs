//! `GET /listen/ws` (#852 C3-6) through a real axum server on loopback and a
//! real `tokio_tungstenite` client (the #471 pattern: a `oneshot` request
//! cannot upgrade).

use super::*;
use crate::router::build_router;
use crate::tracing_capture::Capture;
use chrono::{DateTime, TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use presenter_core::listen::transcript::{ClosedSentence, TailText};
use presenter_core::listen::transcription::{MicTranscription, TranscriptionState};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, 5)
        .single()
        .unwrap()
}

fn closed(text: &str) -> ClosedSentence {
    ClosedSentence {
        text: text.to_string(),
        start_ms: 0,
        end_ms: 1_000,
        closed_at: at(),
        language: Some("en".into()),
    }
}

/// A served router and the hub its `/listen/ws` reads.
async fn serve() -> (SocketAddr, TranscriptHub) {
    let state = AppState::in_memory().await.unwrap();
    let hub = state.listen_transcripts();
    let app = build_router(state);
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
    (addr, hub)
}

async fn connect(addr: SocketAddr) -> Client {
    let (client, _response) = tokio_tungstenite::connect_async(format!("ws://{addr}/listen/ws"))
        .await
        .unwrap();
    client
}

/// The next JSON message (pings and the like skipped), within 5 s.
async fn next(client: &mut Client) -> ListenWsMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let WsMessage::Text(text) = frame {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

fn sentence_ids(message: &ListenWsMessage) -> Vec<u64> {
    match message {
        ListenWsMessage::Snapshot { snapshot } => snapshot.sentences.iter().map(|s| s.id).collect(),
        ListenWsMessage::Sentence { sentence } => vec![sentence.id],
        _ => Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_message_is_a_snapshot_with_the_newest_sentences() {
    let (addr, hub) = serve().await;
    for n in 0..502 {
        let _id = hub.push_sentence("guest", closed(&format!("Sentence {n}.")));
    }
    hub.set_tail(
        "guest",
        Some(TailText {
            final_text: "For God so".into(),
            partial_text: " loved".into(),
            start_ms: Some(0),
            language: None,
        }),
        at(),
    );
    hub.set_session(MicTranscription {
        mic_id: "guest".into(),
        state: TranscriptionState::Streaming,
        ..MicTranscription::default()
    });
    let mut client = connect(addr).await;
    let ListenWsMessage::Snapshot { snapshot } = next(&mut client).await else {
        panic!("the first message must be a snapshot");
    };
    assert_eq!(snapshot.sentences.len(), 500);
    assert_eq!(snapshot.sentences.first().map(|s| s.id), Some(3));
    assert_eq!(
        snapshot.sentences.last().map(|s| s.text.as_str()),
        Some("Sentence 501.")
    );
    assert_eq!(snapshot.tails.len(), 1);
    assert_eq!(snapshot.tails[0].final_text, "For God so");
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.sessions[0].state, TranscriptionState::Streaming);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentences_and_tails_follow_in_order() {
    let (addr, hub) = serve().await;
    let mut client = connect(addr).await;
    let first = next(&mut client).await;
    assert_eq!(
        first,
        ListenWsMessage::Snapshot {
            snapshot: TranscriptSnapshot::default()
        },
        "an empty hub sends an empty snapshot"
    );
    client
        .send(WsMessage::Text("ignored".into()))
        .await
        .unwrap();
    let tail = TailText {
        final_text: "Turn with me".into(),
        partial_text: " to".into(),
        start_ms: Some(0),
        language: None,
    };
    hub.set_tail("guest", Some(tail), at());
    let id = hub.push_sentence("guest", closed("Turn with me to John 3:16."));
    hub.set_tail("guest", None, at());
    let received = [
        next(&mut client).await,
        next(&mut client).await,
        next(&mut client).await,
    ];
    assert!(
        matches!(&received[0], ListenWsMessage::Tail { tail } if tail.partial_text == " to"),
        "{received:?}"
    );
    assert_eq!(sentence_ids(&received[1]), vec![id]);
    assert!(
        matches!(&received[2], ListenWsMessage::Tail { tail } if tail.final_text.is_empty()),
        "{received:?}"
    );
}

#[tokio::test]
async fn a_lagging_reader_gets_a_fresh_snapshot() {
    let capture = Capture::install();
    let (addr, hub) = serve().await;
    let mut client = connect(addr).await;
    assert_eq!(sentence_ids(&next(&mut client).await), Vec::<u64>::new());
    // Current-thread runtime: the server task cannot forward while this loop
    // runs, so its receiver falls 600 − 512 messages behind.
    for n in 1..=600 {
        let _id = hub.push_sentence("guest", closed(&format!("S{n}.")));
    }
    let resync = next(&mut client).await;
    assert!(
        matches!(resync, ListenWsMessage::Snapshot { .. }),
        "{resync:?}"
    );
    let ids = sentence_ids(&resync);
    assert_eq!(
        (ids.len(), ids.first(), ids.last()),
        (500, Some(&101), Some(&600))
    );
    let after = hub.push_sentence("guest", closed("After."));
    assert_eq!(sentence_ids(&next(&mut client).await), vec![after]);
    client.close(None).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let disconnected = loop {
        let lines = capture.events(LISTEN_WS_TARGET);
        if let Some(line) = lines.iter().find(|line| {
            line.field("message")
                .is_some_and(|m| m.contains("listen ws reader disconnected"))
        }) {
            break line.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "{lines:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(disconnected.field("resyncs"), Some("1"));
    assert_eq!(
        disconnected.field("reason"),
        Some("\"client close (no close code)\"")
    );
    let lines = capture.events(LISTEN_WS_TARGET);
    assert!(lines.iter().any(|line| line
        .field("message")
        .is_some_and(|m| m.contains("listen ws reader connected"))
        && line.field("client_ip").is_some()));
    assert!(lines.iter().any(|line| line.field("missed") == Some("88")));
}

/// The first disconnect line on the reader target, waited for up to 8 s.
async fn disconnect_line(capture: &Capture) -> crate::tracing_capture::Captured {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let lines = capture.events(LISTEN_WS_TARGET);
        if let Some(line) = lines.iter().find(|line| {
            line.field("message")
                .is_some_and(|m| m.contains("listen ws reader disconnected"))
        }) {
            return line.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "{lines:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_reader_gets_pings_and_stays_while_it_answers() {
    let (addr, _hub) = serve().await;
    let mut client = connect(addr).await;
    assert!(matches!(
        next(&mut client).await,
        ListenWsMessage::Snapshot { .. }
    ));
    // Reading answers each ping with the automatic pong.
    let mut pings = 0;
    let until = tokio::time::Instant::now() + KEEPALIVE * 4 + KEEPALIVE / 2;
    while let Ok(frame) = tokio::time::timeout_at(until, client.next()).await {
        match frame {
            Some(Ok(WsMessage::Ping(_))) => pings += 1,
            other => panic!("only pings while the hub is quiet, got {other:?}"),
        }
    }
    assert!(pings >= 3, "a ping every keepalive interval: {pings}");
}

#[tokio::test]
async fn a_silent_reader_is_dropped_and_logged_with_the_tunnel_client_ip() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let capture = Capture::install();
    let (addr, _hub) = serve().await;
    let mut request = format!("ws://{addr}/listen/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
    // Never read again after the upgrade: no pong ever goes back.
    let (_client, _response) = tokio_tungstenite::connect_async(request).await.unwrap();
    let line = disconnect_line(&capture).await;
    assert_eq!(
        line.field("reason"),
        Some("\"client silent (no frame for two keepalive intervals)\"")
    );
    assert_eq!(
        line.field("client_ip"),
        Some("\"203.0.113.7\""),
        "from a loopback peer (the tunnel) X-Forwarded-For names the client (#793)"
    );
}
