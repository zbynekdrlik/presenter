//! Nameplate live-preview channel (#779) — the editor's "Prehrať" button.
//!
//! The editor previews a plate's animation WITHOUT broadcasting to the real
//! outputs by posting a fake `ActiveNameplate` into the `?preview=1` output
//! iframe via `postMessage`; the output page feeds it into `StreamContext.nameplate`
//! so the actual `ElementLowerThird` animates. Sibling of `draft_preview.rs`
//! (element props) — a SEPARATE message type so a plate preview and a prop draft
//! never collide. `active: None` clears the preview (hide).

use leptos::prelude::*;
use presenter_core::ActiveNameplate;
use serde::{Deserialize, Serialize};

/// The `postMessage` discriminator for a nameplate preview.
pub const NAMEPLATE_PREVIEW_TYPE: &str = "presenter-stream-nameplate-preview";

/// The preview `postMessage` payload (JSON). `active: None` clears the plate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameplatePreviewMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub active: Option<ActiveNameplate>,
}

/// Serialize a nameplate preview push to a JSON string for `postMessage`.
pub fn serialize_message(active: Option<ActiveNameplate>) -> String {
    let msg = NameplatePreviewMessage {
        kind: NAMEPLATE_PREVIEW_TYPE.to_string(),
        active,
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

/// Parse an incoming preview message; `None` if it is not our message.
pub fn parse_message(data: &str) -> Option<NameplatePreviewMessage> {
    let msg: NameplatePreviewMessage = serde_json::from_str(data).ok()?;
    (msg.kind == NAMEPLATE_PREVIEW_TYPE).then_some(msg)
}

/// Install the preview-only window `message` listener (output page, `?preview=1`
/// only). Origin-checked; feeds `nameplate` (the shared `StreamContext.nameplate`).
#[cfg(not(target_arch = "wasm32"))]
pub fn install_nameplate_preview_listener(_nameplate: RwSignal<Option<ActiveNameplate>>) {}

/// Install the preview-only window `message` listener (output page, `?preview=1`
/// only). Origin-checked; feeds `nameplate`.
#[cfg(target_arch = "wasm32")]
pub fn install_nameplate_preview_listener(nameplate: RwSignal<Option<ActiveNameplate>>) {
    use leptos::wasm_bindgen::prelude::*;

    let Some(window) = leptos::web_sys::window() else {
        return;
    };
    let own_origin = window.location().origin().unwrap_or_default();

    let cb = Closure::<dyn FnMut(leptos::web_sys::MessageEvent)>::new(
        move |ev: leptos::web_sys::MessageEvent| {
            if ev.origin() != own_origin {
                return;
            }
            let Some(text) = ev.data().as_string() else {
                return;
            };
            let Some(msg) = parse_message(&text) else {
                return;
            };
            nameplate.set(msg.active);
        },
    );

    if window
        .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref())
        .is_err()
    {
        leptos::logging::warn!("stream preview: failed to register nameplate message listener");
    }
    cb.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::NameplateSource;

    fn active() -> ActiveNameplate {
        ActiveNameplate {
            source: NameplateSource::Person,
            nameplate_id: Some(3),
            primary: "Ján".to_string(),
            secondary: "pastor".to_string(),
            seq: 1,
        }
    }

    #[test]
    fn message_roundtrips_through_json() {
        let json = serialize_message(Some(active()));
        let parsed = parse_message(&json).expect("our message");
        assert_eq!(parsed.active, Some(active()));
    }

    #[test]
    fn clear_message_has_null_active() {
        let json = serialize_message(None);
        assert!(json.contains("\"active\":null"));
        assert_eq!(parse_message(&json).unwrap().active, None);
    }

    #[test]
    fn foreign_message_is_rejected() {
        assert!(parse_message("{\"type\":\"other\",\"active\":null}").is_none());
        assert!(parse_message("not json").is_none());
    }
}
