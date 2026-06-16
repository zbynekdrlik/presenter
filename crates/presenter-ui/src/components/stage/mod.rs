pub mod api_stage;
pub mod bible_layout;
pub mod bible_overlay;
pub mod camera_crew;
pub mod ndi_fullscreen;
pub mod ndi_video;
mod ndi_watchdog;
pub mod preach_layout;
pub mod status_bar;
pub mod timer_layout;
pub mod wake_lock;
pub mod worship_pp;
pub mod worship_snv;

/// Map an `ndi_status` string (from `LiveEvent::NdiConnectionStatus`) to the
/// user-facing overlay text rendered on top of the NDI video. The three
/// expected values are produced by `presenter-server`:
/// - `""` → no NDI activity (no overlay)
/// - `"connecting"` → pipeline starting; show "Connecting…"
/// - `"disconnected"` → pipeline lost frames; show "Signal Lost — Reconnecting…"
/// - `"failed: <reason>"` → pipeline.start() returned Err; show the reason so
///   the operator can actually see what's wrong.
pub fn ndi_status_text(status: &str) -> String {
    if status == "disconnected" {
        "Signal Lost — Reconnecting...".to_string()
    } else if status == "connecting" {
        "Connecting...".to_string()
    } else if let Some(reason) = status.strip_prefix("failed: ") {
        format!("NDI pipeline failed: {reason}")
    } else if status == "failed" {
        "NDI pipeline failed".to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ndi_status_text;

    /// Regression test for the bug surfaced 2026-05-19 where the stage
    /// `.stage-ndi__overlay` rendered "Connecting…" indefinitely even after
    /// the WebRTC pipeline was actively streaming video.
    ///
    /// The state machine: `presenter-server` publishes `LiveEvent::
    /// NdiConnectionStatus { status }` over WS; the WASM stage stores
    /// `status` in `StageContext::ndi_status` and renders the overlay only
    /// when this mapping returns a non-empty string. If the server fails to
    /// publish a terminal "connected" / "failed: …" status, the WASM
    /// keeps the initial "connecting" value forever.
    ///
    /// Asserts here are the exact contract the server-side `state/
    /// integrations.rs::activate_video_source` is bound to: on success it
    /// publishes `status="connected"`, on failure it publishes
    /// `status="failed: <reason>"`. Both must yield an empty string OR a
    /// human-readable reason — never the literal `"connecting"` text.
    #[test]
    fn connected_status_produces_no_overlay_text() {
        // The 'connected' status is the load-bearing case: it MUST collapse
        // to an empty string so the <Show when=...> guard hides the overlay.
        assert_eq!(ndi_status_text("connected"), "");
    }

    #[test]
    fn empty_status_produces_no_overlay_text() {
        // Initial / deactivated state — no overlay.
        assert_eq!(ndi_status_text(""), "");
    }

    #[test]
    fn connecting_status_produces_connecting_overlay_text() {
        assert_eq!(ndi_status_text("connecting"), "Connecting...");
    }

    #[test]
    fn disconnected_status_produces_signal_lost_overlay_text() {
        assert_eq!(
            ndi_status_text("disconnected"),
            "Signal Lost — Reconnecting...",
        );
    }

    #[test]
    fn failed_with_reason_renders_full_reason_text() {
        // The format the server emits when start_pipeline returns Err.
        // The reason must surface to the operator verbatim so they see WHY.
        assert_eq!(
            ndi_status_text("failed: no hardware H264 encoder registered"),
            "NDI pipeline failed: no hardware H264 encoder registered",
        );
    }

    #[test]
    fn failed_without_reason_renders_bare_message() {
        // Defensive: if the server publishes `status="failed"` without a
        // reason (e.g. truncated message), the overlay still surfaces SOME
        // signal instead of falling back to "" (silent failure).
        assert_eq!(ndi_status_text("failed"), "NDI pipeline failed");
    }

    #[test]
    fn unknown_status_produces_no_overlay_text() {
        // Any future status string the WASM doesn't know about collapses to
        // no-overlay. Preferable to a literal "{status}" leak in the UI.
        assert_eq!(ndi_status_text("starting"), "");
        assert_eq!(ndi_status_text("streaming"), "");
        assert_eq!(ndi_status_text("totally-new-state-string"), "");
    }
}
