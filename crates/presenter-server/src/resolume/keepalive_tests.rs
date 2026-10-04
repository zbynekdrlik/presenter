//! #808 follow-up: the Resolume client must never send a request on a pooled
//! keep-alive connection that Arena may already be closing.
//!
//! Arena closes idle keep-alive connections at irregular times (on SNV after
//! 2 min, then after 30 s). A pooled client that reuses such a connection writes
//! the next request onto a socket Arena is closing. It gets back `client error
//! (SendRequest): connection closed before message completed`. SNV logged 2671 of
//! these in 2 days. Each one is an ERROR line, a status flip and a #484 backoff
//! window that skips pushes, and it can lose a lyric line or a clip trigger.
//!
//! The mock Arena below answers the FIRST request on every connection with
//! `Connection: keep-alive` and keeps the socket open, like Arena does. If a
//! second request arrives on that connection, the mock closes the socket
//! without answering. That is the crossing seen on SNV: Arena closed the idle
//! socket just as our request reached it. A mock that closes right after each
//! response would not reproduce the bug: the FIN reaches hyper before the next
//! checkout, and the pool drops the connection itself.
//!
//! The driver runs on `ResolumeRegistry::new()`'s own client, so these tests
//! pin the pool policy every host worker uses, not a client built for a test.

use super::driver::HostDriver;
use super::mapping_refresh::Push;
use super::{ResolumeConnectionSnapshot, ResolumeConnectionState, ResolumeRegistry, StageUpdate};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

type Status = Arc<RwLock<ResolumeConnectionSnapshot>>;

/// How many ticks / lyric lines each test drives.
const ROUNDS: usize = 4;

/// The idle gap between two driver operations. On SNV the probes are 10 s
/// apart and lyric lines seconds apart, so a pooling client always had the
/// previous connection back in its idle pool by the next request. This short
/// gap gives hyper's pool-return task the same chance to run. It is the bug's
/// precondition, not a wait for a result.
const IDLE_GAP: Duration = Duration::from_millis(20);

#[derive(Default)]
struct Counters {
    /// Requests the mock answered: the first request on each connection.
    answered: AtomicUsize,
    /// Requests that arrived on an already-used connection. The mock closed
    /// the socket without an answer.
    reused: AtomicUsize,
}

struct KeepAliveArena {
    addr: SocketAddr,
    counters: Arc<Counters>,
}

impl KeepAliveArena {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the mock Arena");
        let addr = listener.local_addr().expect("mock Arena address");
        let counters = Arc::new(Counters::default());
        let accept_counters = Arc::clone(&counters);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_connection(stream, Arc::clone(&accept_counters)));
            }
        });
        Self { addr, counters }
    }

    fn answered(&self) -> usize {
        self.counters.answered.load(SeqCst)
    }

    fn reused(&self) -> usize {
        self.counters.reused.load(SeqCst)
    }
}

/// Answer the first request on this connection and keep the socket open. If a
/// second request arrives, drop the socket unanswered. The whole request was
/// read, so the client sees a clean FIN while it waits for the response.
async fn serve_connection(stream: TcpStream, counters: Arc<Counters>) {
    let mut reader = BufReader::new(stream);
    let mut answered_one = false;
    while let Some((method, path)) = read_request(&mut reader).await {
        if answered_one {
            counters.reused.fetch_add(1, SeqCst);
            return;
        }
        answered_one = true;
        // Counted before the write, so the client can never see the response
        // before the counter does.
        counters.answered.fetch_add(1, SeqCst);
        let response = response_for(&method, &path);
        if reader
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Read one HTTP/1.1 request: the head plus its `Content-Length` body. Returns
/// `None` once the client closed the connection.
async fn read_request(reader: &mut BufReader<TcpStream>) -> Option<(String, String)> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.ok()? == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok()?;
            }
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.ok()?;
    Some((method, path))
}

fn clip(id: i64, name: &str, param_id: i64) -> Value {
    json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": { "text": { "valuetype": "ParamText", "id": param_id } } },
    })
}

/// The routes the driver uses. Every answer advertises `Connection: keep-alive`.
fn response_for(method: &str, path: &str) -> String {
    let (status, body) = match (method, path) {
        ("GET", "/api/v1/composition") => (
            "200 OK",
            json!({ "layers": [ { "clips": [clip(100, "#main-a", 1), clip(101, "#main-b", 2)] } ] })
                .to_string(),
        ),
        ("GET", "/api/v1/product") => (
            "200 OK",
            json!({ "name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0 })
                .to_string(),
        ),
        ("PUT", p) if p.starts_with("/api/v1/parameter/by-id/") => ("200 OK", String::new()),
        ("POST", p) if p.starts_with("/api/v1/composition/clips/by-id/") => {
            ("200 OK", String::new())
        }
        _ => ("404 Not Found", String::new()),
    };
    format!(
        "HTTP/1.1 {status}\r\nConnection: keep-alive\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// A driver for the mock Arena on the registry's own client.
fn driver_on_registry_client(arena: &KeepAliveArena) -> (HostDriver, Status) {
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Keep-alive Arena".into(),
        arena.addr.ip().to_string(),
        arena.addr.port(),
        true,
        now,
        now,
    );
    let client = ResolumeRegistry::new().expect("registry").client;
    (
        HostDriver::new(client, config),
        Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled())),
    )
}

async fn assert_no_host_error(status: &Status, step: &str) {
    let snapshot = status.read().await;
    assert_eq!(
        snapshot.consecutive_failures, 0,
        "{step}: the host recorded an error: {:?}",
        snapshot.last_error
    );
    assert_eq!(
        snapshot.state,
        ResolumeConnectionState::Connected,
        "{step}: {:?}",
        snapshot.last_error
    );
}

fn stage(main: &str) -> Push {
    Push::Stage(StageUpdate {
        current_main: Some(main.to_string()),
        current_translation: None,
        song_name: None,
        band_name: None,
        enqueued_at: None,
        correlation_id: None,
    })
}

/// RED before the fix: the second tick's `/product` probe went out on the
/// composition fetch's pooled connection. The mock closed it unanswered, and
/// the host recorded `connection closed before message completed`.
#[tokio::test]
async fn liveness_ticks_never_reuse_a_keep_alive_connection() {
    let arena = KeepAliveArena::start().await;
    let (mut driver, status) = driver_on_registry_client(&arena);

    for round in 0..ROUNDS {
        if round > 0 {
            tokio::time::sleep(IDLE_GAP).await;
        }
        // Round 0 fetches the composition; the later rounds are /product probes.
        driver.tick(&status).await;
        assert_no_host_error(&status, &format!("tick {round}")).await;
    }

    assert_eq!(
        arena.reused(),
        0,
        "no request may go out on a reused keep-alive connection"
    );
    assert_eq!(arena.answered(), ROUNDS, "one answered request per tick");
}

/// RED before the fix: the first lyric PUT reused the tick's pooled connection.
/// The mock closed it unanswered, so the line never reached the wall, and the
/// #484 backoff then skipped the next lines.
#[tokio::test]
async fn lyric_pushes_never_reuse_a_keep_alive_connection() {
    let arena = KeepAliveArena::start().await;
    let (mut driver, status) = driver_on_registry_client(&arena);
    driver.tick(&status).await; // cold start: fetch the composition
    assert_no_host_error(&status, "cold-start tick").await;

    for line in 1..=ROUNDS {
        tokio::time::sleep(IDLE_GAP).await;
        driver
            .dispatch_push(stage(&format!("Line {line}")), &status)
            .await;
        assert_no_host_error(&status, &format!("push of line {line}")).await;
    }

    assert_eq!(
        arena.reused(),
        0,
        "no request may go out on a reused keep-alive connection"
    );
    assert_eq!(
        arena.answered(),
        1 + 2 * ROUNDS,
        "the cold-start fetch, then one text PUT and one clip connect per line"
    );
}
