//! #813: the #564 port-drift probe must never move a host onto another
//! Resolume host's Arena on the same machine.
//!
//! PP runs two Arenas on one PC: Arena-Bridge on 8090 (presenter host
//! `arena bridge`) and the songs Arena on 8091 (host `arena songs`). With
//! Arena-Bridge down, the bridge host's probe scanned 8090..=8095, found the
//! songs Arena answering `/product` on 8091, adopted it and persisted it
//! (`2026-10-06T21:25:59Z WARN resolume port drifted configured_port=8090
//! to=8091`). From then on both hosts drove Songs PP: every lyric twice, every
//! Bible push into a composition without `#bible` clips.
//!
//! These tests go through the real registry and host workers
//! (`ResolumeRegistry::set_hosts`, as `AppState::sync_resolume_hosts` does),
//! with mock Arenas on explicit loopback ports. Ports come from
//! `free_port_pair()` / `free_port_triple()` (verified-free consecutive ports,
//! `.claude/rules/resolume-port-drift.md`), never `free_port() + N`.

use super::driver::HostCommand;
use super::port_drift_integration_tests::free_port_pair;
use super::{
    notify_siblings_changed, ResolumeConnectionSnapshot, ResolumeConnectionState,
    ResolumeErrorKind, ResolumeRegistry,
};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostDraft, ResolumeHostId};
use presenter_persistence::{Repository, SettingsAuditSource};
use std::net::TcpListener as StdTcpListener;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Bound for anything the workers must reach (a connect, an adoption, a
/// persisted clear). Generous for a loaded CI runner under llvm-cov.
const WAIT_BOUND: Duration = Duration::from_secs(10);
/// How long a host must keep NOT adopting a port after its probe ran. On
/// loopback the probe answers in milliseconds (one refused dial plus one
/// `/product` GET per candidate, 500 ms timeout each at worst).
const NEVER_WINDOW: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(50);

/// Three CONSECUTIVE free ports, verified exactly like `free_port_pair()`
/// (#744): `base` and `base + 1` stay bound while `base + 2` is tried, so a
/// success proves all three were free at once.
fn free_port_triple() -> (u16, u16, u16) {
    for _ in 0..100 {
        let base_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind base ephemeral port");
        let base = base_listener.local_addr().expect("base local addr").port();
        let (Some(second), Some(third)) = (base.checked_add(1), base.checked_add(2)) else {
            continue; // no room above base; retry
        };
        let Ok(second_listener) = StdTcpListener::bind(("127.0.0.1", second)) else {
            continue;
        };
        let Ok(third_listener) = StdTcpListener::bind(("127.0.0.1", third)) else {
            continue;
        };
        drop(third_listener);
        drop(second_listener);
        drop(base_listener);
        return (base, second, third);
    }
    panic!("could not find three free consecutive ports after 100 attempts");
}

#[test]
fn free_port_triple_returns_three_consecutive_ports() {
    let (base, second, third) = free_port_triple();
    assert_eq!((second, third), (base + 1, base + 2));
}

/// A mock Arena answering `/product` (as Arena) and `/composition` on an
/// explicit loopback port.
async fn start_arena_on(port: u16) -> MockServer {
    let listener = StdTcpListener::bind(("127.0.0.1", port)).expect("bind the Arena port");
    let server = MockServer::builder().listener(listener).start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/product"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Composition", "layers": [], "columns": [],
        })))
        .mount(&server)
        .await;
    server
}

fn host_on(label: &str, host: &str, port: u16) -> ResolumeHost {
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        label.to_string(),
        host.to_string(),
        port,
        true,
        now,
        now,
    )
}

/// Poll the host's status snapshot until `pred` holds, bounded by
/// [`WAIT_BOUND`].
async fn wait_for(
    registry: &ResolumeRegistry,
    id: ResolumeHostId,
    what: &str,
    pred: impl Fn(&ResolumeConnectionSnapshot) -> bool,
) -> ResolumeConnectionSnapshot {
    let deadline = Instant::now() + WAIT_BOUND;
    loop {
        let snap = registry.snapshot_for(id).await;
        if pred(&snap) {
            return snap;
        }
        assert!(
            Instant::now() < deadline,
            "{what} within {WAIT_BOUND:?}; last snapshot: {snap:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// The host's own Arena refused its connection, which is what runs the
/// port-drift probe (`record_error` probes right after recording it).
async fn wait_for_refused(registry: &ResolumeRegistry, id: ResolumeHostId) {
    wait_for(
        registry,
        id,
        "the down host records a refused connection",
        |s| s.last_error_kind == Some(ResolumeErrorKind::ConnectRefused),
    )
    .await;
}

/// Over [`NEVER_WINDOW`], the host's snapshot must never satisfy `bad`.
async fn assert_never(
    registry: &ResolumeRegistry,
    id: ResolumeHostId,
    what: &str,
    bad: impl Fn(&ResolumeConnectionSnapshot) -> bool,
) {
    let deadline = Instant::now() + NEVER_WINDOW;
    while Instant::now() < deadline {
        let snap = registry.snapshot_for(id).await;
        assert!(!bad(&snap), "{what}; snapshot: {snap:?}");
        tokio::time::sleep(POLL).await;
    }
}

async fn persisted_host(repo: &Repository, id: ResolumeHostId) -> ResolumeHost {
    repo.list_resolume_hosts()
        .await
        .expect("list resolume hosts")
        .into_iter()
        .find(|host| host.id == id)
        .expect("the host is persisted")
}

/// Poll the DB until the host's persisted `active_port` equals `expected`.
async fn wait_for_persisted_active_port(
    repo: &Repository,
    id: ResolumeHostId,
    expected: Option<u16>,
    what: &str,
) {
    let deadline = Instant::now() + WAIT_BOUND;
    loop {
        let active_port = persisted_host(repo, id).await.active_port;
        if active_port == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} within {WAIT_BOUND:?}; persisted active_port is {active_port:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

async fn create_host(repo: &Repository, label: &str, port: u16) -> ResolumeHost {
    repo.create_resolume_host(
        &ResolumeHostDraft::new(label, "127.0.0.1", port),
        SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .expect("create resolume host")
}

/// The PP incident. Two hosts on one address, adjacent ports. The bridge's
/// Arena is down, the songs Arena answers `/product` on the next port up: the
/// bridge host must NOT drift onto it.
#[tokio::test]
async fn a_host_never_drifts_onto_the_port_of_a_sibling_on_the_same_address() {
    let (bridge_port, songs_port) = free_port_pair();
    let _songs_arena = start_arena_on(songs_port).await;
    let bridge = host_on("arena bridge", "127.0.0.1", bridge_port);
    let songs = host_on("arena songs", "127.0.0.1", songs_port);

    let registry = ResolumeRegistry::new().expect("registry");
    registry
        .set_hosts(vec![bridge.clone(), songs.clone()])
        .await;

    wait_for(&registry, songs.id, "the songs host connects", |s| {
        s.state == ResolumeConnectionState::Connected
    })
    .await;
    wait_for_refused(&registry, bridge.id).await;
    assert_never(
        &registry,
        bridge.id,
        "the bridge host adopted the songs host's port, both hosts now drive the songs Arena",
        |s| s.active_port.is_some(),
    )
    .await;
}

/// A sibling that already drifted (persisted `active_port`) owns that port
/// too: another host on the address must not adopt it.
#[tokio::test]
async fn a_host_never_drifts_onto_a_port_a_sibling_has_adopted() {
    // The songs Arena could not bind its configured port, the next one up was
    // the bridge's, so it bound the one after; the songs host found it there
    // earlier (#564) and persisted it.
    let (songs_configured_port, bridge_port, songs_drifted_port) = free_port_triple();
    let _songs_arena = start_arena_on(songs_drifted_port).await;
    let bridge = host_on("arena bridge", "127.0.0.1", bridge_port);
    let songs = host_on("arena songs", "127.0.0.1", songs_configured_port)
        .with_active_port(Some(songs_drifted_port));

    let registry = ResolumeRegistry::new().expect("registry");
    registry
        .set_hosts(vec![bridge.clone(), songs.clone()])
        .await;

    wait_for(
        &registry,
        songs.id,
        "the songs host connects on its adopted port",
        |s| {
            s.state == ResolumeConnectionState::Connected
                && s.active_port == Some(songs_drifted_port)
        },
    )
    .await;
    wait_for_refused(&registry, bridge.id).await;
    assert_never(
        &registry,
        bridge.id,
        "the bridge host adopted the port the songs host drifted to",
        |s| s.active_port.is_some(),
    )
    .await;
}

/// A sibling's drift learned at RUNTIME (never persisted to the registry's
/// config) is excluded as well.
#[tokio::test]
async fn a_host_never_drifts_onto_a_port_a_sibling_adopted_at_runtime() {
    let (songs_port, bridge_port, arena_port) = free_port_triple();
    let _songs_arena = start_arena_on(arena_port).await;
    let songs = host_on("arena songs", "127.0.0.1", songs_port);
    let bridge = host_on("arena bridge", "127.0.0.1", bridge_port);

    let registry = ResolumeRegistry::new().expect("registry");
    // Alone on the address, the songs host drifts onto its Arena (#564).
    registry.set_hosts(vec![songs.clone()]).await;
    wait_for(&registry, songs.id, "the songs host drifts as #564", |s| {
        s.active_port == Some(arena_port)
    })
    .await;

    // The bridge host is added, as from the settings page.
    registry
        .set_hosts(vec![songs.clone(), bridge.clone()])
        .await;
    wait_for_refused(&registry, bridge.id).await;
    assert_never(
        &registry,
        bridge.id,
        "the bridge host adopted the port the songs host drifted to at runtime",
        |s| s.active_port.is_some(),
    )
    .await;
    assert_eq!(
        registry.snapshot_for(songs.id).await.active_port,
        Some(arena_port),
        "adding a host must not take the songs host off its own Arena"
    );
}

/// #564 unchanged for a host alone on its address.
#[tokio::test]
async fn a_host_alone_on_its_address_still_drifts_to_the_next_port() {
    let (configured_port, drifted_port) = free_port_pair();
    let _arena = start_arena_on(drifted_port).await;
    let host = host_on("arena", "127.0.0.1", configured_port);

    let registry = ResolumeRegistry::new().expect("registry");
    registry.set_hosts(vec![host.clone()]).await;

    wait_for(
        &registry,
        host.id,
        "the host adopts the drifted port",
        |s| s.active_port == Some(drifted_port),
    )
    .await;
}

/// A host on ANOTHER address with the same port number is not a sibling:
/// it runs a different Arena, so it does not block the drift.
#[tokio::test]
async fn a_host_on_another_address_does_not_block_the_drift() {
    let (configured_port, drifted_port) = free_port_pair();
    let _arena = start_arena_on(drifted_port).await;
    let host = host_on("arena", "127.0.0.1", configured_port);
    // 127.0.0.2 is loopback too, but nothing listens there: another machine.
    let elsewhere = host_on("arena elsewhere", "127.0.0.2", drifted_port);

    let registry = ResolumeRegistry::new().expect("registry");
    registry.set_hosts(vec![host.clone(), elsewhere]).await;

    wait_for(
        &registry,
        host.id,
        "the host adopts the drifted port",
        |s| s.active_port == Some(drifted_port),
    )
    .await;
}

/// The PP DB after the incident: the bridge host persisted the songs host's
/// port as its `active_port`. Loading the hosts (a restart after the deploy)
/// must clear it in memory AND in the DB, so the incident heals by itself.
#[tokio::test]
async fn a_persisted_active_port_that_a_sibling_owns_is_cleared_on_load() {
    let (bridge_port, songs_port) = free_port_pair();
    let _songs_arena = start_arena_on(songs_port).await;
    let repo = Repository::connect_in_memory().await.expect("repo");
    let bridge = create_host(&repo, "arena bridge", bridge_port).await;
    create_host(&repo, "arena songs", songs_port).await;
    repo.update_resolume_host_active_port(bridge.id, Some(songs_port))
        .await
        .expect("persist the incident's active_port");

    let registry = ResolumeRegistry::new().expect("registry");
    registry.attach_audit_writer(repo.clone());
    registry
        .set_hosts(repo.list_resolume_hosts().await.expect("list hosts"))
        .await;

    wait_for_persisted_active_port(
        &repo,
        bridge.id,
        None,
        "the bridge host's sibling-owned active_port is cleared in the DB",
    )
    .await;
    wait_for(
        &registry,
        bridge.id,
        "the bridge host dials its configured port",
        |s| s.active_port.is_none(),
    )
    .await;
}

/// A host added on the port another host had drifted to owns that port from
/// then on: the drifted host drops it (memory and DB) on that config refresh.
#[tokio::test]
async fn adding_a_host_on_an_adopted_port_clears_that_adoption() {
    let (configured_port, drifted_port) = free_port_pair();
    let _arena = start_arena_on(drifted_port).await;
    let repo = Repository::connect_in_memory().await.expect("repo");
    let drifted = create_host(&repo, "arena", configured_port).await;
    repo.update_resolume_host_active_port(drifted.id, Some(drifted_port))
        .await
        .expect("persist a #564 drift");

    let registry = ResolumeRegistry::new().expect("registry");
    registry.attach_audit_writer(repo.clone());
    registry
        .set_hosts(repo.list_resolume_hosts().await.expect("list hosts"))
        .await;
    wait_for(
        &registry,
        drifted.id,
        "alone, the host keeps its drift",
        |s| s.state == ResolumeConnectionState::Connected && s.active_port == Some(drifted_port),
    )
    .await;

    create_host(&repo, "arena songs", drifted_port).await;
    registry
        .set_hosts(repo.list_resolume_hosts().await.expect("list hosts"))
        .await;

    wait_for_persisted_active_port(
        &repo,
        drifted.id,
        None,
        "the drift onto the new host's port is cleared in the DB",
    )
    .await;
    wait_for(
        &registry,
        drifted.id,
        "the host dials its configured port again",
        |s| s.active_port.is_none(),
    )
    .await;
}

/// `set_hosts` holds the hosts lock every push needs while it notifies the
/// workers, so a worker with a full queue must not make it wait: the notice
/// still arrives once the worker has room.
#[tokio::test]
async fn a_full_worker_queue_never_holds_up_the_siblings_changed_notice() {
    let (command_tx, mut command_rx) = mpsc::channel(1);
    command_tx
        .try_send(HostCommand::Shutdown)
        .expect("fill the one-slot queue");

    // Synchronous: returns at once although the queue is full.
    notify_siblings_changed(&command_tx);

    assert!(matches!(
        command_rx.recv().await,
        Some(HostCommand::Shutdown)
    ));
    let next = tokio::time::timeout(WAIT_BOUND, command_rx.recv())
        .await
        .expect("the notice arrives once the queue has room");
    assert!(
        matches!(next, Some(HostCommand::SiblingsChanged)),
        "got {next:?}"
    );
}
