//! Content-crossfade primitive for the stream output page (#716, epic #718).
//!
//! `CrossfadeText` animates a single text line whose CONTENT changes over time
//! (a lyric line, a verse line). It renders the current text plus —
//! transiently, during a `Fade` — the OUTGOING text, each as a stacked
//! `.stream-crossfade__layer`, so the old fades out while the new fades in. A
//! `Cut` swaps atomically (never two layers, no overlap frame). A `FadeThrough`
//! (#834) fades the old text out FIRST and mounts the new one only after it is
//! gone (the pure state lives in `content_layers.rs`). It is the shared
//! implementation reused by the lyrics / verse elements so the transition
//! logic lives in ONE place.
//!
//! Change detection rides the `Memo<String>` the caller passes: a Memo only
//! notifies on a genuine value change, so a re-derived but unchanged line
//! never animates. (The countdown does not use this component: its per-tick
//! digits swap in place, #776.)
//!
//! Empty text renders NOTHING (the wrapper is unmounted when no layer remains),
//! so a cleared line is DOM-absent — the transparent output stays clean and the
//! `[data-role]` count-0 contract the #710 lyrics/verse specs assert is preserved.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::ContentTransition;

use super::content_layers::{fade_through_change, fade_through_settle, ContentLayer};

/// A leaving layer is removed this many ms AFTER its fade duration, so the
/// opacity transition has fully completed before the node leaves the DOM.
const FADE_REMOVE_BUFFER_MS: u32 = 80;

#[component]
pub fn CrossfadeText(
    /// The reactive current text (empty ⇒ no visible content). A `Memo` so the
    /// crossfade fires only on a genuine value change.
    text: Memo<String>,
    /// How a content change animates: `Cut` = instant, `Fade` = crossfade,
    /// `FadeThrough` = fade out, then fade the new text in (#834).
    transition: ContentTransition,
    /// `data-role` for the wrapper (e.g. `"stream-lyrics-main"`) — the element the
    /// E2E targets for text/geometry/count.
    #[prop(into)]
    role: String,
    /// Extra CSS classes for the wrapper (e.g. `"stream-lyrics__line ..."`); the
    /// text style + wrapping is carried here and INHERITED by the layers.
    #[prop(into, optional)]
    wrapper_class: String,
    /// Inline CSS for the wrapper (text style + width). Empty for the countdown,
    /// which keeps its style on the outer element and the wrapper inherits it.
    #[prop(into, optional)]
    wrapper_style: String,
    /// Fill the wrapper width (`minmax(0,1fr)`) so text wraps within the Frame —
    /// lyrics/verse lines. The countdown leaves this false (content-sized +
    /// centered by the element's flex box).
    #[prop(optional)]
    fill: bool,
) -> impl IntoView {
    // `is_fade`: layers animate in/out. `through`: a change waits for the
    // fade-out before the new text is mounted (#834) instead of crossfading.
    let (is_fade, through, fade_ms) = match transition {
        ContentTransition::Cut => (false, false, 0u32),
        ContentTransition::Fade { duration_ms } => (true, false, duration_ms),
        ContentTransition::FadeThrough { duration_ms } => (true, true, duration_ms),
    };

    let layers = RwSignal::new(Vec::<ContentLayer>::new());
    // #834: the text waiting for the current fade-out (`FadeThrough` only).
    let pending = StoredValue::new(None::<String>);
    let next_seq = StoredValue::new(0u64);
    let bump = move || {
        let s = next_seq.get_value();
        next_seq.set_value(s + 1);
        s
    };

    // Drive the layer list off the text Memo. First run seeds the current text;
    // later runs (only fire on a genuine change) fade the old out / cut it, and
    // add the new (non-empty) text.
    Effect::new(move |prev: Option<String>| {
        let cur = text.get();
        match prev {
            None => {
                if !cur.is_empty() {
                    let seq = bump();
                    layers.update(|ls| {
                        ls.push(ContentLayer {
                            seq,
                            text: cur.clone(),
                            fade: is_fade,
                            leaving: false,
                        });
                    });
                }
            }
            Some(p) if p == cur => {}
            Some(_) if through => fade_through(layers, pending, &cur, fade_ms, bump),
            Some(_) if is_fade => {
                let mut removing = Vec::new();
                layers.update(|ls| {
                    for l in ls.iter_mut().filter(|l| !l.leaving) {
                        l.leaving = true;
                        removing.push(l.seq);
                    }
                    if !cur.is_empty() {
                        let seq = bump();
                        ls.push(ContentLayer {
                            seq,
                            text: cur.clone(),
                            fade: true,
                            leaving: false,
                        });
                    }
                });
                for seq in removing {
                    Timeout::new(fade_ms + FADE_REMOVE_BUFFER_MS, move || {
                        let _ = layers.try_update(|ls| ls.retain(|x| x.seq != seq));
                    })
                    .forget();
                }
            }
            Some(_) => {
                // Cut: update the existing (single) layer's text IN PLACE, keeping
                // its `seq` — the keyed `<For>` then reconciles the SAME DOM node
                // instead of destroying + recreating it each change (#776). Empty
                // ⇒ drop the layer (wrapper unmounts, count-0-on-clear preserved).
                // The layer's text is read REACTIVELY by seq below, so an in-place
                // text change on an unchanged key still reaches the DOM
                // (#496/#693/#716 keyed-`<For>` reactive-field trap).
                if cur.is_empty() {
                    layers.set(Vec::new());
                } else {
                    layers.update(|ls| match ls.iter_mut().find(|l| !l.leaving) {
                        Some(l) => l.text = cur.clone(),
                        None => ls.push(ContentLayer {
                            seq: bump(),
                            text: cur.clone(),
                            fade: false,
                            leaving: false,
                        }),
                    });
                }
            }
        }
        cur
    });

    let mut wrapper_classes = String::from("stream-crossfade");
    if fill {
        wrapper_classes.push_str(" stream-crossfade--fill");
    }
    if !wrapper_class.is_empty() {
        wrapper_classes.push(' ');
        wrapper_classes.push_str(&wrapper_class);
    }

    let has_layers = move || layers.with(|ls| !ls.is_empty());
    let layers_for_each = move || layers.get();

    view! {
        <Show when=has_layers>
            <div
                class=wrapper_classes.clone()
                data-role=role.clone()
                style=wrapper_style.clone()
            >
                <For
                    each=layers_for_each
                    key=|l| l.seq
                    children=move |l: ContentLayer| {
                        let seq = l.seq;
                        let fade = l.fade;
                        // #496/#693: read `leaving` REACTIVELY by seq — a keyed
                        // `<For>` does not re-run children when only the leaving
                        // flag flips, so a captured bool would never apply the
                        // fade-out class.
                        let class = move || {
                            let mut c = String::from("stream-crossfade__layer");
                            if fade {
                                c.push_str(" stream-crossfade__layer--fade");
                            }
                            let leaving = layers
                                .with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.leaving))
                                .unwrap_or(true);
                            if leaving {
                                c.push_str(" stream-crossfade__layer--leaving");
                            }
                            c
                        };
                        // #776: read `text` REACTIVELY by seq too — the `Cut` path
                        // now updates a layer's text in place (same seq), and a
                        // keyed `<For>` does not re-run children for an unchanged
                        // key, so a captured `l.text` would go stale. `Fade`
                        // (new seq each change) is unaffected.
                        let text = move || {
                            layers
                                .with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.text.clone()))
                                .unwrap_or_default()
                        };
                        let style = if fade {
                            format!("transition-duration:{fade_ms}ms;")
                        } else {
                            String::new()
                        };
                        view! {
                            <div class=class data-role="stream-crossfade-layer" style=style>
                                {text}
                            </div>
                        }
                    }
                />
            </div>
        </Show>
    }
}

/// A `FadeThrough` text change (#834): the visible text starts fading out and
/// `cur` waits as the pending text; when that fade-out ends, ONE dispose-safe
/// `try_update` drops the old layer and mounts the newest pending text, which
/// then fades in through `@starting-style`. A change while a fade-out is
/// already running only replaces the pending text (no second timer).
fn fade_through(
    layers: RwSignal<Vec<ContentLayer>>,
    pending: StoredValue<Option<String>>,
    cur: &str,
    fade_ms: u32,
    bump: impl Fn() -> u64 + Copy + 'static,
) {
    let mut waiting = pending.get_value();
    let mut started = Vec::new();
    layers.update(|ls| started = fade_through_change(ls, &mut waiting, cur, bump));
    pending.set_value(waiting);
    if started.is_empty() {
        return;
    }
    Timeout::new(fade_ms + FADE_REMOVE_BUFFER_MS, move || {
        // The element may have been re-rendered or the page closed meanwhile:
        // every access is a `try_`, so a late timer is a no-op.
        let Some(mut waiting) = pending.try_get_value() else {
            return;
        };
        let _ = layers.try_update(|ls| fade_through_settle(ls, &mut waiting, &started, bump));
        let _ = pending.try_set_value(waiting);
    })
    .forget();
}
