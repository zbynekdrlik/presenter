//! Stream-graphics output-page components (#709, epic #718).
//!
//! `SceneRender` maps a scene's elements (already ordered by `z_order` in the
//! def) to per-kind components. This lane ships the IMAGE and COUNTDOWN kinds;
//! LYRICS and VERSE arrive in #710 and are skipped here (rendered as nothing, so
//! the transparent output stays clean for a scene that mixes kinds).

pub mod element_countdown;
pub mod element_image;
pub mod scene_render;

use leptos::prelude::*;

use crate::ws::stream::TimersReceipt;

/// Shared reactive context for the stream output page, provided by
/// `pages::stream_output` and consumed by the countdown element.
///
/// `timers` is the latest `Timers` snapshot stamped with its receipt time;
/// `now_ms` is bumped on a 250 ms interval so the countdown re-derives its
/// remaining time smoothly between server pushes.
#[derive(Clone, Copy)]
pub struct StreamContext {
    pub timers: RwSignal<Option<TimersReceipt>>,
    pub now_ms: RwSignal<f64>,
}
