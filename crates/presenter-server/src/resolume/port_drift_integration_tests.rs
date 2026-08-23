//! #564: integration tests proving the port-drift auto-recovery loop against
//! a REAL (if minimal) HTTP server — a mock Arena that "moves" from one port
//! to another mid-test, exactly the field incident (resolume-pp configured on
//! 8090, Arena actually on 8091). Uses wiremock's
//! `MockServer::builder().listener(...)` to bind EXPLICIT ports (instead of
//! wiremock's usual random port) so the drift target (`configured_port + 1`)
//! is deterministic and controllable mid-test.

use super::driver::HostDriver;
use super::{ResolumeConnectionSnapshot, ResolumeConnectionState};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A pair of CONSECUTIVE free ephemeral ports `(base, base + 1)` — both
/// verified bindable, then released. The port-drift tests need the drift
/// target to be exactly `configured_port + 1` (the #564 field incident:
/// resolume-pp configured on 8090, Arena actually on 8091) AND within the
/// driver's probe window, so the two ports must stay contiguous and BOTH be
/// free.
///
/// #744: the old `free_port()` grabbed ONE `:0` port and ASSUMED `+ 1` was
/// free without ever checking it — under parallel `cargo test` load another
/// process routinely held `base + 1`, so the tests' explicit
/// `bind(("127.0.0.1", base + 1))` panicked and red the whole Test job. This
/// binds `base` via `:0`, then ACTUALLY tries to bind `base + 1`; if that is
/// taken (or `base` is `u16::MAX`), it retries with a fresh `base`. `base + 1`
/// is now VERIFIED free at allocation instead of blindly assumed. The residual
/// release-then-rebind TOCTOU is the same negligible on-loopback window the
/// single-port helper already accepted.
fn free_port_pair() -> (u16, u16) {
    for _ in 0..100 {
        let base_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind base ephemeral port");
        let base = base_listener.local_addr().expect("base local addr").port();
        let Some(next) = base.checked_add(1) else {
            continue; // base == u16::MAX — no room for base + 1; retry
        };
        // Hold `base` while probing `next`, so a success proves BOTH ports
        // were simultaneously free.
        let Ok(next_listener) = StdTcpListener::bind(("127.0.0.1", next)) else {
            continue; // base + 1 already taken — pick a different base
        };
        drop(next_listener);
        drop(base_listener);
        return (base, next);
    }
    panic!("could not find a free consecutive port pair after 100 attempts");
}

#[test]
fn free_port_pair_returns_two_consecutive_ports() {
    // The port-drift tests rely on the drift target being exactly
    // `configured_port + 1` and inside the driver's probe window — the pair
    // MUST be contiguous. Deterministic: it asserts the invariant only and
    // never rebinds, so it cannot reintroduce the TOCTOU it guards against.
    let (base, next) = free_port_pair();
    assert_eq!(
        next,
        base + 1,
        "free_port_pair must return contiguous ports"
    );
}

async fn mount_arena(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/product"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0,
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Composition", "layers": [], "columns": [],
        })))
        .mount(server)
        .await;
}

/// A non-Resolume HTTP server on the drifted port — proves the probe does
/// NOT adopt just any server that happens to answer nearby.
async fn mount_unrelated_server(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/product"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "nginx",
        })))
        .mount(server)
        .await;
}

fn host_config(port: u16) -> ResolumeHost {
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        "Test Arena".to_string(),
        "127.0.0.1".to_string(),
        port,
        true,
        now,
        now,
    )
}

/// Drive the fail→classify→probe cycle until the driver's dial state reaches
/// `expected_active`, bounded (retry-with-assert, per the project's E2E
/// discipline). A ONE-SHOT assert on a single cycle is racy on a loaded
/// runner: the dropped mock's shutdown is asynchronous, so the first dial
/// can land accepted-then-reset (classified Reset, not ConnectRefused — no
/// probe), and the 500ms probe budget can starve under llvm-cov
/// instrumentation (both seen as a real coverage-job flake, 2026-07-20).
/// Production recovers the same way this loop does — the driver probes again
/// on the NEXT connect-refused failure — so retrying does not weaken the
/// contract: the driver MUST reach `expected_active` within the bound.
async fn drive_until_active_port(
    driver: &mut HostDriver,
    status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    expected_active: Option<u16>,
    context: &str,
) {
    for _ in 0..20 {
        if driver.active_port == expected_active {
            return;
        }
        // An Ok here means the OLD mock was still draining its shutdown and
        // answered — not a failure of the drift logic; dial again.
        if let Err(err) = driver.refresh_mapping().await {
            driver.record_error(err, status).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(driver.active_port, expected_active, "{context}");
}

fn fresh_snapshot() -> Arc<RwLock<ResolumeConnectionSnapshot>> {
    Arc::new(RwLock::new(ResolumeConnectionSnapshot {
        state: ResolumeConnectionState::Connecting,
        last_success: None,
        last_latency_ms: None,
        last_error: None,
        last_error_kind: None,
        consecutive_failures: 0,
        last_attempt: None,
        error_since: None,
        next_retry_at: None,
        active_port: None,
        missing_clips: Vec::new(),
    }))
}

#[tokio::test]
async fn driver_adopts_the_drifted_port_after_a_connection_refused_and_reconnects() {
    let (configured_port, drifted_port) = free_port_pair();

    // Arena starts on the CONFIGURED port; a baseline fetch succeeds.
    let listener_a =
        StdTcpListener::bind(("127.0.0.1", configured_port)).expect("bind configured port");
    let server_a = MockServer::builder().listener(listener_a).start().await;
    mount_arena(&server_a).await;

    let mut driver = HostDriver::new(Client::new(), host_config(configured_port));
    let status = fresh_snapshot();

    driver
        .refresh_mapping()
        .await
        .expect("baseline fetch on the configured port succeeds");
    assert_eq!(driver.active_port, None, "no drift yet");

    // Arena "moves": the old listener drops (frees the configured port,
    // guaranteeing the next dial is refused) and a NEW listener is bound
    // EXPLICITLY on drifted_port, simulating Arena rebinding the next port up.
    drop(server_a);
    let listener_b = StdTcpListener::bind(("127.0.0.1", drifted_port)).expect("bind drifted port");
    let server_b = MockServer::builder().listener(listener_b).start().await;
    mount_arena(&server_b).await;

    // Fetches against the (now-refused) configured port fail; record_error
    // classifies ConnectRefused and probes the window until adoption.
    drive_until_active_port(
        &mut driver,
        &status,
        Some(drifted_port),
        "the driver must adopt the drifted port after a connect-refused probe",
    )
    .await;
    let snap = status.read().await.clone();
    assert_eq!(
        snap.active_port,
        Some(drifted_port),
        "the status snapshot must reflect the drift immediately, not after the next success"
    );

    // The NEXT fetch (now dialing the adopted port) succeeds.
    driver
        .refresh_mapping()
        .await
        .expect("subsequent fetch dials the adopted port and succeeds");
}

#[tokio::test]
async fn driver_heals_back_to_the_configured_port_once_it_answers_again() {
    let (configured_port, drifted_port) = free_port_pair();

    // Start drifted: Arena only answers on drifted_port; the driver has
    // already adopted it (simulating a prior drift discovery).
    let listener_b = StdTcpListener::bind(("127.0.0.1", drifted_port)).expect("bind drifted port");
    let server_b = MockServer::builder().listener(listener_b).start().await;
    mount_arena(&server_b).await;

    let mut driver = HostDriver::new(
        Client::new(),
        host_config(configured_port).with_active_port(Some(drifted_port)),
    );
    let status = fresh_snapshot();
    driver
        .refresh_mapping()
        .await
        .expect("baseline fetch on the drifted port succeeds");

    // Arena restarts cleanly and re-binds its CONFIGURED base port; the
    // drifted port goes away.
    drop(server_b);
    let listener_c =
        StdTcpListener::bind(("127.0.0.1", configured_port)).expect("rebind configured port");
    let server_c = MockServer::builder().listener(listener_c).start().await;
    mount_arena(&server_c).await;

    // Fetches (still dialing the now-dead drifted port) fail, and the probe
    // — which ALWAYS scans from the CONFIGURED port first — finds it healthy
    // again and heals back.
    drive_until_active_port(
        &mut driver,
        &status,
        None,
        "healing back to the configured port must clear active_port",
    )
    .await;
    let snap = status.read().await.clone();
    assert_eq!(
        snap.active_port, None,
        "the snapshot must clear the drift note too"
    );

    driver
        .refresh_mapping()
        .await
        .expect("subsequent fetch dials the configured port and succeeds");
}

#[tokio::test]
async fn probe_never_adopts_a_non_resolume_server_on_a_nearby_port() {
    let (configured_port, drifted_port) = free_port_pair();

    // Nothing listens on the configured port (guaranteed refused). A
    // DIFFERENT, non-Resolume HTTP server happens to be up on the next port —
    // the probe must NOT mistake it for a drifted Arena.
    let listener =
        StdTcpListener::bind(("127.0.0.1", drifted_port)).expect("bind unrelated server port");
    let unrelated = MockServer::builder().listener(listener).start().await;
    mount_unrelated_server(&unrelated).await;

    let mut driver = HostDriver::new(Client::new(), host_config(configured_port));
    let status = fresh_snapshot();

    let err = driver
        .refresh_mapping()
        .await
        .expect_err("nothing listens on the configured port");
    driver.record_error(err, &status).await;

    assert_eq!(
        driver.active_port, None,
        "a non-Resolume server on a nearby port must never be adopted as the active port"
    );
    let snap = status.read().await.clone();
    assert_eq!(
        snap.consecutive_failures, 1,
        "the failure is recorded normally — the probe just found nothing to adopt"
    );
}
