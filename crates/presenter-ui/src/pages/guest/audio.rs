//! The guest page's speech output (#999 L3): ONE `AudioContext` made in the
//! Play tap (the gesture iOS needs to unlock audio), asked for 24 kHz so the
//! server's frames play untouched (a phone that refuses gets them resampled),
//! and the `guest-player` AudioWorklet (`/ui-pkg/guest-player.js`) with its
//! short jitter buffer. The worklet reports what it played; the page shows it
//! on `body[data-guest-played]` (the E2E reads it).

use js_sys::{Array, Float32Array, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, AudioContextOptions, AudioWorkletNode, MessageEvent};

use crate::state::guest::{pcm_to_f32, resample};

/// Where Trunk copies the worklet module (`index.html` copy-file).
const WORKLET_URL: &str = "/ui-pkg/guest-player.js";
const PROCESSOR: &str = "guest-player";

/// What the worklet reported last.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerReport {
    pub played: f64,
    pub queued: f64,
    pub dropped: f64,
    pub underruns: f64,
}

pub struct AudioOut {
    context: AudioContext,
    node: AudioWorkletNode,
    rate: f32,
    _on_report: Closure<dyn FnMut(MessageEvent)>,
}

/// Made synchronously inside the tap (iOS unlocks audio only there).
pub fn new_context(source_rate: u32) -> Result<AudioContext, JsValue> {
    let options = AudioContextOptions::new();
    options.set_sample_rate(source_rate as f32);
    AudioContext::new_with_context_options(&options).or_else(|_| AudioContext::new())
}

impl AudioOut {
    /// Resumes the context and loads the worklet; `on_report` gets the
    /// worklet's counters about four times a second.
    pub async fn start(
        context: AudioContext,
        on_report: impl FnMut(PlayerReport) + 'static,
    ) -> Result<Self, JsValue> {
        JsFuture::from(context.resume()?).await?;
        JsFuture::from(context.audio_worklet()?.add_module(WORKLET_URL)?).await?;
        let node = AudioWorkletNode::new(&context, PROCESSOR)?;
        node.connect_with_audio_node(&context.destination())?;
        let mut on_report = on_report;
        let handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let data = event.data();
            let read = |key: &str| {
                Reflect::get(&data, &JsValue::from_str(key))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
            };
            on_report(PlayerReport {
                played: read("played"),
                queued: read("queued"),
                dropped: read("dropped"),
                underruns: read("underruns"),
            });
        });
        node.port()?
            .set_onmessage(Some(handler.as_ref().unchecked_ref()));
        let rate = context.sample_rate();
        Ok(Self {
            context,
            node,
            rate,
            _on_report: handler,
        })
    }

    /// The context's rate (24 000 when the phone accepted it).
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Queue one s16le chunk recorded at `source_rate`.
    pub fn push(&self, pcm: &[u8], source_rate: u32) {
        let samples = resample(&pcm_to_f32(pcm), source_rate as f32, self.rate);
        let array = Float32Array::from(samples.as_slice());
        let transfer = Array::of1(&array.buffer());
        if let Ok(port) = self.node.port() {
            let _ = port.post_message_with_transferable(&array, &transfer);
        }
    }

    /// Forget what is queued (a reconnect starts fresh, never late).
    pub fn reset(&self) {
        if let Ok(port) = self.node.port() {
            let _ = port.post_message(&JsValue::from_str("reset"));
        }
    }

    /// Resume after the phone suspended the context (a call, the lock screen).
    pub fn resume(&self) {
        let _ = self.context.resume();
    }
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        if let Ok(port) = self.node.port() {
            port.set_onmessage(None);
        }
        let _ = self.node.disconnect();
        let _ = self.context.close();
    }
}

/// `navigator.wakeLock.request("screen")` (not in stable web-sys): the screen
/// stays on while the guest listens. `None` where the browser has no wake lock.
pub async fn request_wake_lock() -> Option<JsValue> {
    let navigator = web_sys::window()?.navigator();
    let lock = Reflect::get(&navigator, &JsValue::from_str("wakeLock")).ok()?;
    if lock.is_undefined() || lock.is_null() {
        return None;
    }
    let request: js_sys::Function = Reflect::get(&lock, &JsValue::from_str("request"))
        .ok()?
        .dyn_into()
        .ok()?;
    let promise: js_sys::Promise = request
        .call1(&lock, &JsValue::from_str("screen"))
        .ok()?
        .dyn_into()
        .ok()?;
    JsFuture::from(promise).await.ok()
}

/// Let the screen sleep again.
pub fn release_wake_lock(sentinel: &JsValue) {
    if let Ok(release) = Reflect::get(sentinel, &JsValue::from_str("release")) {
        if let Ok(release) = release.dyn_into::<js_sys::Function>() {
            let _ = release.call0(sentinel);
        }
    }
}
