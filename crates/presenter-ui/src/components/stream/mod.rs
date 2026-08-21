//! Stream-graphics output-page components (#709, epic #718; lyrics + verse #710).
//!
//! `SceneRender` maps a scene's elements (already ordered by `z_order` in the
//! def) to per-kind components: IMAGE + COUNTDOWN (#709) and LYRICS + VERSE
//! (#710). Lyrics + verse bind to the EXISTING worship-stage and Bible live
//! events (no new content pipeline); an element with no active content renders
//! nothing, so the transparent output stays clean for a scene that mixes kinds.

pub mod element_countdown;
pub mod element_image;
pub mod element_lyrics;
pub mod element_verse;
pub mod scene_render;
pub mod style;
pub mod transition;

use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, StageDisplaySnapshot};

use crate::ws::stream::TimersReceipt;

/// Shared reactive context for the stream output page, provided by
/// `pages::stream_output` and consumed by the text elements.
///
/// - `timers` is the latest `Timers` snapshot stamped with its receipt time;
///   `now_ms` is bumped on a 250 ms interval so the countdown re-derives its
///   remaining time smoothly between server pushes.
/// - `stage` is the latest worship `Stage` snapshot (its `current` slide drives
///   the lyrics element; `None` current ⇒ no lyrics).
/// - `bible` is the current Bible slide output (`Some` from `BibleSlide`, `None`
///   from `BibleCleared`) driving the verse element.
///
/// All four are seeded by a REST cold-load on WS connect and kept fresh by the
/// `ws/stream.rs` hook — a reconnecting OBS source recovers the live look.
#[derive(Clone, Copy)]
pub struct StreamContext {
    pub timers: RwSignal<Option<TimersReceipt>>,
    pub now_ms: RwSignal<f64>,
    pub stage: RwSignal<Option<StageDisplaySnapshot>>,
    pub bible: RwSignal<Option<BibleSlideOutput>>,
}
