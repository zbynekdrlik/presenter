//! Live-preview draft-override channel (#777).
//!
//! The editor pushes the operator's UNSAVED element edits into the REAL output
//! iframe via `window.postMessage`, so the preview is the actual renderer (fonts,
//! transitions, timers included) and equals the output by construction — no
//! save round trip to see a change.
//!
//! Protocol: the editor posts a JSON string `{type:"presenter-stream-draft",
//! elementId, props}` to the iframe's `contentWindow`; `props` is the serde
//! form of [`StreamElementProps`] (tag `kind`, camelCase `frame`), `null` to
//! clear. The OUTPUT page installs the listener ONLY when `?preview=1`
//! (production outputs never do), verifies `event.origin` == its own origin, and
//! feeds a [`StreamDraftOverride`] context that `scene_render` resolves an
//! element's props through. This module is the ONE place the wire shape lives, so
//! the editor and the output page can never drift.

use leptos::prelude::*;
use presenter_core::StreamElementProps;
use serde::{Deserialize, Serialize};

/// The `postMessage` discriminator. A message with any other `type` is ignored.
pub const DRAFT_MESSAGE_TYPE: &str = "presenter-stream-draft";

/// The draft-preview `postMessage` payload (JSON). `props: None` clears the
/// override (nothing to preview / no element selected).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub element_id: Option<i64>,
    pub props: Option<StreamElementProps>,
}

/// The preview-only draft override the output page feeds into `scene_render`:
/// `Some((element_id, props))` replaces that element's stored props while
/// editing. Provided as a Leptos context ONLY on a `?preview=1` page.
#[derive(Debug, Clone, Copy)]
pub struct StreamDraftOverride(pub RwSignal<Option<(i64, StreamElementProps)>>);

/// Resolve an element's effective props against the override: the override wins
/// only for its own `element_id`; otherwise the stored props are used.
pub fn resolve_props(
    override_sig: Option<StreamDraftOverride>,
    element_id: i64,
    stored: &StreamElementProps,
) -> StreamElementProps {
    override_sig
        .and_then(|o| o.0.get())
        .and_then(|(id, props)| (id == element_id).then_some(props))
        .unwrap_or_else(|| stored.clone())
}

/// Serialize a draft push message to a JSON string for `postMessage`.
pub fn serialize_message(element_id: Option<i64>, props: Option<StreamElementProps>) -> String {
    let msg = DraftMessage {
        kind: DRAFT_MESSAGE_TYPE.to_string(),
        element_id,
        props,
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

/// Parse an incoming `postMessage` body; `None` if it is not our message.
pub fn parse_message(data: &str) -> Option<DraftMessage> {
    let msg: DraftMessage = serde_json::from_str(data).ok()?;
    (msg.kind == DRAFT_MESSAGE_TYPE).then_some(msg)
}

/// Install the preview-only window `message` listener (output page, `?preview=1`
/// only). Origin-checked against this page's own origin; feeds `override_sig`.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_preview_listener(_override_sig: RwSignal<Option<(i64, StreamElementProps)>>) {}

/// Install the preview-only window `message` listener (output page, `?preview=1`
/// only). Origin-checked against this page's own origin; feeds `override_sig`.
#[cfg(target_arch = "wasm32")]
pub fn install_preview_listener(override_sig: RwSignal<Option<(i64, StreamElementProps)>>) {
    use leptos::wasm_bindgen::prelude::*;

    let Some(window) = leptos::web_sys::window() else {
        return;
    };
    let own_origin = window.location().origin().unwrap_or_default();

    let cb = Closure::<dyn FnMut(leptos::web_sys::MessageEvent)>::new(
        move |ev: leptos::web_sys::MessageEvent| {
            // Cross-origin frames must never be able to drive the preview.
            if ev.origin() != own_origin {
                return;
            }
            let Some(text) = ev.data().as_string() else {
                return;
            };
            let Some(msg) = parse_message(&text) else {
                return;
            };
            match (msg.element_id, msg.props) {
                (Some(id), Some(props)) => override_sig.set(Some((id, props))),
                _ => override_sig.set(None),
            }
        },
    );

    if window
        .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref())
        .is_err()
    {
        leptos::logging::warn!("stream preview: failed to register draft message listener");
    }
    // The output page lives for the iframe's lifetime; forget matches the crate.
    cb.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{Frame, ImageFit};

    fn img(x: f32) -> StreamElementProps {
        StreamElementProps::Image {
            asset_id: 1,
            fit: ImageFit::Contain,
            frame: Frame {
                x_pct: x,
                y_pct: 10.0,
                w_pct: 20.0,
                h_pct: 20.0,
            },
            opacity: 1.0,
        }
    }

    #[test]
    fn message_roundtrips_through_json() {
        let json = serialize_message(Some(7), Some(img(12.5)));
        let parsed = parse_message(&json).expect("our message");
        assert_eq!(parsed.element_id, Some(7));
        assert_eq!(parsed.props, Some(img(12.5)));
    }

    #[test]
    fn clear_message_has_null_props() {
        let json = serialize_message(None, None);
        assert!(json.contains("\"props\":null"));
        let parsed = parse_message(&json).expect("our message");
        assert_eq!(parsed.element_id, None);
        assert_eq!(parsed.props, None);
    }

    #[test]
    fn foreign_message_is_rejected() {
        assert!(parse_message("{\"type\":\"other\",\"elementId\":1,\"props\":null}").is_none());
        assert!(parse_message("not json").is_none());
    }

    #[test]
    fn props_serialize_with_kind_tag_and_camelcase_frame() {
        let json = serialize_message(Some(1), Some(img(5.0)));
        assert!(json.contains("\"kind\":\"image\""));
        assert!(json.contains("\"xPct\":5"));
    }
}
