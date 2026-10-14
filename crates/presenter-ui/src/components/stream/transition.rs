//! Content-crossfade primitive for the stream output page (#716, epic #718;
//! generalised to a block in #863).
//!
//! `Crossfade` animates content whose VALUE changes over time. The content is a
//! `Memo<String>` KEY: since #863 the encoded lines of a whole text block (the
//! verse, the lyrics: `text_block.rs`). It renders the current content plus — transiently, during a
//! `Fade` — the OUTGOING content, each as a stacked `.stream-crossfade__layer`,
//! so the old fades out while the new fades in. A `Cut` swaps atomically (never
//! two layers, no overlap frame). A `FadeThrough` (#834) fades the old content
//! out FIRST and mounts the new one only after it is gone (the pure state lives
//! in `content_layers.rs`). A caller-supplied [`LayerRender`] draws ONE layer
//! from its content key, so the layer machinery lives in ONE place.
//!
//! Change detection rides the `Memo<String>` the caller passes: a Memo only
//! notifies on a genuine value change, so re-derived but unchanged content
//! never animates. (The countdown does not use this component: its per-tick
//! digits swap in place, #776.)
//!
//! An empty key renders NOTHING (the wrapper is unmounted when no layer
//! remains), so cleared content is DOM-absent — the transparent output stays
//! clean and the `[data-role]` count-0 contract the #710 lyrics/verse specs
//! assert is preserved.

use std::sync::Arc;

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::ContentTransition;

use super::content_layers::{fade_through_change, fade_through_settle, ContentLayer};

/// A leaving layer is removed this many ms AFTER its fade duration, so the
/// opacity transition has fully completed before the node leaves the DOM.
const FADE_REMOVE_BUFFER_MS: u32 = 80;

/// One crossfade layer as its [`LayerRender`] sees it (#863).
///
/// Both fields are REACTIVE, read by the layer's `seq`: the `Cut` path updates a
/// layer's content IN PLACE (same `seq`) and `leaving` flips later, and a keyed
/// `<For>` does not re-run its children for an unchanged key — a captured value
/// would go stale (#496/#693/#716/#776 keyed-`<For>` reactive-field trap).
#[derive(Clone, Copy)]
pub struct CrossfadeLayer {
    /// The layer's content key.
    pub content: Signal<String>,
    /// `true` once the layer fades out (or is about to be removed). A block
    /// withdraws its line `data-role`s then, so a role always names the copy
    /// that stays (#863).
    pub leaving: Signal<bool>,
}

/// Draws ONE crossfade layer (#863). Called once per layer, when it mounts.
/// `Send + Sync` because the layers render inside a `<Show>`.
pub type LayerRender = Arc<dyn Fn(CrossfadeLayer) -> AnyView + Send + Sync>;

/// The generic content crossfade (#716; generalised in #863): one layer per
/// content value, drawn by `render`, stacked in one grid cell.
#[component]
pub fn Crossfade(
    /// The reactive content key (empty ⇒ no visible content). A `Memo` so the
    /// crossfade fires only on a genuine value change.
    content: Memo<String>,
    /// How a content change animates: `Cut` = instant, `Fade` = crossfade,
    /// `FadeThrough` = fade out, then fade the new content in (#834).
    transition: ContentTransition,
    /// `data-role` for the wrapper — the element the E2E targets.
    #[prop(into)]
    role: String,
    /// Extra CSS classes for the wrapper (e.g. `stream-text-block`).
    #[prop(into, optional)]
    wrapper_class: String,
    /// Fill the wrapper width (`minmax(0,1fr)`) so text wraps within the Frame —
    /// lyrics/verse. Off ⇒ the wrapper is content-sized.
    #[prop(optional)]
    fill: bool,
    /// Draws one layer from its content key (see [`LayerRender`]).
    render: LayerRender,
) -> impl IntoView {
    // `is_fade`: layers animate in/out. `through`: a change waits for the
    // fade-out before the new content is mounted (#834) instead of crossfading.
    let (is_fade, through, fade_ms) = match transition {
        ContentTransition::Cut => (false, false, 0u32),
        ContentTransition::Fade { duration_ms } => (true, false, duration_ms),
        ContentTransition::FadeThrough { duration_ms } => (true, true, duration_ms),
    };

    let layers = RwSignal::new(Vec::<ContentLayer>::new());
    // #834: the content waiting for the current fade-out (`FadeThrough` only).
    let pending = StoredValue::new(None::<String>);
    let next_seq = StoredValue::new(0u64);
    let bump = move || {
        let s = next_seq.get_value();
        next_seq.set_value(s + 1);
        s
    };

    // Drive the layer list off the content Memo. First run seeds the current
    // content; later runs (only fire on a genuine change) fade the old out / cut
    // it, and add the new (non-empty) content.
    Effect::new(move |prev: Option<String>| {
        let cur = content.get();
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
            Some(_) if is_fade => crossfade_change(layers, &cur, fade_ms, bump),
            Some(_) => cut_change(layers, &cur, bump),
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
    // One layer's DOM. Built once per layer (keyed on `seq`); cloned into the
    // `<For>` on every `<Show>` mount.
    let layer_view = move |l: ContentLayer| {
        let seq = l.seq;
        let fade = l.fade;
        // #496/#693: read `leaving` REACTIVELY by seq — a keyed `<For>` does not
        // re-run children when only the leaving flag flips, so a captured bool
        // would never apply the fade-out class.
        let leaving = Signal::derive(move || {
            layers
                .with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.leaving))
                .unwrap_or(true)
        });
        let class = move || {
            let mut c = String::from("stream-crossfade__layer");
            if fade {
                c.push_str(" stream-crossfade__layer--fade");
            }
            if leaving.get() {
                c.push_str(" stream-crossfade__layer--leaving");
            }
            c
        };
        // #776: the content is read REACTIVELY by seq too — the `Cut` path
        // updates a layer's content in place (same seq). `Fade` (new seq each
        // change) is unaffected.
        let layer_content = Signal::derive(move || {
            layers
                .with(|ls| ls.iter().find(|x| x.seq == seq).map(|x| x.text.clone()))
                .unwrap_or_default()
        });
        let style = if fade {
            format!("transition-duration:{fade_ms}ms;")
        } else {
            String::new()
        };
        view! {
            <div class=class data-role="stream-crossfade-layer" style=style>
                {render(CrossfadeLayer { content: layer_content, leaving })}
            </div>
        }
    };

    view! {
        <Show when=has_layers>
            <div class=wrapper_classes.clone() data-role=role.clone()>
                <For each=layers_for_each key=|l| l.seq children=layer_view.clone() />
            </div>
        </Show>
    }
}

/// A `Fade` change: every visible layer starts leaving (removed after its fade
/// plus the buffer, dispose-safe) and the new non-empty content mounts as a
/// fresh layer, which fades in through `@starting-style`.
fn crossfade_change(
    layers: RwSignal<Vec<ContentLayer>>,
    cur: &str,
    fade_ms: u32,
    bump: impl Fn() -> u64 + Copy + 'static,
) {
    let mut removing = Vec::new();
    layers.update(|ls| {
        for l in ls.iter_mut().filter(|l| !l.leaving) {
            l.leaving = true;
            removing.push(l.seq);
        }
        if !cur.is_empty() {
            ls.push(ContentLayer {
                seq: bump(),
                text: cur.to_string(),
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

/// A `Cut` change: update the existing (single) layer's content IN PLACE,
/// keeping its `seq` — the keyed `<For>` then reconciles the SAME DOM node
/// instead of destroying + recreating it each change (#776). Empty ⇒ drop the
/// layer (wrapper unmounts, count-0-on-clear preserved). The layer's content is
/// read REACTIVELY by seq in `Crossfade`, so an in-place change on an unchanged
/// key still reaches the DOM (#496/#693/#716 keyed-`<For>` reactive-field trap).
fn cut_change(layers: RwSignal<Vec<ContentLayer>>, cur: &str, bump: impl Fn() -> u64 + Copy) {
    if cur.is_empty() {
        layers.set(Vec::new());
    } else {
        layers.update(|ls| match ls.iter_mut().find(|l| !l.leaving) {
            Some(l) => l.text = cur.to_string(),
            None => ls.push(ContentLayer {
                seq: bump(),
                text: cur.to_string(),
                fade: false,
                leaving: false,
            }),
        });
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
