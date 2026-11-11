//! The Dante mics (#852): the tap's continuity rule and rings, the reframing
//! of dlink-sized blocks into the chain's exact 100 ms frames, dlink's own
//! config check of the device Presenter asks for, and the supervisor driven
//! through a fake device (the only mock: the external `dlink` boundary — the
//! tap, drain, chain, health and frame channel are the real ones).

use std::f32::consts::PI;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use presenter_core::audio::level::rms_dbfs;
use presenter_core::audio::vban::f32_to_int16;
use presenter_core::listen::mics::{
    DanteInput, MicChannel, MicConfig, MicLanguage, MicRole, MicTransport,
};
use presenter_core::listen::status::{DanteState, MicState};

use super::link::{dlink_config, ChannelFlow, DanteBackend, LinkPlan, RunningLink};
use super::tap::{continuity, tap, Continuity, Tap, RING_SAMPLES};
use super::*;
use crate::listen::mic_chain::{Block, MicChain};

/// dlink's app block at 16 kHz on a 48 kHz network: 128 device frames → 42⅔.
const DLINK_BLOCKS: [usize; 3] = [43, 43, 42];

fn tone(samples: usize, offset: usize) -> Vec<f64> {
    (0..samples)
        .map(|i| 0.5 * (2.0 * PI * 1_000.0 * (offset + i) as f32 / 16_000.0).sin() as f64)
        .collect()
}

/// One media-thread call of `tap` on channel 0 = `input`, at media time `start`.
fn feed(tap: &mut Tap, input: &[f64], channels: usize, start: Option<u64>) {
    let frames = input.len();
    let all: Vec<f64> = (0..channels).flat_map(|_| input.iter().copied()).collect();
    let mut out: Vec<f64> = Vec::new();
    let block = dlink::Block::new(frames, &all, &mut out);
    let mut block = match start {
        Some(start) => block.at(start),
        None => block,
    };
    dlink::Process::process(tap, &mut block);
}

#[test]
fn the_continuity_rule_inserts_up_to_one_second_and_resyncs_beyond() {
    assert_eq!(continuity(None, Some(5), 16_000), Continuity::InOrder);
    assert_eq!(continuity(Some(5), None, 16_000), Continuity::InOrder);
    assert_eq!(
        continuity(Some(100), Some(100), 16_000),
        Continuity::InOrder
    );
    assert_eq!(continuity(Some(100), Some(101), 16_000), Continuity::Gap(1));
    assert_eq!(
        continuity(Some(100), Some(16_100), 16_000),
        Continuity::Gap(16_000)
    );
    assert_eq!(
        continuity(Some(100), Some(16_101), 16_000),
        Continuity::Resync
    );
    assert_eq!(
        continuity(Some(100), Some(99), 16_000),
        Continuity::Resync,
        "backwards"
    );
}

#[test]
fn dlink_sized_blocks_become_exact_100_ms_frames_with_nothing_lost() {
    let (mut tap, mut outputs) = tap(1);
    let mut chain = MicChain::new(MicChannel::Left);
    let mut fed: Vec<f32> = Vec::new();
    let mut frames: Vec<Vec<i16>> = Vec::new();
    let mut start = 0u64;
    // 1 s of audio: 375 blocks of 43/43/42 frames, drained every ~20 ms
    // (8 blocks ≈ 341 samples) as the supervisor does.
    for k in 0..375 {
        let size = DLINK_BLOCKS[k % 3];
        let input = tone(size, start as usize);
        fed.extend(input.iter().map(|&x| x as f32));
        feed(&mut tap, &input, 1, Some(start));
        start += size as u64;
        if k % 8 == 7 {
            let samples = outputs.drain(0);
            frames.extend(chain.push(&Block {
                format: DANTE_BLOCK_FORMAT,
                silence_frames: 0,
                samples,
            }));
        }
    }
    frames.extend(chain.push(&Block {
        format: DANTE_BLOCK_FORMAT,
        silence_frames: 0,
        samples: outputs.drain(0),
    }));
    assert_eq!(fed.len(), 16_000, "375 blocks of 42⅔ frames = 1 s");
    assert_eq!(frames.len(), 10, "ten whole 100 ms frames");
    assert!(frames.iter().all(|frame| frame.len() == 1_600));
    let expected: Vec<i16> = fed.iter().copied().map(f32_to_int16).collect();
    assert_eq!(
        frames.concat(),
        expected,
        "16 kHz passes through untouched, in order"
    );
    let level = rms_dbfs(&frames[9]);
    assert!(
        (level - -9.03).abs() < 0.05,
        "a half-scale tone, got {level}"
    );
    let counters = &outputs.counters;
    assert_eq!(counters.overflow_samples.load(Ordering::Relaxed), 0);
    assert_eq!(counters.gaps.load(Ordering::Relaxed), 0);
}

#[test]
fn a_skipped_stretch_is_silence_and_a_long_jump_is_a_resync() {
    let (mut tap, mut outputs) = tap(2);
    feed(&mut tap, &[0.25; 43], 2, Some(1_000));
    feed(&mut tap, &[0.25; 43], 2, Some(1_043 + 100));
    for channel in 0..2 {
        let samples = outputs.drain(channel);
        assert_eq!(samples.len(), 43 + 100 + 43, "channel {channel}");
        assert!(
            samples[43..143].iter().all(|&x| x == 0.0),
            "the gap is silence"
        );
        assert!(samples[143..].iter().all(|&x| x == 0.25));
    }
    feed(&mut tap, &[0.25; 43], 2, Some(1_186 + 16_001));
    assert_eq!(
        outputs.drain(0).len(),
        43,
        "a jump over 1 s inserts nothing"
    );
    assert_eq!(outputs.drain(1).len(), 43);
    assert_eq!(outputs.counters.gaps.load(Ordering::Relaxed), 1);
    assert_eq!(outputs.counters.resyncs.load(Ordering::Relaxed), 1);
    dlink::Process::discontinuity(&mut tap);
    assert_eq!(
        outputs.counters.resyncs.load(Ordering::Relaxed),
        2,
        "dlink's own skip signal is counted as a resync"
    );
    feed(&mut tap, &[0.25; 43], 2, Some(5));
    assert_eq!(
        outputs.drain(1).len(),
        43,
        "after a reopen the next block starts over"
    );
}

#[test]
fn a_full_ring_drops_what_does_not_fit_and_counts_it() {
    let (mut tap, mut outputs) = tap(1);
    let mut start = 0;
    while start < RING_SAMPLES as u64 + 430 {
        feed(&mut tap, &[0.1; 43], 1, Some(start));
        start += 43;
    }
    let dropped = outputs.counters.overflow_samples.load(Ordering::Relaxed);
    assert_eq!(outputs.drain(0).len(), RING_SAMPLES);
    assert_eq!(dropped, start - RING_SAMPLES as u64);
}

fn input(channel: &str, source: Option<&str>) -> DanteInput {
    DanteInput {
        channel: channel.to_string(),
        source: source.map(str::to_string),
    }
}

fn plan(rate: u32, device: &str) -> LinkPlan {
    LinkPlan {
        device: device.to_string(),
        interface: "enp2s0.70".to_string(),
        network_rate: rate,
        state_file: std::env::temp_dir().join("presenter-dante-test.toml"),
        channels: vec![
            input("Host EN", Some("Mic 1@stagebox-1")),
            input("Tlmocnik", None),
        ],
    }
}

#[test]
fn dlink_accepts_the_device_presenter_asks_for() {
    for rate in [96_000, 48_000] {
        let config = dlink_config(&plan(rate, DEFAULT_DEVICE_NAME));
        assert!(config.converts(), "16 kHz app on a {rate} Hz network");
        assert_eq!(config.device_rate(), rate);
        let device = config.device();
        assert!(device.is_ok(), "{rate}: {device:?}");
        let split = config.split().unwrap();
        assert!(split.rx_path_ns <= dlink::BUDGET_NS, "{split:?}");
    }
    let refused = dlink_config(&plan(48_000, "bad name!")).device();
    assert!(
        refused.is_err_and(|problems| problems.iter().any(|p| p.contains("name"))),
        "dlink names the problem of a bad device name"
    );
}

#[test]
fn the_env_settings_default_and_place_the_state_file_next_to_the_database() {
    let env = DanteEnv::parse(
        Some("  "),
        None,
        None,
        "sqlite:///opt/presenter/presenter.db",
    );
    assert_eq!(env.interface, None, "blank = no Dante on this box");
    assert_eq!(env.device, "presenter-listen");
    assert_eq!(env.network_rate, 96_000);
    assert_eq!(
        env.state_file,
        std::path::Path::new("/opt/presenter/dante-listen.toml")
    );
    let env = DanteEnv::parse(Some(" enp2s0.70 "), Some("snv-listen"), Some("48000"), "");
    assert_eq!(
        (
            env.interface.as_deref(),
            env.device.as_str(),
            env.network_rate
        ),
        (Some("enp2s0.70"), "snv-listen", 48_000)
    );
    assert_eq!(
        DanteEnv::parse(None, None, Some("fast"), "").network_rate,
        96_000
    );
}

// ---- the supervisor, through a fake device -------------------------------

#[derive(Default)]
struct FakeDevice {
    starts: Mutex<Vec<LinkPlan>>,
    stops: AtomicUsize,
    tap: Mutex<Option<Tap>>,
    flows: Mutex<Vec<ChannelFlow>>,
    ended: Mutex<Option<String>>,
    refuse: Mutex<Option<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct FakeBackend(Arc<FakeDevice>);
struct FakeLink(Arc<FakeDevice>);

impl DanteBackend for FakeBackend {
    fn start(&self, plan: &LinkPlan, tap: Tap) -> Result<Box<dyn RunningLink>, String> {
        if let Some(error) = lock(&self.0.refuse).clone() {
            return Err(error);
        }
        lock(&self.0.starts).push(plan.clone());
        *lock(&self.0.tap) = Some(tap);
        *lock(&self.0.flows) = vec![ChannelFlow::default(); plan.channels.len()];
        *lock(&self.0.ended) = None;
        Ok(Box::new(FakeLink(Arc::clone(&self.0))))
    }
}

impl RunningLink for FakeLink {
    fn flows(&self) -> Vec<ChannelFlow> {
        lock(&self.0.flows).clone()
    }
    fn ended(&self) -> Option<String> {
        lock(&self.0.ended).clone()
    }
    fn stop(self: Box<Self>) {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
    }
}

fn dante(id: &str, name: &str, channel: &str, source: Option<&str>) -> MicConfig {
    MicConfig {
        id: id.to_string(),
        name: name.to_string(),
        stream_name: String::new(),
        sender_ip: None,
        channel: MicChannel::Left,
        language: MicLanguage::En,
        role: MicRole::Guest,
        transport: MicTransport::Dante,
        dante: Some(DanteInput {
            channel: channel.to_string(),
            source: source.map(str::to_string),
        }),
    }
}

fn env(interface: Option<&str>) -> DanteEnv {
    DanteEnv::parse(interface, None, Some("48000"), "")
}

fn started(device: &FakeDevice) -> usize {
    lock(&device.starts).len()
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting until {what}");
}

/// The fake device's media thread: `blocks` dlink-sized blocks of a tone into
/// channel 0, its `written` count advanced like dlink's RX does.
fn play(device: &FakeDevice, start: &mut u64, blocks: usize) {
    let mut tap = lock(&device.tap);
    let Some(tap) = tap.as_mut() else { return };
    let channels = lock(&device.flows).len();
    for k in 0..blocks {
        let input = tone(DLINK_BLOCKS[k % 3], *start as usize);
        feed(tap, &input, channels, Some(*start));
        *start += input.len() as u64;
    }
    for flow in lock(&device.flows).iter_mut() {
        flow.written = *start * 3;
        flow.source = Some("Mic 1@stagebox-1 (prijíma)".into());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dante_mic_starts_a_device_and_its_audio_reaches_the_mics_frame_channel() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    let backend: Arc<dyn DanteBackend> = Arc::new(FakeBackend(Arc::clone(&device)));
    tokio::spawn(supervise(service.clone(), backend, env(Some("enp2s0.70"))));
    wait_until("idle", || service.status().dante.state == DanteState::Idle).await;
    assert_eq!(started(&device), 0, "no Dante mic → no device");

    let mut frames = service.subscribe_frames("guest");
    service.apply(vec![dante(
        "guest",
        "Hosť",
        "Host EN",
        Some("Mic 1@stagebox-1"),
    )]);
    wait_until("the device starts", || started(&device) == 1).await;
    let plan = lock(&device.starts)[0].clone();
    assert_eq!(
        plan.channels,
        vec![input("Host EN", Some("Mic 1@stagebox-1"))]
    );
    assert_eq!(
        (plan.interface.as_str(), plan.network_rate),
        ("enp2s0.70", 48_000)
    );
    wait_until("running", || {
        service.status().dante.state == DanteState::Running
    })
    .await;
    assert_eq!(
        service.status().mics[0].state,
        MicState::Waiting,
        "no flow yet"
    );

    let mut start = 0;
    let mut got = Vec::new();
    for _ in 0..200 {
        play(&device, &mut start, 9);
        tokio::time::sleep(Duration::from_millis(15)).await;
        while let Ok(frame) = frames.try_recv() {
            got.push(frame);
        }
        if got.len() >= 3 {
            break;
        }
    }
    assert!(got.len() >= 3, "frames reached the channel: {}", got.len());
    assert!(got.iter().all(|frame| frame.len() == 1_600));
    let status = service.status();
    let mic = &status.mics[0];
    assert_eq!(mic.state, MicState::Receiving);
    assert_eq!(mic.sender.as_deref(), Some("Mic 1@stagebox-1 (prijíma)"));
    assert_eq!(mic.format, Some(DANTE_BLOCK_FORMAT));
    assert!(mic.frames_out >= 3 && mic.packets > 0, "{mic:?}");
    assert!(
        mic.level_dbfs.is_some_and(|l| (l - -9.03).abs() < 0.1),
        "{mic:?}"
    );

    service.apply(vec![dante(
        "guest",
        "Hosť (premenovaný)",
        "Host EN",
        Some("Mic 1@stagebox-1"),
    )]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(started(&device), 1, "a rename keeps the device");
    assert!(
        service.status().mics[0].frames_out >= mic.frames_out,
        "and the mic's counters"
    );

    service.apply(vec![dante("guest", "Hosť", "Guest 2", None)]);
    wait_until("a new channel restarts the device", || {
        started(&device) == 2
    })
    .await;
    assert_eq!(device.stops.load(Ordering::SeqCst), 1);

    service.apply(Vec::new());
    wait_until("the last Dante mic stops the device", || {
        device.stops.load(Ordering::SeqCst) == 2
    })
    .await;
    wait_until("idle again", || {
        service.status().dante.state == DanteState::Idle
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_channel_never_reads_as_receiving() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    tokio::spawn(supervise(
        service.clone(),
        Arc::new(FakeBackend(Arc::clone(&device))),
        env(Some("enp2s0.70")),
    ));
    service.apply(vec![dante("guest", "Hosť", "Host EN", None)]);
    wait_until("the device starts", || started(&device) == 1).await;
    // the pump runs without a flow: blocks arrive, `written` never advances
    let mut start = 0;
    for _ in 0..20 {
        let flows_before = lock(&device.flows).clone();
        play(&device, &mut start, 9);
        *lock(&device.flows) = flows_before;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let mic = &service.status().mics[0];
    assert_eq!(
        (mic.state, mic.packets, mic.frames_out),
        (MicState::Waiting, 0, 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_dante_interface_no_device_starts_and_the_mic_says_why() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    tokio::spawn(supervise(
        service.clone(),
        Arc::new(FakeBackend(Arc::clone(&device))),
        env(None),
    ));
    service.apply(vec![dante("guest", "Hosť", "Host EN", None)]);
    wait_until("no interface", || {
        service.status().dante.state == DanteState::NoInterface
    })
    .await;
    let status = service.status();
    assert!(status.mics[0]
        .error
        .as_deref()
        .is_some_and(|e| e.contains("PRESENTER_LISTEN_DANTE_IFACE")));
    assert_eq!(started(&device), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_start_is_retried_and_shown() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    *lock(&device.refuse) = Some("no such interface".into());
    tokio::spawn(supervise(
        service.clone(),
        Arc::new(FakeBackend(Arc::clone(&device))),
        env(Some("eth9")),
    ));
    service.apply(vec![dante("guest", "Hosť", "Host EN", None)]);
    wait_until("retrying", || {
        service.status().dante.state == DanteState::Retrying
    })
    .await;
    let status = service.status();
    assert!(status
        .dante
        .error
        .as_deref()
        .is_some_and(|e| e.contains("no such interface")));
    assert!(status.mics[0]
        .error
        .as_deref()
        .is_some_and(|e| e.contains("no such interface")));
    *lock(&device.refuse) = None;
    wait_until("the retry after 1 s starts it", || started(&device) == 1).await;
    wait_until("running", || {
        service.status().dante.state == DanteState::Running
    })
    .await;
    assert_eq!(service.status().mics[0].error, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vban_mic_beside_a_dante_mic_keeps_both_counters() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    tokio::spawn(supervise(
        service.clone(),
        Arc::new(FakeBackend(Arc::clone(&device))),
        env(Some("enp2s0.70")),
    ));
    // the REAL VBAN socket task beside it, on a loopback port
    tokio::spawn(crate::listen::vban_rx::supervise(
        service.clone(),
        "127.0.0.1:0".parse().unwrap(),
    ));
    let mut vban = dante("interp", "Tlmočník", "x", None);
    vban.transport = MicTransport::Vban;
    vban.dante = None;
    vban.stream_name = "INTERP_SK".into();
    service.apply(vec![vban.clone(), dante("guest", "Hosť", "Host EN", None)]);
    wait_until("the device starts", || started(&device) == 1).await;
    assert_eq!(
        lock(&device.starts)[0].channels.len(),
        1,
        "only the Dante mic is a channel"
    );
    wait_until("the VBAN socket binds", || {
        service.status().socket.state == presenter_core::listen::status::SocketState::Bound
    })
    .await;
    let mut start = 0;
    wait_until_frames(&service, &device, &mut start).await;
    let frames_of = |service: &ListenService, id: &str| {
        service
            .status()
            .mics
            .into_iter()
            .find(|m| m.config.id == id)
            .map(|m| m.frames_out)
    };
    let before = frames_of(&service, "guest").unwrap_or(0);
    assert!(before > 0);
    // the VBAN mic goes: its socket closes and ends its session
    service.apply(vec![dante("guest", "Hosť", "Host EN", None)]);
    wait_until("the VBAN socket closes", || {
        service.status().socket.state == presenter_core::listen::status::SocketState::Idle
    })
    .await;
    assert!(
        frames_of(&service, "guest").is_some_and(|n| n >= before),
        "a closed VBAN socket keeps the Dante mic's counters"
    );
    assert_eq!(started(&device), 1, "and the Dante device runs on");
    // the VBAN mic comes back beside the Dante mic: the device is not touched
    service.apply(vec![vban, dante("guest", "Hosť", "Host EN", None)]);
    wait_until("the VBAN socket binds again", || {
        service.status().socket.state == presenter_core::listen::status::SocketState::Bound
    })
    .await;
    assert!(frames_of(&service, "guest").is_some_and(|n| n >= before));
    assert_eq!(frames_of(&service, "interp"), Some(0));
    assert_eq!(started(&device), 1);
}

#[test]
fn one_mic_id_switching_transport_is_owned_by_one_receiver_at_a_time() {
    use crate::listen::health::ListenHealth;
    for vban_first in [true, false] {
        let mut health = ListenHealth::default();
        let dante_gen = health.set_transport_mics(MicTransport::Dante, &[], &["guest"]);
        health.record_dante_block("guest", 43, None, false, Instant::now());
        // the save makes it a VBAN mic: each receiver applies its own list
        let apply_vban = |h: &mut ListenHealth| h.set_mics(&[], &["guest"]);
        let apply_dante =
            |h: &mut ListenHealth| h.set_transport_mics(MicTransport::Dante, &[], &[]);
        let vban_gen = if vban_first {
            let g = apply_vban(&mut health);
            apply_dante(&mut health);
            g
        } else {
            apply_dante(&mut health);
            apply_vban(&mut health)
        };
        assert!(
            !health.record_chain("guest", dante_gen, 1, None, None),
            "the old Dante chain can no longer write ({vban_first})"
        );
        assert!(health.record_chain("guest", vban_gen, 1, None, None));
        health.record_dante_block("guest", 43, None, false, Instant::now());
        let status = health.snapshot(&[dante("guest", "Hosť", "x", None)], Instant::now());
        assert_eq!(status.mics[0].packets, 0, "a VBAN mic takes no Dante block");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_ends_by_itself_is_restarted_and_a_new_device_starts_silent() {
    let service = ListenService::new();
    let device = Arc::new(FakeDevice::default());
    tokio::spawn(supervise(
        service.clone(),
        Arc::new(FakeBackend(Arc::clone(&device))),
        env(Some("enp2s0.70")),
    ));
    service.apply(vec![dante("guest", "Hosť", "Host EN", None)]);
    wait_until("the device starts", || started(&device) == 1).await;
    let mut start = 0;
    wait_until_frames(&service, &device, &mut start).await;
    *lock(&device.ended) = Some("the interface went away".into());
    wait_until("retrying", || {
        service.status().dante.state == DanteState::Retrying
    })
    .await;
    assert!(service
        .status()
        .dante
        .error
        .is_some_and(|e| e.contains("the interface went away")));
    assert_eq!(device.stops.load(Ordering::SeqCst), 1);
    wait_until("the retry starts a new device", || started(&device) == 2).await;
    wait_until("running", || {
        service.status().dante.state == DanteState::Running
    })
    .await;
    // the new device's pump runs without a flow: its silence is never audio,
    // even within a second of the old device's last live block
    let packets = service.status().mics[0].packets;
    for _ in 0..10 {
        if let Some(tap) = lock(&device.tap).as_mut() {
            feed(tap, &tone(43, 0), 1, None);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(service.status().mics[0].packets, packets);
}

async fn wait_until_frames(service: &ListenService, device: &FakeDevice, start: &mut u64) {
    for _ in 0..300 {
        play(device, start, 9);
        tokio::time::sleep(Duration::from_millis(15)).await;
        if service.status().mics.iter().any(|m| m.frames_out > 0) {
            return;
        }
    }
    panic!("no frames");
}
