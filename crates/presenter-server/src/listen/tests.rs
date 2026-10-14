//! #852 C1a: the receiver's routing, health and socket life cycle. Routing
//! cases call `MicRouter::handle_datagram` directly with RFC 5737 senders and
//! explicit instants (never a sleep); the socket path has its own real
//! loopback UDP tests on `127.0.0.1:0` with bounded 5 s polls (`wait_for`).

use super::health::{ListenHealth, PacketSeen, UnmatchedCount, RECENT_CAP};
use super::vban_rx::{receive, MicRouter, SocketEnd, Streak};
use super::*;
use crate::tracing_capture::{quoted, Capture};
use presenter_core::audio::vban::{VbanAudioFormat, VbanSampleType};
use presenter_core::audio::vban_encode::{tone, VbanEncoder};
use presenter_core::listen::mics::{MicChannel, MicLanguage, MicRole};
use presenter_core::listen::status::{MicState, MicStatus, SocketState};

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
            UnmatchedCount {
                packets: 1,
                fresh: true,
                names: k + 1
            }
        );
    }
    let again = t0 + Duration::from_millis(17);
    assert_eq!(
        health.note_unmatched("S16", addr(OTHER), MONO_48K, again),
        UnmatchedCount {
            packets: 2,
            fresh: false,
            names: 17
        },
        "per NAME"
    );
    let evicted = t0 + Duration::from_millis(18);
    assert_eq!(
        health.note_unmatched("S00", addr(OTHER), MONO_48K, evicted),
        UnmatchedCount {
            packets: 1,
            fresh: true,
            names: 18
        },
        "an evicted name enters again"
    );
    let status = health.snapshot(&[], t0 + Duration::from_secs(1));
    let names: Vec<&str> = status
        .unmatched
        .iter()
        .map(|s| s.stream_name.as_str())
        .collect();
    assert_eq!(names.len(), RECENT_CAP);
    assert!(!names.contains(&"S01"), "the oldest went");
    assert_eq!(names.first(), Some(&"S00"), "sorted by name");
    assert_eq!(status.unmatched[15].packets, 2);
}

#[tokio::test]
async fn a_closed_socket_ends_the_session_counters() {
    let t0 = Instant::now();
    let mics = vec![mic("guest", "GUEST_EN", None)];
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest"]);
    health.record_packet("guest", seen(t0));
    health.note_unmatched("Stream1", addr(OTHER), MONO_48K, t0);
    health.end_session();
    let status = health.snapshot(&mics, t0);
    let guest = &status.mics[0];
    assert_eq!(
        (guest.state, guest.packets, guest.sender.as_deref()),
        (MicState::Waiting, 0, None)
    );
    assert!(status.unmatched.is_empty());
    assert_eq!(
        health.note_unmatched("Stream1", addr(OTHER), MONO_48K, t0),
        UnmatchedCount {
            packets: 1,
            fresh: true,
            names: 1
        },
        "the log gates start over"
    );
}

#[test]
fn a_socket_failing_right_after_each_bind_backs_off_and_logs_on_power_of_two() {
    let mut streak = Streak::default();
    let mut bound_lines = Vec::new();
    let mut delays = Vec::new();
    for _ in 0..8 {
        bound_lines.push(streak.log_bound());
        streak.session_failed(false, Duration::from_millis(5));
        delays.push(bind_retry_delay(streak.failures).as_secs());
    }
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(
        bound_lines,
        vec![true, true, false, true, false, false, false, true]
    );
}

#[test]
fn a_healthy_socket_restarts_the_backoff_and_a_freed_port_logs_its_bind() {
    let mut early = Streak::default();
    for _ in 0..3 {
        early.session_failed(false, Duration::from_secs(1));
    }
    assert_eq!(early.failures, 3);
    let mut received = early;
    received.session_failed(true, Duration::from_millis(5));
    assert_eq!((received.failures, received.log_bound()), (1, true));
    let mut long = early;
    long.session_failed(false, Duration::from_secs(30));
    assert_eq!(long.failures, 1, "30 s bound is healthy");
    let mut short = early;
    short.session_failed(false, Duration::from_millis(29_999));
    assert_eq!(short.failures, 4);
    let mut taken = Streak::default();
    for _ in 0..3 {
        taken.bind_failed();
    }
    assert_eq!(
        (taken.failures, taken.log_bound()),
        (3, true),
        "bind retries never hide the bind that finally works"
    );
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

// ---- the receiver (MicRouter): explicit senders and instants ----------------

fn guest_encoder() -> VbanEncoder {
    VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16)
}

/// One 256-frame packet of a 1 kHz tone (48 kHz: 187 packets per second).
fn next_packet(encoder: &mut VbanEncoder) -> Vec<u8> {
    encoder
        .packet(&tone(48_000, 1_000.0, 0.5, 0, 256, 1))
        .unwrap()
}

fn router_for(configs: Vec<MicConfig>) -> (ListenService, MicRouter) {
    let service = ListenService::new();
    service.apply(configs.clone());
    let router = MicRouter::new(&service, &configs);
    (service, router)
}

fn status_of(service: &ListenService, id: &str, now: Instant) -> MicStatus {
    let mics = service.mics();
    let status = service.with_health(|health| health.snapshot(&mics, now));
    status
        .mics
        .into_iter()
        .find(|mic| mic.config.id == id)
        .unwrap()
}

#[tokio::test]
async fn packets_are_routed_by_stream_name_and_sender_ip() {
    let (service, mut router) = router_for(vec![
        mic("guest", "GUEST_EN", Some("192.0.2.10")),
        mic("interp", "INTERP_SK", None),
    ]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    let mut interp = VbanEncoder::new("INTERP_SK", 48_000, 1, VbanSampleType::Int16);
    let mut stray = VbanEncoder::new("Stream1", 48_000, 2, VbanSampleType::Int24);
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    router.handle_datagram(&next_packet(&mut guest), addr(OTHER), t0); // right name, wrong sender
    router.handle_datagram(&next_packet(&mut interp), addr(OTHER), t0);
    let stray_packet = stray.packet(&tone(48_000, 440.0, 0.25, 0, 64, 2)).unwrap();
    router.handle_datagram(&stray_packet, addr(OTHER), t0);
    assert_eq!(status_of(&service, "guest", t0).packets, 1);
    assert_eq!(status_of(&service, "interp", t0).packets, 1);
    let status = service.with_health(|health| health.snapshot(&[], t0));
    let seen: Vec<(&str, &str)> = status
        .unmatched
        .iter()
        .map(|s| (s.stream_name.as_str(), s.sender.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![("GUEST_EN", OTHER), ("Stream1", OTHER)],
        "seen, not used"
    );
    assert_eq!(
        status.unmatched[1].format,
        VbanAudioFormat {
            rate_hz: 48_000,
            channels: 2,
            sample_type: VbanSampleType::Int24
        }
    );
}

#[tokio::test]
async fn an_ipv4_mapped_peer_matches_the_canonical_sender_ip() {
    // A dual-stack socket reports an IPv4 sender as `::ffff:192.0.2.10`; the
    // saved list holds `192.0.2.10` (C1a-1 review 1).
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", Some("192.0.2.10"))]);
    let t0 = Instant::now();
    let mapped = addr("[::ffff:192.0.2.10]:6980");
    router.handle_datagram(&next_packet(&mut guest_encoder()), mapped, t0);
    let guest = status_of(&service, "guest", t0);
    assert_eq!(guest.packets, 1);
    assert_eq!(guest.sender.as_deref(), Some("[::ffff:192.0.2.10]:6980"));
}

#[tokio::test]
async fn a_datagram_that_is_not_vban_audio_reaches_no_mic() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    router.handle_datagram(b"hello", addr(GUEST), t0);
    let mut text = next_packet(&mut guest_encoder());
    text[4] = 0x40; // sub-protocol TEXT (VB-Matrix / VBAN-TEXT traffic on 6980)
    router.handle_datagram(&text, addr(GUEST), t0);
    assert_eq!(status_of(&service, "guest", t0).packets, 0);
    let status = service.with_health(|health| health.snapshot(&[], t0));
    assert!(status.unmatched.is_empty(), "{:?}", status.unmatched);
}

#[tokio::test]
async fn an_unassigned_stream_is_logged_on_power_of_two_packets_per_name() {
    let capture = Capture::install();
    let (_service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut stray = VbanEncoder::new("Stream1", 48_000, 1, VbanSampleType::Int16);
    for _ in 0..5 {
        router.handle_datagram(&next_packet(&mut stray), addr(OTHER), t0);
    }
    // the same name from another sender counts on the same per-name gate
    router.handle_datagram(&next_packet(&mut stray), addr(GUEST), t0);
    let lines = capture.events_with_message("VBAN stream not assigned to any mic");
    let packets: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.field("packets"))
        .collect();
    assert_eq!(packets, vec!["1", "2", "4"]);
    assert_eq!(lines[0].field("stream"), Some(quoted("Stream1").as_str()));
}

#[tokio::test]
async fn a_stopped_sender_shows_no_audio_after_3_s_and_logs_it_once() {
    let capture = Capture::install();
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    router.handle_datagram(&next_packet(&mut guest_encoder()), addr(GUEST), t0);
    let just_before = t0 + Duration::from_millis(2_999);
    assert_eq!(
        status_of(&service, "guest", just_before).state,
        MicState::Receiving
    );
    let at_3_s = t0 + Duration::from_millis(3_000);
    assert_eq!(
        status_of(&service, "guest", at_3_s).state,
        MicState::NoAudio
    );
    for ms in [2_999, 3_100, 4_100] {
        router.on_tick(t0 + Duration::from_millis(ms));
    }
    let events = capture.events(MIC_TARGET);
    let messages: Vec<&str> = events
        .iter()
        .filter_map(|event| event.field("message"))
        .collect();
    assert_eq!(
        messages,
        vec![
            "mic receives VBAN",
            "mic lost audio — no VBAN packet for 3 s"
        ]
    );
    assert_eq!(events[0].field("mic"), Some(quoted("guest").as_str()));
    assert_eq!(events[0].field("stream"), Some(quoted("GUEST_EN").as_str()));
    assert_eq!(events[0].field("rate"), Some("48000"));
}

#[tokio::test]
async fn a_rename_keeps_the_mic_and_a_new_stream_name_starts_it_fresh() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    router.handle_datagram(&next_packet(&mut guest_encoder()), addr(GUEST), t0);
    let mut renamed = mic("guest", "GUEST_EN", None);
    renamed.name = "Hosť z Londýna".into();
    renamed.language = MicLanguage::Off;
    service.apply(vec![renamed.clone()]);
    router.update(&[renamed]);
    assert_eq!(
        status_of(&service, "guest", t0).packets,
        1,
        "a rename / language change resets nothing"
    );
    let moved = mic("guest", "GUEST_2", None);
    service.apply(vec![moved.clone()]);
    router.update(&[moved]);
    assert_eq!(
        status_of(&service, "guest", t0).packets,
        0,
        "another stream name: a fresh mic"
    );
}

// ---- the socket (real loopback UDP) ----------------------------------------

#[tokio::test]
async fn receive_ends_at_once_when_the_list_was_emptied_before_it_started() {
    // supervise saw a mic, then the list was emptied before the bind finished
    let service = ListenService::new();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut config = service.config_receiver();
    let end = tokio::time::timeout(
        Duration::from_secs(5),
        receive(&service, &socket, &mut config),
    )
    .await
    .expect("receive returns at once");
    assert_eq!(end, SocketEnd::NoMic);
}

#[tokio::test]
async fn no_mic_binds_no_socket_and_a_saved_mic_binds_one() {
    let service = ListenService::new();
    assert!(service.start(addr("127.0.0.1:0")));
    assert!(!service.start(addr("127.0.0.1:0")), "started once");
    let idle = wait_for(&service, "idle", |s| s.socket.state == SocketState::Idle).await;
    assert_eq!(
        (idle.socket.local_addr, idle.socket.bind_addr.as_deref()),
        (None, Some("127.0.0.1:0"))
    );
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let local: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    assert!(local.ip().is_loopback() && local.port() != 0);
    service.apply(Vec::new());
    wait_for(&service, "closed", |s| s.socket.state == SocketState::Idle).await;
    std::net::UdpSocket::bind(local).expect("the port is free again: the socket was closed");
}

#[tokio::test]
async fn a_taken_port_is_retried_and_reported_until_it_is_free() {
    let holder = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let taken = holder.local_addr().unwrap();
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    service.start(taken);
    let retrying = wait_for(&service, "retrying", |s| {
        s.socket.state == SocketState::Retrying
    })
    .await;
    let error = retrying.socket.error.unwrap_or_default();
    assert!(error.contains(&taken.port().to_string()), "{error}");
    drop(holder);
    wait_for(&service, "bound after a retry", |s| {
        s.socket.state == SocketState::Bound
    })
    .await;
}

#[tokio::test]
async fn a_sender_rotating_20_names_over_udp_logs_log2_lines_and_one_summary() {
    let capture = Capture::install();
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", Some("127.0.0.1"))]);
    service.start(addr("127.0.0.1:0"));
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let vban: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut rotating: Vec<VbanEncoder> = (0..20)
        .map(|k| VbanEncoder::new(&format!("ROT{k:02}"), 48_000, 1, VbanSampleType::Int16))
        .collect();
    for _ in 0..5 {
        for encoder in &mut rotating {
            tx.send_to(&next_packet(encoder), vban).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // one socket, in order: once the guest's packet counts, the flood was handled
    tx.send_to(&next_packet(&mut guest_encoder()), vban)
        .await
        .unwrap();
    wait_for(&service, "the closing guest packet", |s| {
        s.mics[0].packets == 1
    })
    .await;
    let lines = capture.events_with_message("VBAN stream not assigned to any mic");
    let names: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.field("names"))
        .collect();
    assert_eq!(
        names,
        vec!["1", "2", "4", "8", "16", "32", "64"],
        "100 packets of 20 rotating names: one line per power of two, not 100"
    );
    let summaries = capture.events_with_message("rotate through more than the names kept");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].field("names"), Some("17"));
}

#[tokio::test]
async fn a_failed_rebind_shows_none_of_the_last_sessions_counters() {
    // a fixed port, so the rebind can be made to fail
    let fixed = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", Some("127.0.0.1"))]);
    service.start(fixed);
    wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut guest = guest_encoder();
    for _ in 0..3 {
        tx.send_to(&next_packet(&mut guest), fixed).await.unwrap();
    }
    wait_for(&service, "3 packets", |s| s.mics[0].packets == 3).await;
    service.apply(Vec::new());
    wait_for(&service, "closed", |s| s.socket.state == SocketState::Idle).await;
    let _holder = std::net::UdpSocket::bind(fixed).expect("the port was released");
    service.apply(vec![mic("guest", "GUEST_2", None)]);
    let status = wait_for(&service, "retrying", |s| {
        s.socket.state == SocketState::Retrying
    })
    .await;
    let guest = &status.mics[0];
    assert_eq!(
        (guest.state, guest.packets, guest.sender.as_deref()),
        (MicState::Waiting, 0, None),
        "never the last session's numbers under the new stream name"
    );
}

#[tokio::test]
async fn real_udp_packets_reach_the_mic_through_the_socket() {
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", Some("127.0.0.1"))]);
    service.start(addr("127.0.0.1:0"));
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let vban: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut guest = guest_encoder();
    for k in 0..12 {
        tx.send_to(&next_packet(&mut guest), vban).await.unwrap();
        if k % 4 == 3 {
            tokio::time::sleep(Duration::from_millis(5)).await; // Voicemeeter-like bursts of 4
        }
    }
    let status = wait_for(&service, "12 packets", |s| s.mics[0].packets == 12).await;
    let mic = &status.mics[0];
    assert_eq!(
        (mic.state, mic.format, mic.frames_per_packet),
        (MicState::Receiving, Some(MONO_48K), Some(256))
    );
    assert_eq!(mic.sender, Some(tx.local_addr().unwrap().to_string()));
}
