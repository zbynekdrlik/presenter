//! #852 C3-7 supervisor tests: the real `ListenService` and supervisor, a
//! fake `StreamingAsr` (records each open and the frames it got, answers a
//! closed input with `Finished(None)`), a fake store, the paused clock and a
//! settable local clock for the day.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime, Utc};
use presenter_core::listen::context::ContextInputs;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::transcript::{AsrToken, TokenBatch};
use presenter_core::listen::transcription::{
    next_override, MicUsage, OffReason, SessionMode, SessionOverride, TranscriptionSetting,
    TranscriptionState, UsageRow,
};
use tokio::sync::{mpsc, RwLock};

use super::{TranscriptionDeps, TranscriptionStore};
use crate::listen::asr::{
    AsrEvent, AsrInput, AsrSession, AsrSessionConfig, StreamingAsr, EVENT_CAPACITY, INPUT_CAPACITY,
};
use crate::listen::recorder::RecorderEnv;
use crate::listen::{ListenService, MicFrame};
use crate::tracing_capture::Capture;

const DAY: &str = "2026-11-22";

/// One session the fake opened.
struct Opened {
    cfg: AsrSessionConfig,
    events: mpsc::Sender<AsrEvent>,
    /// The first sample of every frame it read.
    frames: Arc<Mutex<Vec<i16>>>,
    closed: Arc<AtomicBool>,
    /// A stuck session's input, held open and never read.
    _unread: Option<mpsc::Receiver<AsrInput>>,
}

#[derive(Default)]
struct FakeAsr {
    opened: Mutex<Vec<Opened>>,
    /// Never reads its input (a session that fell behind).
    stuck: bool,
}

impl StreamingAsr for FakeAsr {
    fn open(&self, cfg: AsrSessionConfig) -> AsrSession {
        let (input, mut inputs) = mpsc::channel(INPUT_CAPACITY);
        let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let frames = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let mut unread = None;
        if self.stuck {
            unread = Some(inputs);
        } else {
            let (frames, closed, events_tx) =
                (Arc::clone(&frames), Arc::clone(&closed), events_tx.clone());
            tokio::spawn(async move {
                while let Some(AsrInput::Frame { pcm, .. }) = inputs.recv().await {
                    frames.lock().unwrap().push(pcm[0]);
                }
                closed.store(true, Ordering::SeqCst);
                let _ = events_tx.send(AsrEvent::Finished(None)).await;
            });
        }
        self.opened.lock().unwrap().push(Opened {
            cfg,
            events: events_tx,
            frames,
            closed,
            _unread: unread,
        });
        AsrSession { input, events }
    }
}

impl FakeAsr {
    fn opens(&self) -> usize {
        self.opened.lock().unwrap().len()
    }

    fn events(&self, n: usize) -> mpsc::Sender<AsrEvent> {
        self.opened.lock().unwrap()[n].events.clone()
    }

    fn closed(&self, n: usize) -> bool {
        self.opened.lock().unwrap()[n].closed.load(Ordering::SeqCst)
    }

    fn frames(&self, n: usize) -> Vec<i16> {
        self.opened.lock().unwrap()[n]
            .frames
            .lock()
            .unwrap()
            .clone()
    }
}

#[derive(Default)]
struct FakeStore {
    inputs: ContextInputs,
    saved: Mutex<Vec<UsageRow>>,
}

#[async_trait]
impl TranscriptionStore for FakeStore {
    async fn context_inputs(&self, _today: NaiveDate) -> anyhow::Result<ContextInputs> {
        Ok(self.inputs.clone())
    }

    async fn save_usage(&self, row: UsageRow) -> anyhow::Result<()> {
        self.saved.lock().unwrap().push(row);
        Ok(())
    }
}

struct Rig {
    listen: ListenService,
    asr: Arc<FakeAsr>,
    store: Arc<FakeStore>,
    layout: Arc<RwLock<String>>,
    now: Arc<Mutex<NaiveDateTime>>,
}

fn mics() -> Vec<MicConfig> {
    serde_json::from_str(
        r#"[{"id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null,
             "channel": "left", "language": "en", "role": "guest"},
            {"id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null,
             "channel": "left", "language": "sk", "role": "interpreter"}]"#,
    )
    .unwrap()
}

fn on(cap: u32) -> TranscriptionSetting {
    TranscriptionSetting {
        enabled: true,
        guest_mic: Some("guest".to_string()),
        daily_cap_minutes: cap,
    }
}

fn day() -> NaiveDate {
    DAY.parse().unwrap()
}

fn choice(mode: SessionMode) -> SessionOverride {
    SessionOverride {
        day: day(),
        mode,
        extra_minutes: 0,
    }
}

/// A service with the two mics, `setting`, `usage` and the supervisor started.
fn rig_with(
    setting: TranscriptionSetting,
    usage: Option<UsageRow>,
    key: bool,
    asr: FakeAsr,
    inputs: ContextInputs,
) -> Rig {
    let listen = ListenService::new();
    let now = Arc::new(Mutex::new(
        format!("{DAY}T10:00:00").parse::<NaiveDateTime>().unwrap(),
    ));
    let read = Arc::clone(&now);
    listen.set_recorder_env(RecorderEnv {
        root: std::env::temp_dir().join("presenter-recorder-never-written"),
        clock: Arc::new(move || *read.lock().unwrap()),
        first_pass_delay: Duration::ZERO,
    });
    listen.apply(mics());
    listen.transcription().load(setting, None);
    listen.transcription().load_usage(usage, day());
    let asr = Arc::new(asr);
    let store = Arc::new(FakeStore {
        inputs,
        ..FakeStore::default()
    });
    let layout = Arc::new(RwLock::new("lyrics".to_string()));
    assert!(listen.start_transcription_with(TranscriptionDeps {
        asr: Arc::clone(&asr) as Arc<dyn StreamingAsr>,
        key,
        model: "stt-rt-v6".to_string(),
        layout: Arc::clone(&layout),
        store: Arc::clone(&store) as Arc<dyn TranscriptionStore>,
    }));
    Rig {
        listen,
        asr,
        store,
        layout,
        now,
    }
}

fn rig(setting: TranscriptionSetting) -> Rig {
    rig_with(
        setting,
        None,
        true,
        FakeAsr::default(),
        ContextInputs::default(),
    )
}

async fn wait(seconds: u64) {
    tokio::time::sleep(Duration::from_millis(seconds * 1000 + 100)).await;
}

impl Rig {
    async fn set_layout(&self, layout: &str) {
        *self.layout.write().await = layout.to_string();
    }

    /// The guest's status line: (state, reason).
    fn guest(&self) -> (TranscriptionState, Option<OffReason>) {
        let status = self.listen.transcription().status(day());
        let guest = status.mics.first().expect("the guest mic is shown");
        (guest.state, guest.reason)
    }

    fn send(&self, mic: &str, value: i16) {
        let frame: MicFrame = Arc::from(vec![value; 1_600]);
        let _ = self.listen.frame_sender(mic).send(frame);
    }
}

#[tokio::test(start_paused = true)]
async fn nothing_starts_without_the_switch_the_guest_mic_the_key_and_the_layout() {
    let missing = TranscriptionSetting {
        guest_mic: Some("nobody".to_string()),
        ..on(240)
    };
    let full = Some(UsageRow {
        day: day(),
        mics: [(
            "guest".to_string(),
            MicUsage {
                audio_ms: 0,
                connected_ms: 240 * 60_000,
            },
        )]
        .into(),
    });
    let cases = [
        (on(240), None, true, OffReason::NotPreach),
        (
            TranscriptionSetting {
                enabled: false,
                ..on(240)
            },
            None,
            true,
            OffReason::Disabled,
        ),
        (missing, None, true, OffReason::GuestMicMissing),
        (on(240), None, false, OffReason::NoKey),
        (on(240), full, true, OffReason::CapReached),
    ];
    for (setting, usage, key, reason) in cases {
        let rig = rig_with(
            setting,
            usage,
            key,
            FakeAsr::default(),
            ContextInputs::default(),
        );
        wait(3).await;
        assert_eq!(rig.asr.opens(), 0, "{reason:?}");
        assert_eq!(rig.guest(), (TranscriptionState::Off, Some(reason)));
    }
    let no_mic = rig(TranscriptionSetting {
        guest_mic: None,
        ..on(240)
    });
    no_mic.set_layout("preach").await;
    wait(3).await;
    assert_eq!(no_mic.asr.opens(), 0);
    assert!(no_mic.listen.transcription().status(day()).mics.is_empty());
    let stopped_today = rig(on(240));
    stopped_today
        .listen
        .transcription()
        .set_override(choice(SessionMode::Off));
    stopped_today.set_layout("preach").await;
    wait(3).await;
    assert_eq!(stopped_today.asr.opens(), 0);
    assert_eq!(
        stopped_today.guest(),
        (TranscriptionState::Off, Some(OffReason::OverrideOff))
    );
}

#[tokio::test(start_paused = true)]
async fn preach_starts_the_guest_session_and_leaving_it_stops_after_sixty_seconds() {
    let rig = rig(on(240));
    rig.set_layout("preach").await;
    wait(1).await;
    assert_eq!(rig.asr.opens(), 1);
    assert_eq!(rig.guest(), (TranscriptionState::Armed, None));
    rig.set_layout("lyrics").await;
    wait(58).await;
    assert!(!rig.asr.closed(0), "still within the 60 s stop delay");
    wait(3).await;
    assert!(rig.asr.closed(0), "stopped after the delay");
    assert_eq!(
        rig.guest(),
        (TranscriptionState::Off, Some(OffReason::NotPreach))
    );
    rig.set_layout("preach").await;
    wait(1).await;
    assert_eq!(rig.asr.opens(), 2, "Kázeň again starts a new session");
}

#[tokio::test(start_paused = true)]
async fn an_on_override_starts_without_preach_and_off_stops_for_the_rest_of_the_day() {
    let rig = rig(on(240));
    wait(1).await;
    let generation = rig
        .listen
        .transcription()
        .set_override(choice(SessionMode::On));
    // What a PUT does before it answers: the answer shows the started session.
    rig.listen.transcription_decided_on(generation).await;
    assert_eq!(rig.asr.opens(), 1, "on runs without Kázeň");
    assert_eq!(rig.guest(), (TranscriptionState::Armed, None));
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::Off));
    rig.set_layout("preach").await;
    wait(5).await;
    assert!(rig.asr.closed(0));
    assert_eq!(rig.asr.opens(), 1, "off wins over Kázeň today");
    assert_eq!(
        rig.guest().1,
        Some(OffReason::OverrideOff),
        "today's choice is shown"
    );
    *rig.now.lock().unwrap() = "2026-11-23T08:00:00".parse().unwrap();
    wait(2).await;
    assert_eq!(rig.asr.opens(), 2, "the next day follows Kázeň again");
}

#[tokio::test(start_paused = true)]
async fn only_the_guest_mic_frames_reach_the_session() {
    let rig = rig(on(240));
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(1).await;
    for _ in 0..5 {
        rig.send("interp", 7);
        rig.send("guest", 3);
    }
    wait(1).await;
    assert_eq!(rig.asr.frames(0), [3; 5]);
}

#[tokio::test(start_paused = true)]
async fn the_daily_cap_stops_the_guest_session_and_a_start_adds_an_hour() {
    let rig = rig(on(10));
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    rig.asr
        .events(0)
        .send(AsrEvent::Usage {
            audio_ms: 300_000,
            connected_ms: 600_000,
        })
        .await
        .unwrap();
    wait(2).await;
    assert!(rig.asr.closed(0), "10 connected minutes reach the cap");
    assert_eq!(
        rig.guest(),
        (TranscriptionState::Off, Some(OffReason::CapReached))
    );
    assert!(transcription.cap_reached(day()));
    let start = next_override(
        transcription.rows().over.as_ref(),
        day(),
        SessionMode::On,
        true,
    );
    assert_eq!(start.extra_minutes, 60);
    transcription.set_override(start);
    wait(1).await;
    assert_eq!(
        rig.asr.opens(),
        2,
        "a start after the cap runs one more hour"
    );
    let status = transcription.status(day());
    assert_eq!(status.mics[0].cap_ms, 70 * 60_000);
    assert_eq!(status.mics[0].connected_ms_today, 600_000);
    assert_eq!(status.mics[0].audio_ms_today, 300_000);
}

#[tokio::test(start_paused = true)]
async fn usage_survives_a_restart_the_same_day() {
    let first = rig(on(10));
    first
        .listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(1).await;
    first
        .asr
        .events(0)
        .send(AsrEvent::Usage {
            audio_ms: 1_000,
            connected_ms: 600_000,
        })
        .await
        .unwrap();
    wait(2).await;
    let saved = first.store.saved.lock().unwrap().last().cloned();
    let row = saved.expect("the session end writes listen.usage");
    assert_eq!(row.day, day());
    assert_eq!(row.mics["guest"].connected_ms, 600_000);
    let second = rig_with(
        on(10),
        Some(row),
        true,
        FakeAsr::default(),
        ContextInputs::default(),
    );
    second
        .listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(2).await;
    assert_eq!(
        second.asr.opens(),
        0,
        "the cap stays reached after a restart"
    );
    assert_eq!(second.guest().1, Some(OffReason::CapReached));
}

#[tokio::test(start_paused = true)]
async fn a_provider_stop_blocks_the_mic_until_the_setting_changes() {
    let rig = rig(on(240));
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    let events = rig.asr.events(0);
    events
        .send(AsrEvent::State {
            state: TranscriptionState::Stopped,
            error: Some("401 unauthorized".to_string()),
        })
        .await
        .unwrap();
    events
        .send(AsrEvent::Finished(Some(OffReason::Auth)))
        .await
        .unwrap();
    wait(10).await;
    assert_eq!(rig.asr.opens(), 1, "no restart loop");
    let status = transcription.status(day());
    assert_eq!(status.mics[0].state, TranscriptionState::Stopped);
    assert_eq!(status.mics[0].reason, Some(OffReason::Auth));
    assert_eq!(status.mics[0].error.as_deref(), Some("401 unauthorized"));
    transcription.set_setting(on(300));
    wait(2).await;
    assert_eq!(rig.asr.opens(), 1, "the cap is not what a person fixes");
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    assert_eq!(rig.asr.opens(), 2, "a new choice runs it again");
}

#[tokio::test(start_paused = true)]
async fn the_context_carries_todays_prepared_references_and_tip_terms() {
    let inputs = ContextInputs {
        prepared_refs: vec!["Romans 8:37".to_string()],
        tip_terms: vec!["Shuttlesworth".to_string()],
    };
    let rig = rig_with(on(240), None, true, FakeAsr::default(), inputs);
    rig.set_layout("preach").await;
    wait(1).await;
    let opened = rig.asr.opened.lock().unwrap();
    let cfg = &opened[0].cfg;
    assert_eq!(cfg.mic_id, "guest");
    let terms = &cfg.context.terms;
    let at = |t: &str| terms.iter().position(|term| term == t);
    assert_eq!(at("Genesis"), Some(0), "the books come first");
    assert!(at("Romans 8:37") > at("Revelation"), "{terms:?}");
    assert!(at("Shuttlesworth") > at("Romans 8:37"), "{terms:?}");
    assert!(at("Gethsemane") > at("Shuttlesworth"), "{terms:?}");
}

fn token(text: &str, start_ms: u64, is_final: bool) -> AsrToken {
    AsrToken {
        text: text.to_string(),
        start_ms: Some(start_ms),
        end_ms: Some(start_ms + 200),
        confidence: 0.9,
        is_final,
        language: Some("en".to_string()),
        is_translation: false,
    }
}

#[tokio::test(start_paused = true)]
async fn tokens_become_hub_sentences_and_a_tail() {
    let rig = rig(on(240));
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(1).await;
    let batch = TokenBatch {
        connection: 1,
        offset_ms: 0,
        tokens: vec![
            token("Grace", 0, true),
            token(" wins.", 300, true),
            token(" Now", 700, true),
            token(" we", 900, false),
        ],
        received_at: Utc::now(),
    };
    rig.asr
        .events(0)
        .send(AsrEvent::Tokens(batch))
        .await
        .unwrap();
    wait(1).await;
    let snapshot = rig.listen.transcripts().snapshot();
    let texts: Vec<&str> = snapshot.sentences.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["Grace wins."]);
    assert_eq!(snapshot.tails.len(), 1);
    assert_eq!(snapshot.tails[0].final_text, "Now");
    assert_eq!(snapshot.tails[0].partial_text, " we");
    let status = rig.listen.transcription().status(day());
    assert_eq!(status.mics[0].connections, 1);
    assert_eq!(snapshot.sessions[0].mic_id, "guest");
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::Off));
    wait(2).await;
    let snapshot = rig.listen.transcripts().snapshot();
    let texts: Vec<&str> = snapshot.sentences.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        ["Grace wins.", "Now"],
        "a stop closes the open words"
    );
    assert!(snapshot.tails.is_empty());
    assert_eq!(snapshot.sessions[0].reason, Some(OffReason::OverrideOff));
}

#[tokio::test(start_paused = true)]
async fn a_slow_session_never_blocks_the_chain() {
    let capture = Capture::install();
    let stuck = FakeAsr {
        stuck: true,
        ..FakeAsr::default()
    };
    let rig = rig_with(on(240), None, true, stuck, ContextInputs::default());
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(1).await;
    let mut other = rig.listen.subscribe_frames("guest");
    for value in 0..100 {
        rig.send("guest", value);
        tokio::task::yield_now().await;
    }
    wait(1).await;
    let mut seen = 0;
    while other.try_recv().is_ok() {
        seen += 1;
    }
    assert_eq!(seen, 100, "every frame still reaches the other consumers");
    let dropped = capture.events_with_message("frames dropped");
    assert!(!dropped.is_empty(), "a full session input counts drops");
    assert_eq!(dropped.last().unwrap().field("dropped"), Some("32"));
}

async fn usage(events: &mpsc::Sender<AsrEvent>, audio_ms: u64, connected_ms: u64) {
    let event = AsrEvent::Usage {
        audio_ms,
        connected_ms,
    };
    events.send(event).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn usage_totals_add_up_as_deltas_across_events_and_sessions() {
    let stored = UsageRow {
        day: day(),
        mics: [(
            "guest".to_string(),
            MicUsage {
                audio_ms: 1_000,
                connected_ms: 10_000,
            },
        )]
        .into(),
    };
    let rig = rig_with(
        on(240),
        Some(stored),
        true,
        FakeAsr::default(),
        ContextInputs::default(),
    );
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    usage(&rig.asr.events(0), 2_000, 100_000).await;
    usage(&rig.asr.events(0), 5_000, 250_000).await;
    wait(1).await;
    let first = transcription.status(day()).mics[0].clone();
    assert_eq!(
        (first.audio_ms_today, first.connected_ms_today),
        (6_000, 260_000),
        "running totals count once, on top of the stored day"
    );
    transcription.set_override(choice(SessionMode::Off));
    wait(2).await;
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    assert_eq!(rig.asr.opens(), 2);
    usage(&rig.asr.events(1), 500, 50_000).await;
    wait(1).await;
    let second = transcription.status(day()).mics[0].clone();
    assert_eq!(
        (second.audio_ms_today, second.connected_ms_today),
        (6_500, 310_000),
        "a new session's totals start at 0"
    );
    let saved = rig.store.saved.lock().unwrap().clone();
    assert_eq!(saved.last().unwrap().mics["guest"].connected_ms, 260_000);
}

#[tokio::test(start_paused = true)]
async fn a_session_that_does_not_end_within_five_seconds_is_ended() {
    let capture = Capture::install();
    let stuck = FakeAsr {
        stuck: true,
        ..FakeAsr::default()
    };
    let rig = rig_with(on(240), None, true, stuck, ContextInputs::default());
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    transcription.set_override(choice(SessionMode::Off));
    wait(3).await;
    assert_eq!(rig.guest().0, TranscriptionState::Armed, "still waiting");
    wait(4).await;
    assert_eq!(
        rig.guest(),
        (TranscriptionState::Off, Some(OffReason::OverrideOff))
    );
    let ended = capture.events_with_message("did not end within 5 s");
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].level, tracing::Level::WARN);
}

#[tokio::test(start_paused = true)]
async fn a_new_guest_mic_stops_the_old_session_and_shows_it_off() {
    let rig = rig(on(240));
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    transcription.set_setting(TranscriptionSetting {
        guest_mic: Some("interp".to_string()),
        ..on(240)
    });
    wait(2).await;
    assert!(rig.asr.closed(0), "the old guest's session ended");
    assert_eq!(rig.asr.opens(), 2);
    assert_eq!(rig.asr.opened.lock().unwrap()[1].cfg.mic_id, "interp");
    let sessions = rig.listen.transcripts().snapshot().sessions;
    let old = sessions.iter().find(|s| s.mic_id == "guest").unwrap();
    assert_eq!(
        (old.state, old.reason),
        (TranscriptionState::Off, Some(OffReason::NoGuestMic))
    );
    let status = transcription.status(day());
    assert_eq!(status.mics.len(), 1, "only the current guest is shown");
    assert_eq!(status.mics[0].mic_id, "interp");
}

#[tokio::test(start_paused = true)]
async fn a_provider_stop_is_lifted_by_a_new_guest_mic_or_a_new_day() {
    let rig = rig(on(240));
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    let refuse = |n: usize| {
        let events = rig.asr.events(n);
        async move {
            let refusal = AsrEvent::Finished(Some(OffReason::Payment));
            events.send(refusal).await.unwrap();
        }
    };
    refuse(0).await;
    wait(3).await;
    assert_eq!(rig.guest().1, Some(OffReason::Payment));
    transcription.set_setting(TranscriptionSetting {
        guest_mic: Some("interp".to_string()),
        ..on(240)
    });
    wait(1).await;
    assert_eq!(rig.asr.opens(), 2, "another guest mic runs");
    refuse(1).await;
    wait(3).await;
    assert_eq!(rig.asr.opens(), 2);
    rig.set_layout("preach").await;
    wait(2).await;
    assert_eq!(rig.asr.opens(), 2, "Kázeň does not lift a refusal");
    *rig.now.lock().unwrap() = "2026-11-23T08:00:00".parse().unwrap();
    wait(2).await;
    assert_eq!(rig.asr.opens(), 3, "a new day runs again");
}

#[tokio::test(start_paused = true)]
async fn an_unexpected_end_is_retried_after_a_growing_pause() {
    let capture = Capture::install();
    let rig = rig(on(240));
    rig.listen
        .transcription()
        .set_override(choice(SessionMode::On));
    wait(1).await;
    rig.asr
        .events(0)
        .send(AsrEvent::Finished(None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.asr.opens(), 1, "never reopened on the next tick");
    wait(2).await;
    assert_eq!(rig.asr.opens(), 2, "retried after 1 s");
    rig.asr
        .events(1)
        .send(AsrEvent::Finished(None))
        .await
        .unwrap();
    wait(1).await;
    assert_eq!(rig.asr.opens(), 2, "the second pause is 2 s");
    wait(2).await;
    assert_eq!(rig.asr.opens(), 3);
    let warned = capture.events_with_message("ended unexpectedly");
    let ends: Vec<_> = warned.iter().map(|e| e.field("ends")).collect();
    assert_eq!(ends, [Some("1"), Some("2")]);
}

#[tokio::test(start_paused = true)]
async fn onsets_reconnects_and_lost_frames_reach_the_status_and_the_log() {
    let capture = Capture::install();
    let rig = rig(on(240));
    let transcription = rig.listen.transcription();
    transcription.set_override(choice(SessionMode::On));
    wait(1).await;
    let events = rig.asr.events(0);
    for latency_ms in (1..=10).map(|n| n * 100) {
        events.send(AsrEvent::Onset { latency_ms }).await.unwrap();
    }
    for state in [
        TranscriptionState::Reconnecting,
        TranscriptionState::Reconnecting,
        TranscriptionState::Streaming,
        TranscriptionState::Reconnecting,
    ] {
        let error = Some("timeout".to_string());
        events.send(AsrEvent::State { state, error }).await.unwrap();
    }
    for value in 0..200 {
        rig.send("guest", value);
    }
    wait(1).await;
    let status = transcription.status(day()).mics[0].clone();
    assert_eq!(
        (
            status.latency_last_ms,
            status.latency_p50_ms,
            status.latency_p90_ms
        ),
        (Some(1_000), Some(500), Some(900))
    );
    assert_eq!(status.reconnects, 2, "two outages, three retries");
    let lost = capture.events_with_message("frames lost");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].field("lost"), Some("72"));
}
