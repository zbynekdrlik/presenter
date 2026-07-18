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
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A free ephemeral port, released immediately — nothing else binds it in
/// the tiny window before the test claims it explicitly (negligible flake
/// risk on loopback within a test's lifetime).
fn free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
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
    let configured_port = free_port();
    let drifted_port = configured_port + 1;

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

    // The next fetch against the (now-refused) configured port fails;
    // record_error classifies it as ConnectRefused and probes the window.
    let err = driver
        .refresh_mapping()
        .await
        .expect_err("the configured port now refuses connections");
    driver.record_error(err, &status).await;

    assert_eq!(
        driver.active_port,
        Some(drifted_port),
        "the driver must adopt the drifted port after a connect-refused probe"
    );
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
    let configured_port = free_port();
    let drifted_port = configured_port + 1;

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

    // The next fetch (still dialing the now-dead drifted port) fails, and the
    // probe — which ALWAYS scans from the CONFIGURED port first — finds it
    // healthy again and heals back.
    let err = driver
        .refresh_mapping()
        .await
        .expect_err("the drifted port now refuses connections");
    driver.record_error(err, &status).await;

    assert_eq!(
        driver.active_port, None,
        "healing back to the configured port must clear active_port"
    );
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
    let configured_port = free_port();
    let drifted_port = configured_port + 1;

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
