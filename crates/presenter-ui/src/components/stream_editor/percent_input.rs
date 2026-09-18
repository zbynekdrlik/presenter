//! Buffered integer-percent input for a 0..=1 opacity field (#776).
//!
//! The image/color opacity is stored on the wire as a 0..=1 float, but the owner
//! edits it far more naturally as an integer PERCENT (0–100). This control shows
//! + accepts percent and converts in the form only — no wire/API/schema change.
//!
//! It also fixes the "cannot type" bug: the old field parsed every keystroke to
//! f32 and wrote it straight back to the same `draft` the value closure read, so
//! an intermediate `"0."` was dropped and the field reset mid-typing. Worse, a
//! whole number like `50` reached the server and 422'd (`validate_opacity`,
//! `0.0..=1.0`). `PercentInput` buffers the raw text in a LOCAL signal while
//! typing and COMMITS only on `change` (blur / Enter), so a controlled re-render
//! never fights the user. On commit it clamps to 0–100 (with an inline Slovak
//! hint) and writes `pct/100` into the draft, so the server bound is never hit.

use leptos::prelude::*;
use presenter_core::StreamElementProps;

use super::props_access::{read_opacity, with_opacity_mut};

/// Convert a stored 0..=1 opacity to an integer percent (0–100), rounded.
pub fn opacity_to_pct(opacity: f32) -> i64 {
    (opacity * 100.0).round() as i64
}

/// Convert an integer percent (already clamped) back to a 0..=1 opacity.
pub fn pct_to_opacity(pct: i64) -> f32 {
    pct as f32 / 100.0
}

/// Clamp an arbitrary integer to the valid percent range 0–100.
pub fn clamp_pct(pct: i64) -> i64 {
    pct.clamp(0, 100)
}

/// The percent to show for the draft's current opacity; falls back to 100 for a
/// kind that has no opacity (never rendered for such a kind).
fn draft_pct(props: &StreamElementProps) -> i64 {
    read_opacity(props).map(opacity_to_pct).unwrap_or(100)
}

/// A percent (0–100) opacity input bound to `draft`. Commits on blur/Enter.
#[component]
pub fn PercentInput(
    /// The element draft; opacity is read/written via `props_access`.
    draft: RwSignal<StreamElementProps>,
    /// `data-role` for the input (e.g. `"stream-image-opacity"`).
    role: &'static str,
) -> impl IntoView {
    // Local text buffer while typing; committed on `change` (blur/Enter) only, so
    // a controlled per-keystroke re-render never fights the user (#776).
    let text = RwSignal::new(String::new());
    let hint = RwSignal::new(String::new());

    // Re-seed the buffer whenever the draft's opacity changes externally (element
    // re-selected, save refetch) — round(opacity*100). Also clear any lingering
    // clamp hint so a stale "Rozsah 0–100 %" from a prior element never sticks.
    Effect::new(move |_| {
        text.set(draft_pct(&draft.get()).to_string());
        hint.set(String::new());
    });

    let commit = move || {
        match text.get_untracked().trim().parse::<i64>() {
            Ok(n) => {
                let clamped = clamp_pct(n);
                hint.set(if clamped == n {
                    String::new()
                } else {
                    "Rozsah 0–100 %".to_string()
                });
                draft.update(|p| with_opacity_mut(p, |o| *o = pct_to_opacity(clamped)));
                text.set(clamped.to_string());
            }
            // Empty / non-integer entry: discard it and restore the field from the
            // draft's current value — never write garbage into the draft.
            Err(_) => {
                text.set(draft_pct(&draft.get_untracked()).to_string());
                hint.set(String::new());
            }
        }
    };

    view! {
        <label class="stream-editor__field">
            <span>"Priehľadnosť (%)"</span>
            <input
                type="number"
                min="0"
                max="100"
                step="1"
                data-role=role
                prop:value=move || text.get()
                on:input=move |ev| text.set(event_target_value(&ev))
                on:change=move |_| commit()
            />
            <Show when=move || !hint.get().is_empty()>
                <span class="stream-editor__field-hint" data-role="stream-opacity-hint">
                    {move || hint.get()}
                </span>
            </Show>
        </label>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_to_pct_rounds() {
        assert_eq!(opacity_to_pct(0.0), 0);
        assert_eq!(opacity_to_pct(1.0), 100);
        assert_eq!(opacity_to_pct(0.35), 35);
        assert_eq!(opacity_to_pct(0.5), 50);
        // Rounds to the nearest whole percent.
        assert_eq!(opacity_to_pct(0.333), 33);
        assert_eq!(opacity_to_pct(0.336), 34);
    }

    #[test]
    fn pct_to_opacity_divides_by_100() {
        assert_eq!(pct_to_opacity(0), 0.0);
        assert_eq!(pct_to_opacity(100), 1.0);
        assert_eq!(pct_to_opacity(35), 0.35);
        assert_eq!(pct_to_opacity(50), 0.5);
    }

    #[test]
    fn round_trip_35() {
        // The design's canonical round trip: 35 -> 0.35 -> 35.
        assert_eq!(opacity_to_pct(pct_to_opacity(35)), 35);
    }

    #[test]
    fn round_trip_every_percent() {
        for pct in 0..=100 {
            assert_eq!(
                opacity_to_pct(pct_to_opacity(pct)),
                pct,
                "pct {pct} must round-trip through the 0..=1 wire form"
            );
        }
    }

    #[test]
    fn clamp_pct_bounds() {
        assert_eq!(clamp_pct(-5), 0);
        assert_eq!(clamp_pct(0), 0);
        assert_eq!(clamp_pct(50), 50);
        assert_eq!(clamp_pct(100), 100);
        assert_eq!(clamp_pct(150), 100);
        assert_eq!(clamp_pct(1000), 100);
    }
}
