//! Lower-third "menovka" element for the stream output page (#779).
//!
//! Unlike the other elements, a lower third's TEXT is not stored on the element
//! — it comes from the runtime `active_nameplate` show-state on
//! `StreamContext.nameplate`. This element carries only the LOOK: the bar +
//! accent colours, the two text styles, the animation preset, and the in/out
//! durations. It renders NOTHING while idle (no active plate), and animates a
//! plate in on show / out on hide / crossfades A→B on a swap.
//!
//! Animation follows the file's convention — transitions + `@starting-style`, no
//! `@keyframes`; only `transform`/`opacity`/`clip-path` animate (compositor-only,
//! 60 fps in OBS CEF). Layers are keyed on the plate's monotonic `seq` (from the
//! server) with a JS removal buffer, exactly like the scene crossfade
//! (`pages/stream_output.rs`) + `CrossfadeText` — so a re-show of the same plate
//! while the previous copy is still leaving never key-collides.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::{AnimationPreset, Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::StreamContext;

/// A leaving plate is removed this many ms AFTER its out duration completes.
const LEAVE_REMOVE_BUFFER_MS: u32 = 80;

/// One rendered plate copy: the on-air plate, or an outgoing one still leaving.
#[derive(Clone)]
struct PlateLayer {
    seq: u64,
    primary: String,
    secondary: String,
    leaving: bool,
    /// Enter uses `in_ms`; flipped to `out_ms` when the layer starts leaving.
    duration_ms: u32,
}

/// The CSS `data-animation` token for a preset (matches `stream_output.css`).
fn animation_token(animation: AnimationPreset) -> &'static str {
    match animation {
        AnimationPreset::SlideLeft => "slide_left",
        AnimationPreset::SlideUp => "slide_up",
        AnimationPreset::Wipe => "wipe",
    }
}

/// Combine a `#rrggbb`/`#rrggbbaa` bar colour with the separate `bar_opacity`
/// (0..=1) into a CSS `rgba(...)`. The plate element's own `opacity` drives the
/// fade animation, so the bar's transparency lives in the background colour
/// instead (never fighting the animation). A malformed colour degrades to a
/// translucent black.
fn bar_background(hex: &str, opacity: f32) -> String {
    let (r, g, b) = super::style::hex_rgb(hex).unwrap_or((0, 0, 0));
    let a = opacity.clamp(0.0, 1.0);
    format!("rgba({r},{g},{b},{a})")
}

/// Mark a layer leaving (using `out_ms` for its exit transition) + schedule its
/// removal once the exit completes. `try_update` is dispose-safe.
fn mark_leaving(layers: RwSignal<Vec<PlateLayer>>, seq: u64, out_ms: u32) {
    layers.update(|ls| {
        if let Some(l) = ls.iter_mut().find(|l| l.seq == seq) {
            l.leaving = true;
            l.duration_ms = out_ms;
        }
    });
    Timeout::new(out_ms + LEAVE_REMOVE_BUFFER_MS, move || {
        let _ = layers.try_update(|ls| ls.retain(|l| l.seq != seq));
    })
    .forget();
}

#[component]
#[allow(clippy::too_many_arguments)]
pub fn ElementLowerThird(
    id: i64,
    frame: Frame,
    bar_color: String,
    bar_opacity: f32,
    accent_color: String,
    accent_width_pct: f32,
    primary_style: TextStyle,
    secondary_style: TextStyle,
    padding_pct: f32,
    animation: AnimationPreset,
    in_ms: u32,
    out_ms: u32,
    z: i32,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");

    let container = frame_css(&frame, z);
    let anim = animation_token(animation);
    let bar_bg = bar_background(&bar_color, bar_opacity);
    let accent_css = format!("width:{accent_width_pct}%;background:{accent_color};");
    let text_css = format!("padding:{padding_pct}%;");
    let primary_css = text_style_css(&primary_style);
    let secondary_css = text_style_css(&secondary_style);

    let layers = RwSignal::new(Vec::<PlateLayer>::new());

    // Drive the layer list off the active-plate signal. A new plate (different
    // `seq`) fades the old out + the new in; a hide fades the current out.
    Effect::new(move |prev: Option<Option<u64>>| {
        let cur = ctx.nameplate.get();
        let cur_seq = cur.as_ref().map(|a| a.seq);
        let prev_seq = prev.flatten();
        if cur_seq == prev_seq {
            return cur_seq;
        }
        // Fade out every currently-visible plate.
        let leaving_seqs: Vec<u64> =
            layers.with_untracked(|ls| ls.iter().filter(|l| !l.leaving).map(|l| l.seq).collect());
        for seq in leaving_seqs {
            mark_leaving(layers, seq, out_ms);
        }
        // Fade in the new plate (if any).
        if let Some(active) = cur {
            layers.update(|ls| {
                ls.push(PlateLayer {
                    seq: active.seq,
                    primary: active.primary,
                    secondary: active.secondary,
                    leaving: false,
                    duration_ms: in_ms,
                });
            });
        }
        cur_seq
    });

    let layers_each = move || layers.get();

    view! {
        <div
            class="stream-element stream-element--lower-third"
            data-role="stream-element-lower-third"
            data-element-id=id.to_string()
            data-animation=anim
            style=container
        >
            <For
                each=layers_each
                key=|l| l.seq
                children=move |l: PlateLayer| {
                    let seq = l.seq;
                    let primary = l.primary.clone();
                    let secondary = l.secondary.clone();
                    // #496/#693: read `leaving` + `duration_ms` REACTIVELY by seq —
                    // a keyed `<For>` does not re-run children when a field flips.
                    let leaving = Signal::derive(move || {
                        layers.with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.leaving).unwrap_or(true))
                    });
                    let plate_style = {
                        let bar_bg = bar_bg.clone();
                        Signal::derive(move || {
                            let dur = layers
                                .with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.duration_ms))
                                .unwrap_or(0);
                            format!("background:{bar_bg};transition-duration:{dur}ms;")
                        })
                    };
                    let has_secondary = !secondary.is_empty();
                    let primary_css = primary_css.clone();
                    let secondary_css = secondary_css.clone();
                    let accent_css = accent_css.clone();
                    let text_css = text_css.clone();
                    view! {
                        <div
                            class="stream-lower-third__plate"
                            class:stream-lower-third__plate--leaving=leaving
                            data-role="stream-lower-third-plate"
                            style=plate_style
                        >
                            <span class="stream-lower-third__accent" style=accent_css></span>
                            <div class="stream-lower-third__text" style=text_css>
                                <div class="stream-lower-third__primary" data-role="stream-lower-third-primary" style=primary_css>
                                    {primary}
                                </div>
                                <Show when=move || has_secondary>
                                    <div class="stream-lower-third__secondary" data-role="stream-lower-third-secondary" style=secondary_css.clone()>
                                        {secondary.clone()}
                                    </div>
                                </Show>
                            </div>
                        </div>
                    }
                }
            />
        </div>
    }
}
