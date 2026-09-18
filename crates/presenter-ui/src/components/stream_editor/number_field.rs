//! Buffered numeric frame field (#777).
//!
//! A plain `<input type=number>` bound directly to a parsed signal fights the
//! caret whenever the text is momentarily unparseable (`"-"`, `"1."`, `""`) —
//! the same defect class lane #776 fixes for the opacity input. This is our OWN
//! helper for the FRAME fields (x/y/w/h), deliberately NOT `percent_input.rs`
//! (that name belongs to #776). Typing edits a local `String` buffer; the parsed
//! value is CLAMPED to `[min,max]` and committed to the shared draft on
//! change/blur, so a save can never 422 on the frame.

use leptos::prelude::*;
use presenter_core::StreamElementProps;

use super::frame_math::round1;
use super::props_access::{read_frame, with_frame_mut};

/// Which of the four `Frame` fields a [`FrameNumberField`] edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameField {
    X,
    Y,
    W,
    H,
}

impl FrameField {
    fn read(self, props: &StreamElementProps) -> f32 {
        let f = read_frame(props);
        match self {
            FrameField::X => f.x_pct,
            FrameField::Y => f.y_pct,
            FrameField::W => f.w_pct,
            FrameField::H => f.h_pct,
        }
    }

    fn write(self, props: &mut StreamElementProps, v: f32) {
        with_frame_mut(props, |fr| match self {
            FrameField::X => fr.x_pct = v,
            FrameField::Y => fr.y_pct = v,
            FrameField::W => fr.w_pct = v,
            FrameField::H => fr.h_pct = v,
        });
    }
}

/// Format a frame value cleanly (one decimal, no trailing `.0`).
fn format_num(v: f32) -> String {
    round1(v).to_string()
}

/// A buffered numeric input for one `Frame` field of the shared draft.
#[component]
pub fn FrameNumberField(
    draft: RwSignal<StreamElementProps>,
    field: FrameField,
    label: &'static str,
    role: &'static str,
    min: f32,
    max: f32,
    step: f32,
) -> impl IntoView {
    let buffer = RwSignal::new(String::new());
    let editing = RwSignal::new(false);

    // While editing, the input shows the RAW buffer (never reformatted mid-type,
    // so the caret is never disturbed — the #776 defect); otherwise the committed,
    // formatted draft value (kept live by drags on the canvas overlay).
    let committed = move || format_num(field.read(&draft.get()));
    let display = move || {
        if editing.get() {
            buffer.get()
        } else {
            committed()
        }
    };

    // Commit the parsed+clamped value to the shared draft on EVERY keystroke, so
    // the canvas outline + live preview follow the field in real time. Clamping to
    // the legal range here is what makes a save impossible to 422 on the frame.
    let commit_live = move |raw: &str| {
        if let Ok(v) = raw.parse::<f32>() {
            draft.update(|p| field.write(p, round1(v.clamp(min, max))));
        }
    };
    // On blur/change, snap the visible text to the clamped, formatted value.
    let finalize = move |raw: &str| {
        if let Ok(v) = raw.parse::<f32>() {
            let c = round1(v.clamp(min, max));
            draft.update(|p| field.write(p, c));
            buffer.set(format_num(c));
        } else {
            buffer.set(committed());
        }
        editing.set(false);
    };

    view! {
        <label class="stream-editor__field">
            <span>{label}</span>
            <input
                type="number"
                step=step.to_string()
                min=min.to_string()
                max=max.to_string()
                data-role=role
                prop:value=display
                on:focus=move |_| {
                    buffer.set(committed());
                    editing.set(true);
                }
                on:input=move |ev| {
                    let raw = event_target_value(&ev);
                    buffer.set(raw.clone());
                    editing.set(true);
                    commit_live(&raw);
                }
                on:change=move |ev| finalize(&event_target_value(&ev))
                on:blur=move |ev| finalize(&event_target_value(&ev))
            />
        </label>
    }
}
