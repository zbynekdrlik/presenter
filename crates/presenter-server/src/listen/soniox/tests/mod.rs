//! The real connection + provider task against the scripted Soniox server
//! (#852 C3-5): `mock` = the server, `session` = the scenarios. Every test
//! runs on tokio's paused clock, so the 10 s gate, the 5 s keepalives and
//! the 15 s stall are exact without waiting.

mod mock;
mod session;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use presenter_core::listen::context::ContextTerms;
use tokio::sync::mpsc;
use tokio::time::Instant;

use self::mock::{DuplexConnector, MockSoniox, Step};
use super::config::SonioxEnv;
use super::session::SonioxAsr;
use super::wire::DEFAULT_MODEL;
use crate::listen::asr::{AsrEvent, AsrInput, AsrSessionConfig, StreamingAsr};
use crate::listen::MicFrame;

pub(super) const KEY: &str = "test-key-not-real";
/// Speech and room noise as the mic chain reports them.
pub(super) const VOICED: f32 = -20.0;
pub(super) const QUIET: f32 = -80.0;
/// The third sample of every test frame: checks the byte order.
pub(super) const MARKER: i16 = 0x1234;

pub(super) fn env(key: Option<&str>) -> SonioxEnv {
    SonioxEnv {
        url: "ws://127.0.0.1:9/transcribe-websocket".to_string(),
        key: key.map(str::to_string),
        model: DEFAULT_MODEL.to_string(),
        endpoint_delay_ms: None,
    }
}

/// Frame `i`: 1600 samples, the index in the first two, [`MARKER`] third.
pub(super) fn frame(i: u32) -> MicFrame {
    let mut pcm = vec![0_i16; 1600];
    pcm[0] = (i % 30_000) as i16;
    pcm[1] = (i / 30_000) as i16;
    pcm[2] = MARKER;
    Arc::from(pcm)
}

/// The frame index of one BINARY frame the server received.
pub(super) fn index(bytes: &[u8]) -> u32 {
    let sample = |n: usize| i16::from_le_bytes([bytes[2 * n], bytes[2 * n + 1]]) as u32;
    sample(0) + sample(1) * 30_000
}

pub(super) struct Harness {
    pub mock: Arc<MockSoniox>,
    input: mpsc::Sender<AsrInput>,
    events: Arc<Mutex<Vec<(Instant, AsrEvent)>>>,
    next: u32,
}

impl Harness {
    pub fn start(scripts: Vec<Vec<Step>>) -> Self {
        Self::start_with(env(Some(KEY)), scripts)
    }

    pub fn start_with(env: SonioxEnv, scripts: Vec<Vec<Step>>) -> Self {
        let mock = MockSoniox::new(scripts);
        let asr = SonioxAsr::new(env, Arc::new(DuplexConnector(Arc::clone(&mock))));
        let mut session = asr.open(AsrSessionConfig {
            mic_id: "guest".to_string(),
            context: ContextTerms {
                general: vec![("domain".to_string(), "church sermon".to_string())],
                terms: vec!["Romans".to_string()],
            },
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        tokio::spawn(async move {
            while let Some(event) = session.events.recv().await {
                sink.lock().unwrap().push((Instant::now(), event));
            }
        });
        Self {
            mock,
            input: session.input,
            events,
            next: 0,
        }
    }

    /// `n` frames at `dbfs`, one every 100 ms (stops quietly once the
    /// session ended).
    pub async fn speak(&mut self, n: u32, dbfs: f32) {
        for _ in 0..n {
            let pcm = frame(self.next);
            self.next += 1;
            if self
                .input
                .send(AsrInput::Frame { pcm, dbfs })
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn stop(&self) -> Instant {
        let at = Instant::now();
        let _ = self.input.send(AsrInput::Stop).await;
        at
    }

    pub fn events(&self) -> Vec<AsrEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// When the first event matching `want` arrived (waits ≤ 10 min of
    /// paused time).
    pub async fn wait_for(&self, want: impl Fn(&AsrEvent) -> bool) -> Instant {
        for _ in 0..60_000 {
            let found = self
                .events
                .lock()
                .unwrap()
                .iter()
                .find(|(_, e)| want(e))
                .map(|(at, _)| *at);
            if let Some(at) = found {
                return at;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("event not seen; events: {:?}", self.events());
    }

    pub async fn finished(&self) -> Instant {
        self.wait_for(|e| matches!(e, AsrEvent::Finished(_))).await
    }
}
