//! #813: the two `PortClaims` steps of the drift probe, driven on a
//! `HostDriver` directly (`claim_dial_port`), with no network.
//!
//! `probe_port_drift` reads the sibling ports, probes, and only then claims
//! the port it found. Another host's worker can claim the same port in
//! between (two hosts down at once, one Arena in both windows): the claim
//! must then fail without touching the table, and the probe moves on. A
//! heal-back to the configured port must free the drifted port for others.

use super::driver::HostDriver;
use super::port_claims::PortClaims;
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;

const ADDRESS: &str = "10.77.8.201";

fn host_on(label: &str, port: u16) -> ResolumeHost {
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        label.to_string(),
        ADDRESS.to_string(),
        port,
        true,
        now,
        now,
    )
}

/// A driver sharing `claims`, as `run_host_worker` sets it up.
fn driver_with(claims: &PortClaims, host: ResolumeHost) -> HostDriver {
    let mut driver = HostDriver::new(Client::new(), host);
    driver.port_claims = claims.clone();
    driver
}

#[tokio::test]
async fn a_port_a_sibling_claimed_after_the_probe_read_the_table_is_not_taken() {
    let songs = host_on("arena songs", 8089);
    let bridge = host_on("arena bridge", 8090);
    let claims = PortClaims::default();
    claims.rebuild([&songs, &bridge], |_| false);
    let driver = driver_with(&claims, bridge.clone());
    // The bridge dials 8093 (an earlier drift) and its Arena moves again.
    assert!(driver.claim_dial_port(Some(8093)));
    // Both hosts are down and one Arena answers on 8092, inside both probe
    // windows. The songs worker claims it first.
    assert_eq!(claims.try_claim(songs.id, ADDRESS, 8089, 8092), Ok(()));

    assert!(!driver.claim_dial_port(Some(8092)));

    // Nothing changed: the bridge still holds 8093 (not released), the songs
    // host 8092.
    assert_eq!(claims.active_port_of(bridge.id), Some(8093));
    assert_eq!(claims.active_port_of(songs.id), Some(8092));
}

#[tokio::test]
async fn a_heal_back_frees_the_drifted_port_for_a_sibling() {
    let songs = host_on("arena songs", 8089);
    let bridge = host_on("arena bridge", 8090);
    let claims = PortClaims::default();
    claims.rebuild([&songs, &bridge], |_| false);
    let driver = driver_with(&claims, bridge.clone());

    // The bridge drifted to 8092, so the songs host may not take it.
    assert!(driver.claim_dial_port(Some(8092)));
    assert_eq!(claims.active_port_of(bridge.id), Some(8092));
    assert_eq!(
        claims.try_claim(songs.id, ADDRESS, 8089, 8092),
        Err("arena bridge".to_string())
    );

    // The bridge's Arena is back on 8090: heal-back.
    assert!(driver.claim_dial_port(None));

    assert_eq!(claims.active_port_of(bridge.id), None);
    assert_eq!(claims.try_claim(songs.id, ADDRESS, 8089, 8092), Ok(()));
}
