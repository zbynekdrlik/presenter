//! Custom `GstRTPHeaderExtension` subclass for the WebRTC **playout-delay**
//! RTP header extension (URI
//! `http://www.webrtc.org/experiments/rtp-hdrext/playout-delay`).
//!
//! # Why this exists
//!
//! A video-only WebRTC stream has no audio track to anchor the receiver's
//! clock, so Chromium's video jitter buffer has no drift-compensation signal
//! and grows unbounded — end-to-end latency climbs past 1s and never recovers.
//! The fix is to emit the **playout-delay** RTP header extension carrying a
//! small MAX delay. Chromium honors the MAX as a HARD cap on its jitter buffer
//! (`max_playout_delay`), bounding latency regardless of the missing audio
//! anchor.
//!
//! Stock GStreamer 1.24 ships NO playout-delay header-extension element, so we
//! implement one as a [`gstreamer_rtp::RTPHeaderExtension`] subclass. The
//! payloader (`rtph264pay` / `rtpvp8pay`) or `webrtcbin` then negotiates and
//! emits it on every outgoing RTP packet.
//!
//! # Payload format (per the WebRTC playout-delay spec)
//!
//! A STATIC 3-byte payload: a 12-bit MIN-delay followed by a 12-bit MAX-delay,
//! each in units of 10ms. We send MIN=0 (0ms) and MAX=20 (200ms):
//!
//! ```text
//!  0                   1                   2
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |       MIN delay       |       MAX delay       |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! Encoding: `byte0 = MIN>>4`, `byte1 = ((MIN&0xF)<<4)|(MAX>>8)`,
//! `byte2 = MAX&0xFF`. With MIN=0, MAX=20 the bytes are `[0x00, 0x00, 0x14]`.
//!
//! # Usage
//!
//! ```ignore
//! let ext = playout_delay::create();
//! ext.set_id(5); // pick a free RTP extension id (1..=14 for one-byte headers)
//! payloader.emit_by_name::<()>("add-extension", &[&ext]);
//! ```

use std::sync::Once;

use gst::glib;
use gstreamer as gst;
use gstreamer_rtp as gst_rtp;
use gstreamer_rtp::RTPHeaderExtension;
// These preludes re-export `gst::prelude::*` and `gst::subclass::prelude::*`
// respectively, plus the RTP-specific extension traits (`RTPHeaderExtensionExt`
// for `set_id`/`uri`, `RTPHeaderExtensionImpl` for the subclass methods).
use gstreamer_rtp::prelude::*;
use gstreamer_rtp::subclass::prelude::*;

/// The WebRTC playout-delay RTP header extension URI.
pub const PLAYOUT_DELAY_URI: &str = "http://www.webrtc.org/experiments/rtp-hdrext/playout-delay";

/// MIN playout delay, in units of 10ms. 0 = no minimum (0ms).
const MIN_DELAY_10MS: u16 = 0;
/// MAX playout delay, in units of 10ms. 20 = 200ms — a hard jitter-buffer cap.
const MAX_DELAY_10MS: u16 = 20;

/// The static 3-byte playout-delay payload (12-bit MIN || 12-bit MAX).
///
/// `[0x00, 0x00, 0x14]` for MIN=0, MAX=20.
const PAYLOAD: [u8; 3] = encode_payload(MIN_DELAY_10MS, MAX_DELAY_10MS);

/// Encode a 12-bit MIN-delay and 12-bit MAX-delay into the 3-byte playout-delay
/// payload, per the WebRTC spec. `const` so the payload is computed at compile
/// time and the byte layout is unit-testable.
const fn encode_payload(min_10ms: u16, max_10ms: u16) -> [u8; 3] {
    let min = min_10ms & 0x0FFF;
    let max = max_10ms & 0x0FFF;
    [
        (min >> 4) as u8,
        (((min & 0x0F) << 4) | (max >> 8)) as u8,
        (max & 0xFF) as u8,
    ]
}

mod imp {
    use super::*;

    /// Private instance struct for the playout-delay header extension. It holds
    /// no per-instance state — the payload is static.
    #[derive(Default)]
    pub struct PlayoutDelayExtension;

    #[glib::object_subclass]
    impl ObjectSubclass for PlayoutDelayExtension {
        const NAME: &'static str = "PresenterPlayoutDelayExtension";
        type Type = super::PlayoutDelayExtension;
        type ParentType = RTPHeaderExtension;
    }

    impl ObjectImpl for PlayoutDelayExtension {}

    impl GstObjectImpl for PlayoutDelayExtension {}

    impl ElementImpl for PlayoutDelayExtension {
        fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
            static METADATA: std::sync::OnceLock<gst::subclass::ElementMetadata> =
                std::sync::OnceLock::new();
            Some(METADATA.get_or_init(|| {
                gst::subclass::ElementMetadata::new(
                    "WebRTC playout-delay RTP header extension",
                    "Network/Extension/RTPHeader",
                    "Emits the WebRTC playout-delay RTP header extension to cap \
                     the receiver jitter buffer on video-only streams",
                    "Presenter",
                )
            }))
        }
        // RTP header extensions carry no pads, so the default empty
        // `pad_templates()` (returns `&[]`) is correct.
    }

    impl RTPHeaderExtensionImpl for PlayoutDelayExtension {
        // Associates this subclass with the playout-delay URI. The base
        // `IsSubclassable::class_init` calls
        // `gst_rtp_header_extension_class_set_uri(klass, Self::URI)` for us, so
        // `ext.uri()` returns this string automatically — no manual `set_uri`.
        const URI: &'static str = PLAYOUT_DELAY_URI;

        /// We support both one-byte and two-byte RTP header-extension framings.
        fn supported_flags(&self) -> gst_rtp::RTPHeaderExtensionFlags {
            gst_rtp::RTPHeaderExtensionFlags::ONE_BYTE | gst_rtp::RTPHeaderExtensionFlags::TWO_BYTE
        }

        /// The extension is always exactly 3 bytes.
        fn max_size(&self, _input: &gst::BufferRef) -> usize {
            PAYLOAD.len()
        }

        /// Write the static 3-byte playout-delay payload into the output slice.
        fn write(
            &self,
            _input: &gst::BufferRef,
            _write_flags: gst_rtp::RTPHeaderExtensionFlags,
            _output: &gst::BufferRef,
            output_data: &mut [u8],
        ) -> Result<usize, gst::LoggableError> {
            if output_data.len() < PAYLOAD.len() {
                return Err(gst::loggable_error!(
                    gst::CAT_RUST,
                    "playout-delay output buffer too small: have {}, need {}",
                    output_data.len(),
                    PAYLOAD.len()
                ));
            }
            output_data[..PAYLOAD.len()].copy_from_slice(&PAYLOAD);
            Ok(PAYLOAD.len())
        }

        /// We only SEND this extension — reading is a no-op.
        fn read(
            &self,
            _read_flags: gst_rtp::RTPHeaderExtensionFlags,
            _input_data: &[u8],
            _output: &mut gst::BufferRef,
        ) -> Result<(), gst::LoggableError> {
            Ok(())
        }
    }
}

glib::wrapper! {
    /// The playout-delay RTP header extension element.
    pub struct PlayoutDelayExtension(ObjectSubclass<imp::PlayoutDelayExtension>)
        @extends RTPHeaderExtension, gst::Element, gst::Object;
}

/// Force one-time registration of the [`PlayoutDelayExtension`] GType. glib
/// registers the type lazily on first use, but doing it explicitly here makes
/// the registration deterministic and independent of construction order.
//
// `dead_code` is allowed until a consumer pipeline wires this extension into a
// payloader via `add-extension` (intentionally out of scope here — the module
// is self-contained; consumers.rs is not touched).
#[allow(dead_code)]
fn ensure_registered() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // `static_type()` triggers the `#[glib::object_subclass]` registration.
        let _ = PlayoutDelayExtension::static_type();
    });
}

/// Create a NEW playout-delay RTP header extension instance.
///
/// Returns it as a [`gstreamer_rtp::RTPHeaderExtension`] so the caller can call
/// [`RTPHeaderExtensionExt::set_id`] before handing it to a payloader /
/// `webrtcbin` via the `add-extension` signal:
///
/// ```ignore
/// let ext = playout_delay::create();
/// ext.set_id(5);
/// payloader.emit_by_name::<()>("add-extension", &[&ext]);
/// ```
///
/// `ext.uri()` is guaranteed to return [`PLAYOUT_DELAY_URI`].
//
// `dead_code` is allowed until a consumer pipeline calls this and wires the
// returned extension into a payloader (out of scope for this module-only task).
#[allow(dead_code)]
pub fn create() -> RTPHeaderExtension {
    ensure_registered();
    glib::Object::new::<PlayoutDelayExtension>().upcast::<RTPHeaderExtension>()
}

/// Create a playout-delay extension with its RTP extmap `id` already set to the
/// id the browser negotiated in its offer (e.g. 5 for Chromium). Setting a
/// VALID negotiated id is mandatory: an unset (0) id corrupts the RTP header
/// extension block and the receiver drops every packet (black video). The
/// caller parses the id from the offer's `a=extmap:<id> …playout-delay` line.
pub fn create_with_id(id: u32) -> RTPHeaderExtension {
    use gstreamer_rtp::prelude::RTPHeaderExtensionExt;
    let ext = create();
    ext.set_id(id);
    ext
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_bytes_match_spec() {
        // MIN=0 (0ms), MAX=20 (200ms) → [0x00, 0x00, 0x14].
        assert_eq!(PAYLOAD, [0x00, 0x00, 0x14]);
    }

    #[test]
    fn encode_payload_packs_min_and_max() {
        // MIN=1 (10ms): 0x001 over 12 bits, MAX=0x123 over 12 bits.
        // byte0 = 0x001>>4 = 0x00
        // byte1 = ((0x001 & 0xF)<<4) | (0x123>>8) = (0x1<<4)|0x1 = 0x11
        // byte2 = 0x123 & 0xFF = 0x23
        assert_eq!(encode_payload(0x001, 0x123), [0x00, 0x11, 0x23]);
    }

    #[test]
    fn create_returns_playout_delay_uri() {
        gst::init().expect("gstreamer init");
        let ext = create();
        assert_eq!(ext.uri().as_deref(), Some(PLAYOUT_DELAY_URI));
    }

    #[test]
    fn create_accepts_set_id() {
        gst::init().expect("gstreamer init");
        let ext = create();
        ext.set_id(5);
        // No panic / no crash is the assertion; set_id has no getter here.
    }
}
