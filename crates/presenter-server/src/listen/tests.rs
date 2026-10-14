//! #852 C1a: the receiver's routing, health and socket life cycle. Routing
//! cases call `MicRouter::handle_datagram` directly with RFC 5737 senders and
//! explicit instants (never a sleep); the socket path has its own real
//! loopback UDP tests on `127.0.0.1:0` with bounded 5 s polls (`wait_for`).

use super::health::{ListenHealth, PacketSeen, RECENT_CAP};
use super::*;
use presenter_core::audio::vban::{VbanAudioFormat, VbanSampleType};
use presenter_core::listen::mics::{MicChannel, MicLanguage, MicRole};
use presenter_core::listen::status::{MicState, SocketState};

const GUEST: &str = "192.0.2.10:6980";
const OTHER: &str = "198.51.100.7:6980";
const MONO_48K: VbanAudioFormat = VbanAudioFormat {
    rate_hz: 48_000,
    channels: 1,
    sample_type: VbanSampleType::Int16,
};

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn mic(id: &str, stream: &str, sender_ip: Option<&str>) -> MicConfig {
    MicConfig {
        id: id.to_string(),
        name: id.to_string(),
        stream_name: stream.to_string(),
        sender_ip: sender_ip.map(str::to_string),
        channel: MicChannel::Left,
        language: MicLanguage::En,
        role: MicRole::Guest,
    }
}

fn seen(at: Instant) -> PacketSeen {
    PacketSeen {
        from: addr(GUEST),
        format: MONO_48K,
        frames: 256,
        at,
    }
}

/// Poll `service.status()` until `done` holds; fails with the last status after 5 s.
async fn wait_for(
    service: &ListenService,
    what: &str,
    done: impl Fn(&ListenStatus) -> bool,
) -> ListenStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = service.status();
        if done(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "{what}: {status:#?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn should_log_event_is_the_power_of_two_gate() {
    let gated: Vec<bool> = (0..10).map(should_log_event).collect();
    assert_eq!(
        gated,
        vec![false, true, true, false, true, false, false, false, true, false]
    );
}

#[test]
fn bind_retries_back_off_to_a_minute() {
    let delays: Vec<u64> = (1..=9).map(|n| bind_retry_delay(n).as_secs()).collect();
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
    assert_eq!(bind_retry_delay(u32::MAX).as_secs(), 60);
}

#[test]
fn the_bind_address_is_the_env_value_or_the_vban_default() {
    assert_eq!(DEFAULT_VBAN_ADDR.to_string(), "0.0.0.0:6980");
    assert_eq!(parse_bind_addr(None), DEFAULT_VBAN_ADDR);
    assert_eq!(parse_bind_addr(Some("  ")), DEFAULT_VBAN_ADDR);
    assert_eq!(parse_bind_addr(Some(" 127.0.0.1:0 ")), addr("127.0.0.1:0"));
    assert_eq!(parse_bind_addr(Some("nonsense")), DEFAULT_VBAN_ADDR);
}

#[tokio::test]
async fn the_mic_state_follows_the_last_packet() {
    let t0 = Instant::now();
    let mics = vec![mic("guest", "GUEST_EN", None)];
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest"]);
    assert_eq!(health.snapshot(&mics, t0).mics[0].state, MicState::Waiting);
    health.record_packet("guest", seen(t0));
    let mic = &health.snapshot(&mics, t0 + Duration::from_millis(500)).mics[0];
    assert_eq!(
        (mic.state, mic.packets, mic.last_packet_age_ms),
        (MicState::Receiving, 1, Some(500))
    );
    assert_eq!(mic.sender.as_deref(), Some(GUEST));
    assert_eq!(
        (mic.format, mic.frames_per_packet),
        (Some(MONO_48K), Some(256))
    );
    let silent = &health.snapshot(&mics, t0 + Duration::from_secs(3)).mics[0];
    assert_eq!(
        silent.state,
        MicState::NoAudio,
        "3 s after the last packet: no audio (C-S1)"
    );
}

#[tokio::test]
async fn a_saved_list_keeps_kept_counters_and_starts_fresh_ones() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest", "interp"]);
    health.record_packet("guest", seen(t0));
    health.record_packet("interp", seen(t0));
    health.set_mics(&["guest"], &["spare"]);
    let mics = vec![
        mic("guest", "GUEST_EN", None),
        mic("spare", "SPARE", None),
        mic("interp", "INTERP_SK", None),
    ];
    let packets: Vec<u64> = health
        .snapshot(&mics, t0)
        .mics
        .iter()
        .map(|m| m.packets)
        .collect();
    assert_eq!(
        packets,
        vec![1, 0, 0],
        "guest kept, spare fresh, interp dropped"
    );
}

#[tokio::test]
async fn unmatched_streams_keep_the_16_most_recent_and_count_per_name() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    for k in 0..=16u64 {
        let at = t0 + Duration::from_millis(k);
        assert_eq!(
            health.note_unmatched(&format!("S{k:02}"), addr(OTHER), MONO_48K, at),
            1
        );
    }
    let again = t0 + Duration::from_millis(17);
    assert_eq!(
        health.note_unmatched("S16", addr(OTHER), MONO_48K, again),
        2,
        "per NAME"
    );
    let status = health.snapshot(&[], t0 + Duration::from_secs(1));
    let names: Vec<&str> = status
        .unmatched
        .iter()
        .map(|s| s.stream_name.as_str())
        .collect();
    assert_eq!(names.len(), RECENT_CAP);
    assert!(!names.contains(&"S00"), "the oldest went");
    assert_eq!(names.first(), Some(&"S01"), "sorted by name");
    assert_eq!(status.unmatched[15].packets, 2);
}

#[tokio::test]
async fn a_service_that_was_never_started_reports_not_started() {
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    let status = service.status();
    assert_eq!(status.socket.state, SocketState::NotStarted);
    assert_eq!(
        (status.mics.len(), status.mics[0].state),
        (1, MicState::Waiting)
    );
    assert_eq!(service.mics().len(), 1);
}
