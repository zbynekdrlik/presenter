//! Image element for the stream output page (#709).
//!
//! Renders an uploaded asset (`GET /stream/assets/{asset_id}`, #708) inside its
//! `Frame` (percentages of the fixed 16:9 canvas), with the configured object
//! fit and opacity. Static per element — the def only changes on a refetch,
//! which remounts the whole scene, so there is no reactive state here.

use leptos::prelude::*;
use presenter_core::{Frame, ImageFit};

/// CSS `object-fit` value for a stream `ImageFit`.
fn object_fit(fit: ImageFit) -> &'static str {
    match fit {
        ImageFit::Contain => "contain",
        ImageFit::Cover => "cover",
        ImageFit::Stretch => "fill",
    }
}

#[component]
pub fn ElementImage(
    /// `stream_elements.id` — for E2E targeting + a stable DOM identity.
    id: i64,
    /// `stream_assets.id` referenced by the element props.
    asset_id: i64,
    fit: ImageFit,
    frame: Frame,
    opacity: f32,
    /// `z_order` — mirrored to `z-index` so stacking matches the def order even
    /// if the DOM order is ever perturbed.
    z: i32,
) -> impl IntoView {
    let src = format!("/stream/assets/{asset_id}");
    let container_style = format!(
        "left:{}%;top:{}%;width:{}%;height:{}%;opacity:{};z-index:{};",
        frame.x_pct, frame.y_pct, frame.w_pct, frame.h_pct, opacity, z
    );
    let img_style = format!("width:100%;height:100%;object-fit:{};", object_fit(fit));

    view! {
        <div
            class="stream-element stream-element--image"
            data-role="stream-element-image"
            data-element-id=id.to_string()
            data-asset-id=asset_id.to_string()
            style=container_style
        >
            <img src=src alt="" style=img_style />
        </div>
    }
}
