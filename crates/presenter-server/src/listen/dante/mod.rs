//! The Dante microphones (#852, #933): Presenter's own Dante device, embedded
//! in-process through dstack's `dlink`, with two FIXED mono RX channels
//! `mic 1` / `mic 2` (core `fixed_dante_mics`: ids `dante-mic-1` /
//! `dante-mic-2`). Which source feeds them is Dante patching (dstack), never a
//! Presenter setting: the device subscribes to nothing. Their audio feeds the
//! SAME per-mic chain as a VBAN mic (`mic_chain::BlockQueue` → `run_chain`:
//! 100 ms frames, level, the mic's one frame channel → recorder / ASR).
//!
//! [`supervise`] (started by `ListenService::start` only, so validate mode
//! never starts a device) first publishes the two fixed mics when the box
//! names a Dante interface (`PRESENTER_LISTEN_DANTE_IFACE`, a per-site setting
//! like `DANTE_SUBNET`) — independent of the saved `listen.mics` — and then
//! keeps the device in step with the Dante mics of the published list: no
//! interface → no mics, no device, the status says why; otherwise one device
//! whose plan (`LinkPlan`) changes only with the channels, the interface, the
//! name or the rate. A failed start or a device that ended by itself is
//! retried after 1, 2, 4 … 60 s, WARN on power-of-two attempts (#484). Every [`DRAIN`] the
//! samples of each LIVE channel (its `written` count advanced within
//! [`LIVE_WINDOW`]) go to its mic's chain as one block; a silent or
//! unsubscribed channel's samples are dropped, so the mic reads "waiting" /
//! "no audio", never "receiving" at −∞ dB.

mod inputs;
mod link;
mod tap;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use presenter_core::audio::vban::{VbanAudioFormat, VbanSampleType};
use presenter_core::listen::mics::{DanteInput, MicConfig, MicTransport};
use presenter_core::listen::status::{DanteState, DanteStatus};
use tokio::sync::watch;
use tokio::time::{interval, sleep, Instant, MissedTickBehavior};
use tracing::{info, warn};

use self::inputs::DanteInputs;
pub(crate) use self::link::DlinkBackend;
use self::link::{DanteBackend, LinkPlan, RunningLink};
use self::tap::tap;
use super::{bind_retry_delay, should_log_event, ListenService, MIC_TARGET};

/// The app rate dlink converts every channel to: the chain's, so a Dante mic
/// passes through its converter untouched.
pub(crate) const DANTE_APP_RATE_HZ: u32 = 16_000;
/// The format a Dante mic's blocks report (status `format`).
pub(crate) const DANTE_BLOCK_FORMAT: VbanAudioFormat = VbanAudioFormat {
    rate_hz: DANTE_APP_RATE_HZ,
    channels: 1,
    sample_type: VbanSampleType::Float32,
};
/// The box's Dante interface; unset = no Dante device on this box.
pub(crate) const IFACE_ENV: &str = "PRESENTER_LISTEN_DANTE_IFACE";
/// The device's name in Dante Controller.
pub(crate) const NAME_ENV: &str = "PRESENTER_LISTEN_DANTE_NAME";
/// The network's sample rate.
pub(crate) const RATE_ENV: &str = "PRESENTER_LISTEN_DANTE_RATE";
pub(crate) const DEFAULT_DEVICE_NAME: &str = "presenter-listen";
/// dstack #90: the site's Dante network runs at 96 kHz.
pub(crate) const DEFAULT_NETWORK_RATE: u32 = 96_000;
/// The drain's period: one block per live channel this often.
const DRAIN: Duration = Duration::from_millis(20);
/// A channel whose `written` count advanced this recently is live.
pub(crate) const LIVE_WINDOW: Duration = Duration::from_secs(1);
const NO_INTERFACE: &str =
    "Na tomto počítači nie je nastavené rozhranie Dante (PRESENTER_LISTEN_DANTE_IFACE) — kanály „mic 1“ a „mic 2“ tu nie sú.";

/// The box's Dante settings (env; tests set their own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DanteEnv {
    pub(crate) interface: Option<String>,
    pub(crate) device: String,
    pub(crate) network_rate: u32,
    pub(crate) state_file: PathBuf,
}

impl DanteEnv {
    pub(crate) fn from_env() -> Self {
        Self::parse(
            std::env::var(IFACE_ENV).ok().as_deref(),
            std::env::var(NAME_ENV).ok().as_deref(),
            std::env::var(RATE_ENV).ok().as_deref(),
            &std::env::var("PRESENTER_DB_URL").unwrap_or_default(),
        )
    }

    /// Blank = unset; a rate that is not a number WARNs and takes the default.
    pub(crate) fn parse(
        iface: Option<&str>,
        name: Option<&str>,
        rate: Option<&str>,
        db_url: &str,
    ) -> Self {
        let set = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let network_rate = match set(rate) {
            None => DEFAULT_NETWORK_RATE,
            Some(raw) => raw.parse().unwrap_or_else(|_| {
                warn!(target: MIC_TARGET, value = raw.as_str(), "invalid PRESENTER_LISTEN_DANTE_RATE — using 96000");
                DEFAULT_NETWORK_RATE
            }),
        };
        Self {
            interface: set(iface),
            device: set(name).unwrap_or_else(|| DEFAULT_DEVICE_NAME.to_string()),
            network_rate,
            state_file: crate::state::stream_assets::dir_next_to_db(db_url, "dante-listen.toml"),
        }
    }

    /// The device for `mics` (the Dante ones, in order: the fixed pair);
    /// `None` without one or without an interface.
    fn plan(&self, mics: &[MicConfig]) -> Option<LinkPlan> {
        let channels: Vec<DanteInput> = mics.iter().filter_map(|mic| mic.dante.clone()).collect();
        if channels.is_empty() {
            return None;
        }
        Some(LinkPlan {
            device: self.device.clone(),
            interface: self.interface.clone()?,
            network_rate: self.network_rate,
            state_file: self.state_file.clone(),
            channels,
        })
    }
}

/// The Dante mics of a list, in list order.
fn dante_mics(mics: &[MicConfig]) -> Vec<MicConfig> {
    mics.iter().filter(|mic| mic.is_dante()).cloned().collect()
}

type ConfigRx = watch::Receiver<Arc<Vec<MicConfig>>>;

/// The running device, the plan it runs and when it started.
struct Running {
    plan: LinkPlan,
    link: Box<dyn RunningLink>,
    started: Instant,
}

/// Why the run loop returned.
enum RunEnd {
    /// The mic list changed: the device is handed back (kept when the new
    /// plan is the same, else stopped).
    Changed(Running),
    /// The device ended by itself after running this long: retry.
    Ended(String, Duration),
    /// The service is gone.
    Closed,
}

/// A device that ran this long before it ended starts its retry pause at 1 s
/// again (the VBAN socket's rule).
const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// The Dante task: see the module.
pub(crate) async fn supervise(
    service: ListenService,
    backend: Arc<dyn DanteBackend>,
    env: DanteEnv,
) {
    service.set_dante_mics(env.interface.is_some());
    let mut config = service.config_receiver();
    let mut inputs = DanteInputs::default();
    let mut running: Option<Running> = None;
    let mut failures: u32 = 0;
    loop {
        let mics = dante_mics(&config.borrow_and_update());
        inputs.update(&service, &mics);
        let plan = env.plan(&mics);
        if running.as_ref().map(|r| &r.plan) != plan.as_ref() {
            stop(running.take()).await;
        }
        let Some(plan) = plan else {
            failures = 0;
            set_idle(&service, &env);
            if config.changed().await.is_err() {
                return;
            }
            continue;
        };
        let current = match running.take() {
            Some(current) => current,
            None => match start(&backend, &plan, &mut inputs).await {
                Ok(current) => {
                    set_dante(&service, &plan, DanteState::Running, None);
                    inputs.set_error(&service, None);
                    current
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let error = format!("zariadenie Dante sa nedá spustiť: {error}");
                    if !retry_later(&service, &plan, &mut inputs, &mut config, failures, error)
                        .await
                    {
                        return;
                    }
                    continue;
                }
            },
        };
        match run(&service, &mut inputs, current, &mut config).await {
            RunEnd::Changed(kept) => running = Some(kept),
            RunEnd::Closed => return,
            RunEnd::Ended(error, ran) => {
                if ran >= HEALTHY_AFTER {
                    failures = 0;
                }
                failures = failures.saturating_add(1);
                if !retry_later(&service, &plan, &mut inputs, &mut config, failures, error).await {
                    return;
                }
            }
        }
    }
}

/// Stop a device from a blocking thread (dlink sends its goodbyes).
async fn stop(running: Option<Running>) {
    if let Some(running) = running {
        let device = running.plan.device.clone();
        let _stopped = tokio::task::spawn_blocking(move || running.link.stop()).await;
        info!(target: MIC_TARGET, device = device.as_str(), "Dante device closed");
    }
}

/// Start the device of `plan` with a fresh tap (from a blocking thread: dlink
/// starts outside any runtime, on its own thread).
async fn start(
    backend: &Arc<dyn DanteBackend>,
    plan: &LinkPlan,
    inputs: &mut DanteInputs,
) -> Result<Running, String> {
    let (tap, outputs) = tap(plan.channels.len());
    let backend = Arc::clone(backend);
    let starting = plan.clone();
    let link = tokio::task::spawn_blocking(move || backend.start(&starting, tap))
        .await
        .map_err(|err| err.to_string())??;
    info!(
        target: MIC_TARGET,
        device = plan.device.as_str(),
        interface = plan.interface.as_str(),
        network_rate = plan.network_rate,
        channels = plan.channels.len(),
        "Dante device started"
    );
    inputs.attach(outputs);
    Ok(Running {
        plan: plan.clone(),
        link,
        started: Instant::now(),
    })
}

/// Drain the live channels every [`DRAIN`] and count once a second, until the
/// mic list changes or the device ends.
async fn run(
    service: &ListenService,
    inputs: &mut DanteInputs,
    current: Running,
    config: &mut ConfigRx,
) -> RunEnd {
    let mut drain = interval(DRAIN);
    drain.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut second = interval(Duration::from_secs(1));
    second.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = config.changed() => {
                if changed.is_err() {
                    stop(Some(current)).await;
                    return RunEnd::Closed;
                }
                return RunEnd::Changed(current);
            }
            _ = drain.tick() => inputs.drain(service, &current.link.flows(), Instant::now()),
            _ = second.tick() => {
                let now = Instant::now();
                service.with_health(|health| health.on_transport_tick(MicTransport::Dante, now));
                inputs.on_tick(now);
                if let Some(why) = current.link.ended() {
                    let ran = current.started.elapsed();
                    stop(Some(current)).await;
                    return RunEnd::Ended(format!("zariadenie Dante skončilo: {why}"), ran);
                }
            }
        }
    }
}

/// A failed start or an ended device: status + WARN (power-of-two attempts),
/// then wait the back-off or a mic list change. `false` = the service is gone.
async fn retry_later(
    service: &ListenService,
    plan: &LinkPlan,
    inputs: &mut DanteInputs,
    config: &mut ConfigRx,
    failures: u32,
    error: String,
) -> bool {
    let delay = bind_retry_delay(failures);
    if should_log_event(u64::from(failures)) {
        warn!(
            target: MIC_TARGET,
            device = plan.device.as_str(),
            interface = plan.interface.as_str(),
            error = error.as_str(),
            attempt = failures,
            retry_in_s = delay.as_secs(),
            "the Dante device is not running — retrying"
        );
    }
    set_dante(service, plan, DanteState::Retrying, Some(error.clone()));
    inputs.set_error(service, Some(&error));
    tokio::select! {
        () = sleep(delay) => true,
        changed = config.changed() => changed.is_ok(),
    }
}

fn set_dante(service: &ListenService, plan: &LinkPlan, state: DanteState, error: Option<String>) {
    service.with_health(|health| {
        health.dante = DanteStatus {
            state,
            device: Some(plan.device.clone()),
            interface: Some(plan.interface.clone()),
            error,
        };
    });
}

/// No device: no interface on this box (no Dante mics then), or no Dante
/// channel published yet.
fn set_idle(service: &ListenService, env: &DanteEnv) {
    let (state, error) = match env.interface {
        None => (DanteState::NoInterface, Some(NO_INTERFACE.to_string())),
        Some(_) => (DanteState::Idle, None),
    };
    service.with_health(|health| {
        health.dante = DanteStatus {
            state,
            device: Some(env.device.clone()),
            interface: env.interface.clone(),
            error,
        };
    });
}
