use std::net::SocketAddr;

use presenter_core::audio::sequence::SequenceVerdict;
use presenter_core::audio::vban::{VbanAudioFormat, VbanSampleType};

use super::*;
use crate::listen::health::{ListenHealth, PacketSeen};
use crate::tracing_capture::Capture;

const FORMAT: VbanAudioFormat = VbanAudioFormat {
    rate_hz: 48_000,
    channels: 1,
    sample_type: VbanSampleType::Int16,
};

fn packet(health: &mut ListenHealth, mic: &str, verdict: SequenceVerdict) {
    let from: SocketAddr = "192.0.2.10:6980".parse().unwrap();
    let seen = PacketSeen {
        from,
        format: FORMAT,
        frames: 256,
        at: Instant::now(),
    };
    health.record_packet(mic, seen, verdict, false);
}

fn packets_per_line(health: &mut ListenHealth) -> Vec<(String, u64)> {
    health
        .take_minute_summaries(Instant::now())
        .into_iter()
        .map(|(id, _, s)| (id, s.packets))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_minute_reports_max_power_mean_voiced_share_and_deltas_then_resets() {
    let mut health = ListenHealth::default();
    let generation = health.set_mics(&[], &["guest"]);
    for _ in 0..4 {
        packet(&mut health, "guest", SequenceVerdict::InOrder);
    }
    packet(&mut health, "guest", SequenceVerdict::Late);
    let gap = SequenceVerdict::Gap {
        missing: 3,
        silence_frames: 768,
    };
    packet(&mut health, "guest", gap);
    // -30 and -60 dBFS: max -30, power mean 10*log10((1e-3 + 1e-6)/2) = -33.0,
    // one of the two batches voiced (> -50).
    health.record_chain("guest", generation, 1, Some(-30.0), None);
    health.record_chain("guest", generation, 1, Some(-60.0), None);
    // a closed chain of another generation is ignored
    health.record_chain("guest", generation + 1, 1, Some(-1.0), None);
    let lines = health.take_minute_summaries(Instant::now());
    assert_eq!(lines.len(), 1);
    let (id, _state, s) = &lines[0];
    assert_eq!(id, "guest");
    assert_eq!((s.packets, s.late, s.lost), (6, 1, 3));
    assert_eq!(s.level_dbfs_max, -30.0);
    assert!(
        (s.level_dbfs_avg + 33.0).abs() < 0.05,
        "{}",
        s.level_dbfs_avg
    );
    assert_eq!(s.voiced_pct, 50);
    // the next minute starts from zero: one packet, no batch
    packet(&mut health, "guest", SequenceVerdict::InOrder);
    let lines = health.take_minute_summaries(Instant::now());
    let s = lines[0].2;
    assert_eq!((s.packets, s.late, s.lost, s.voiced_pct), (1, 0, 0, 0));
    assert_eq!(s.level_dbfs_max, SILENCE_DBFS);
}

#[tokio::test(start_paused = true)]
async fn a_silent_mic_gets_one_zero_line_then_nothing_until_packets_return() {
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest", "idle"]);
    packet(&mut health, "guest", SequenceVerdict::InOrder);
    // "idle" never had packets and says nothing
    assert_eq!(
        packets_per_line(&mut health),
        vec![("guest".to_string(), 1)]
    );
    assert_eq!(
        packets_per_line(&mut health),
        vec![("guest".to_string(), 0)]
    );
    assert!(packets_per_line(&mut health).is_empty());
    packet(&mut health, "guest", SequenceVerdict::InOrder);
    assert_eq!(
        packets_per_line(&mut health),
        vec![("guest".to_string(), 1)]
    );
}

#[tokio::test(start_paused = true)]
async fn the_supervisor_logs_one_info_line_per_mic_per_minute() {
    let capture = Capture::install();
    let service = ListenService::new();
    service.with_health(|health| {
        let generation = health.set_mics(&[], &["guest"]);
        packet(health, "guest", SequenceVerdict::InOrder);
        health.record_chain("guest", generation, 1, Some(-40.0), None);
    });
    let task = tokio::spawn(supervise(service.clone()));
    // let the task arm its interval before the clock moves
    tokio::task::yield_now().await;
    tokio::time::advance(SUMMARY_EVERY + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let lines = capture.events_with_message("mic minute summary");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(line.level, tracing::Level::INFO);
    assert_eq!(line.target, MIC_TARGET);
    assert_eq!(line.field("mic"), Some("guest"));
    assert_eq!(line.field("packets"), Some("1"));
    assert_eq!(line.field("level_dbfs_max"), Some("-40.0"));
    assert_eq!(line.field("voiced_pct"), Some("100"));
    assert!(line.field("state").is_some());
    // a silent minute: one zero line, then none
    for _ in 0..2 {
        tokio::time::advance(SUMMARY_EVERY).await;
        tokio::task::yield_now().await;
    }
    let lines = capture.events_with_message("mic minute summary");
    let packets: Vec<_> = lines.iter().filter_map(|l| l.field("packets")).collect();
    assert_eq!(packets, vec!["1", "0"]);
    task.abort();
}
