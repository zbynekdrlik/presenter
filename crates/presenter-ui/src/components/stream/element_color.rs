//! Solid-color element for the stream output page (#753).
//!
//! Renders a static solid fill inside its `Frame` (percentages of the fixed
//! 16:9 canvas) with a `background-color` + `opacity`. Typically placed BELOW a
//! content element (a verse) via `z_order` to give a semi-transparent band
//! behind text. Static per element — the def only changes on a refetch, which
//! remounts the whole scene, so there is no reactive state here (mirrors
//! `ElementImage`); scene-level crossfade (#716/#752) covers transitions.

use leptos::prelude::*;

use super::style::frame_css;

#[component]
pub fn ElementColor(
    /// `stream_elements.id` — for E2E targeting + a stable DOM identity.
    id: i64,
    /// Hex fill color (`#rrggbb` / `#rrggbbaa`).
    color: String,
    opacity: f32,
    frame: presenter_core::Frame,
    /// `z_order` — mirrored to `z-index` so the band stacks below higher-z
    /// content even if the DOM order is ever perturbed.
    z: i32,
) -> impl IntoView {
    // Reuse the shared frame→CSS helper (left/top/width/height/z-index), then add
    // the color-specific background-color + opacity — same helper the text
    // elements use (#710), so the frame mapping stays in one place.
    let container_style = format!(
        "{}background-color:{};opacity:{};",
        frame_css(&frame, z),
        color,
        opacity
    );
    view! {
        <div
            class="stream-element stream-element--color"
            data-role="stream-element-color"
            data-element-id=id.to_string()
            style=container_style
        ></div>
    }
}
