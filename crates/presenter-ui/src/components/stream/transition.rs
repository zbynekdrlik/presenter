//! Content-crossfade primitive for the stream output page (#716, epic #718).
//!
//! `CrossfadeText` animates a single text line whose CONTENT changes over time
//! (a lyric line, a verse line, the countdown text). It renders the current text
//! plus — transiently, during a `Fade` — the OUTGOING text, each as a stacked
//! `.stream-crossfade__layer`, so the old fades out while the new fades in. A
//! `Cut` swaps atomically (never two layers, no overlap frame). It is the shared
//! implementation reused by the countdown / lyrics / verse elements so the
//! transition logic lives in ONE place.
//!
//! Change detection rides the `Memo<String>` the caller passes: a Memo only
//! notifies on a genuine value change, so e.g. the countdown (whose text is
//! re-derived every 250 ms off the page tick) crossfades only on the per-second
//! value change, never every tick.
//!
//! Empty text renders NOTHING (the wrapper is unmounted when no layer remains),
//! so a cleared line is DOM-absent — the transparent output stays clean and the
//! `[data-role]` count-0 contract the #710 lyrics/verse specs assert is preserved.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::ContentTransition;

/// A leaving layer is removed this many ms AFTER its fade duration, so the
/// opacity transition has fully completed before the node leaves the DOM.
const FADE_REMOVE_BUFFER_MS: u32 = 80;

/// One rendered copy of the content: the current text, or an outgoing text still
/// fading out. Keyed on `seq` (a monotonic instance id — NOT the text) so the
/// same text re-appearing while a previous copy is still leaving never collides.
#[derive(Clone)]
struct ContentLayer {
    seq: u64,
    text: String,
    /// `true` for a `Fade` layer (animates in/out); `false` for a `Cut` layer
    /// (instant). Immutable per layer.
    fade: bool,
    /// Set when this layer is fading out and scheduled for removal.
    leaving: bool,
}

#[component]
pub fn CrossfadeText(
    /// The reactive current text (empty ⇒ no visible content). A `Memo` so the
    /// crossfade fires only on a genuine value change.
    text: Memo<String>,
    /// How a content change animates: `Cut` = instant, `Fade` = crossfade.
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
    let (is_fade, fade_ms) = match transition {
        ContentTransition::Cut => (false, 0u32),
        ContentTransition::Fade { duration_ms } => (true, duration_ms),
    };

    let layers = RwSignal::new(Vec::<ContentLayer>::new());
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
                // Cut: replace with a single instant layer (or none if empty).
                let next = if cur.is_empty() {
                    Vec::new()
                } else {
                    vec![ContentLayer {
                        seq: bump(),
                        text: cur.clone(),
                        fade: false,
                        leaving: false,
                    }]
                };
                layers.set(next);
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
                        let style = if fade {
                            format!("transition-duration:{fade_ms}ms;")
                        } else {
                            String::new()
                        };
                        view! {
                            <div class=class data-role="stream-crossfade-layer" style=style>
                                {l.text}
                            </div>
                        }
                    }
                />
            </div>
        </Show>
    }
}
