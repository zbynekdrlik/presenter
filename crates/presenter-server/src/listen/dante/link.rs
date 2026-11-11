//! The external boundary of the Dante mics (#852): starting and stopping
//! Presenter's own Dante device. [`DanteBackend`] is the seam — the real one
//! ([`DlinkBackend`]) embeds dstack's `dlink`; tests use a fake that drives the
//! real `Tap`, drain, chain and health.
//!
//! `dlink::Link::start` must run OUTSIDE any tokio runtime (it runs its own,
//! dlink's docs), and a `spawn_blocking` thread has one entered. So the link
//! lives on a std thread of its own: it starts the device, answers the start,
//! then every [`FLOW_POLL`] reads the RX channels' counts into a shared
//! [`Flows`] (what tells a live flow from a silent channel) and whether the
//! device ended by itself, until it is told to stop; then `Link::stop` sends
//! the goodbyes (up to dlink's `STOP_WAIT`).

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use dlink::{Config, Link, Map, Place, RxChannel, SubState};
use presenter_core::listen::mics::DanteInput;

use super::tap::Tap;
use super::DANTE_APP_RATE_HZ;

/// How long a start may take before it counts as failed: a `Link::start`
/// that never returns must not hold the supervisor (the late device is then
/// stopped by its own thread: its stop channel is already gone).
const START_WAIT: Duration = Duration::from_secs(30);
/// How often the link thread reads the RX channels' counts.
const FLOW_POLL: Duration = Duration::from_millis(100);

/// What the device runs: everything that, changed, needs a new device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkPlan {
    /// The device's name in Dante Controller.
    pub(crate) device: String,
    pub(crate) interface: String,
    /// The network's sample rate (the device's; the app's is 16 kHz).
    pub(crate) network_rate: u32,
    /// Where the device keeps what Dante Controller sets.
    pub(crate) state_file: PathBuf,
    /// One RX channel per Dante mic, in mic order (its name and source).
    pub(crate) channels: Vec<DanteInput>,
}

/// One RX channel as the device reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChannelFlow {
    /// Samples written for the channel so far: a live flow advances it.
    pub(crate) written: u64,
    /// What it is subscribed to (`channel@device`) and Dante's status word.
    pub(crate) source: Option<String>,
}

/// The RX channels' latest counts, shared by the link thread and the drain.
pub(crate) type Flows = Arc<Mutex<Vec<ChannelFlow>>>;

/// A running device.
pub(crate) trait RunningLink: Send {
    /// Each RX channel's latest counts, in plan order (never waits on the
    /// device: a copy of the last poll).
    fn flows(&self) -> Vec<ChannelFlow>;
    /// Why the device ended by itself, once it did.
    fn ended(&self) -> Option<String>;
    /// Stop the device; blocks until its goodbyes are sent (call it from a
    /// blocking thread).
    fn stop(self: Box<Self>);
}

/// Starts devices. `start` blocks (call it from a blocking thread).
pub(crate) trait DanteBackend: Send + Sync {
    fn start(&self, plan: &LinkPlan, tap: Tap) -> Result<Box<dyn RunningLink>, String>;
}

/// dlink's config of `plan`: RX only, the app at 16 kHz, the device at the
/// network's rate, each source as the channel's default subscription (one
/// Dante Controller sets is kept in the state file and wins).
pub(crate) fn dlink_config(plan: &LinkPlan) -> Config {
    let names: Vec<&str> = plan
        .channels
        .iter()
        .map(|input| input.channel.as_str())
        .collect();
    let mut config = Config::new(plan.device.clone(), plan.state_file.clone())
        .rx(names)
        .sample_rate(DANTE_APP_RATE_HZ)
        .network_rate(plan.network_rate)
        .product("Presenter", "presenter-listen")
        .flow_slots(plan.channels.len().max(1));
    for (index, input) in plan.channels.iter().enumerate() {
        if let Some((channel, device)) = input.source_parts() {
            config = config.subscribe(index + 1, channel, device);
        }
    }
    config
}

/// A channel's flow as the device reports it.
fn channel_flow(channel: &RxChannel) -> ChannelFlow {
    let source = channel.subscription.as_ref().map(|sub| {
        let status = channel.status.map_or("—", |status| match status.state() {
            SubState::Connected => "prijíma",
            SubState::Pending => "pripája sa",
            SubState::Idle => "nečinný",
            SubState::Error => status.label(),
        });
        format!("{}@{} ({status})", sub.tx_channel, sub.tx_device)
    });
    ChannelFlow {
        written: channel.written,
        source,
    }
}

/// The real device: dstack's `dlink`.
pub(crate) struct DlinkBackend;

struct DlinkRunning {
    flows: Flows,
    ended: Arc<Mutex<Option<String>>>,
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl DanteBackend for DlinkBackend {
    fn start(&self, plan: &LinkPlan, tap: Tap) -> Result<Box<dyn RunningLink>, String> {
        let config = dlink_config(plan);
        let place = Place::Iface(plan.interface.clone());
        let channels = plan.channels.len();
        let flows: Flows = Arc::new(Mutex::new(vec![ChannelFlow::default(); channels]));
        let ended = Arc::new(Mutex::new(None));
        let (started_tx, started_rx) = mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = {
            let (flows, ended) = (Arc::clone(&flows), Arc::clone(&ended));
            std::thread::Builder::new()
                .name("dante-listen".into())
                .spawn(move || {
                    let map = Map {
                        rx: (0..channels).collect(),
                        tx: Vec::new(),
                    };
                    match Link::start(&config, &place, map, tap) {
                        Ok(link) => {
                            let _sent = started_tx.send(Ok(()));
                            run_link(link, &flows, &ended, &stop_rx);
                        }
                        Err(err) => {
                            let _sent = started_tx.send(Err(err.to_string()));
                        }
                    }
                })
                .map_err(|err| format!("vlákno zariadenia Dante sa nedá spustiť: {err}"))?
        };
        match started_rx.recv_timeout(START_WAIT) {
            Ok(Ok(())) => Ok(Box::new(DlinkRunning {
                flows,
                ended,
                stop: stop_tx,
                thread: Some(thread),
            })),
            Ok(Err(error)) => {
                let _joined = thread.join();
                Err(error)
            }
            Err(RecvTimeoutError::Timeout) => {
                // `stop_tx` drops here: a device that starts late stops at once.
                Err(format!(
                    "zariadenie Dante sa nespustilo do {} s",
                    START_WAIT.as_secs()
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _joined = thread.join();
                Err("vlákno zariadenia Dante skončilo pri štarte".to_string())
            }
        }
    }
}

/// The link thread after a start: poll the flows until told to stop (or the
/// device ended by itself), then stop the device.
fn run_link(link: Link, flows: &Flows, ended: &Mutex<Option<String>>, stop: &mpsc::Receiver<()>) {
    loop {
        let now: Vec<ChannelFlow> = link.rx_channels().iter().map(channel_flow).collect();
        *flows.lock().unwrap_or_else(PoisonError::into_inner) = now;
        if let Some(why) = link.ended() {
            *ended.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
            break;
        }
        match stop.recv_timeout(FLOW_POLL) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let stopped = link.stop();
    tracing::info!(target: super::super::MIC_TARGET, stopped = ?stopped, "Dante device stopped");
}

impl RunningLink for DlinkRunning {
    fn flows(&self) -> Vec<ChannelFlow> {
        self.flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn ended(&self) -> Option<String> {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn stop(mut self: Box<Self>) {
        let _sent = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}
