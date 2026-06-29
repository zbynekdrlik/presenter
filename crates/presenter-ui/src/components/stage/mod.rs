pub mod api_stage;
pub mod bible_layout;
pub mod bible_overlay;
pub mod camera_crew;
mod ndi_beacon;
mod ndi_frame_stats;
mod ndi_ice;
pub mod ndi_fullscreen;
mod ndi_health_ticker;
mod ndi_profile;
mod ndi_reload_guard;
pub mod ndi_video;
mod ndi_watchdog;
pub mod preach_layout;
pub mod status_bar;
pub mod timer_layout;
pub mod wake_lock;
pub mod worship_pp;
mod worship_pp_helpers;
pub mod worship_snv;

/// How the stage should present an `ndi_status` over the NDI video area.
///
/// Decouples the on-screen TREATMENT (neutral gray placeholder vs alarming red
/// error overlay vs nothing) from the status string, so a source that is simply
/// OFF/silent (`no-signal`) is shown as a calm "waiting" placeholder rather than
/// a red "pipeline failed" error (#448).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdiOverlayKind {
    /// A stream is flowing (or no NDI activity) → show nothing over the video.
    None,
    /// Expected, non-error states — source configured but not yet producing
    /// (`no-signal`) or the pipeline is still starting (`connecting`). Render a
    /// calm GRAY placeholder; never the red error overlay (#448).
    Neutral,
    /// A genuine problem — the pipeline failed to build/run (`failed[: reason]`)
    /// or lost a previously-good signal (`disconnected`). Render the RED overlay.
    Error,
}

/// Classify an `ndi_status` into how it should be shown over the NDI video.
///
/// `no-signal` (the source is configured but its broadcaster is silent / not
/// producing — an EXPECTED state, not a failure) and `connecting` are NEUTRAL.
/// `failed`/`failed: <reason>` (a real pipeline failure) and `disconnected` (a
/// previously-good signal was lost) are ERRORs. Everything else — `connected`,
/// `""`, `streaming`, unknown — is `None` (a stream is flowing or there is no
/// NDI activity). See #448: an off/silent source must not paint the stage red.
pub fn ndi_overlay_kind(status: &str) -> NdiOverlayKind {
    if status == "no-signal" || status == "connecting" {
        // Expected, non-error states → calm gray placeholder (#448).
        NdiOverlayKind::Neutral
    } else if status == "disconnected" || status == "failed" || status.starts_with("failed: ") {
        // Genuine problems → red overlay.
        NdiOverlayKind::Error
    } else {
        // `connected` / `""` / `streaming` / unknown → a stream is flowing or
        // there is no NDI activity → nothing over the video.
        NdiOverlayKind::None
    }
}

/// Decide whether the NDI fullscreen layout should render its NEUTRAL COVERING
/// placeholder (`stage-ndi__placeholder--cover`) over the `<video>` (#500).
///
/// The cover hides the bare `<video>` while a configured source is in a neutral,
/// non-error state (`connecting` / `no-signal`). But that gray cover must NOT
/// hide a video that is ALREADY decoding frames. A late-joining stage client
/// (the operator preview iframe, or any stage box that loads/reloads after the
/// last broadcast) seeds `ndi_status = "connecting"` and stays there for up to
/// ~30s until the server's next NDI-status tick, while the WHEP `<video>`
/// decodes immediately — so the cover would hide live video for ~30s. Gating it
/// additionally on `!frames_live` makes the cover reflect REALITY (frames on
/// screen) rather than the lagging server status.
///
/// `frames_live` is true while frames are actually presenting (set per presented
/// frame by `NdiVideo`'s rVFC observer / currentTime proxy, flipped false by the
/// health ticker once frames go stale). The genuine-failure ERROR overlay
/// (`NdiOverlayKind::Error`) is a SEPARATE gate and is intentionally unaffected:
/// a failed/disconnected source has no frames, so errors still surface.
pub fn should_show_neutral_cover(ndi_active: bool, status: &str, frames_live: bool) -> bool {
    // The cover shows ONLY when a source is active, its status is neutral
    // (connecting / no-signal), AND no frames are currently presenting. The
    // `!frames_live` term is the #500 fix: a late-joining client whose status is
    // a stale `connecting` but whose `<video>` is already decoding drops the
    // cover immediately instead of hiding live video for ~30s.
    ndi_active && ndi_overlay_kind(status) == NdiOverlayKind::Neutral && !frames_live
}

/// Map an `ndi_status` string (from `LiveEvent::NdiConnectionStatus`) to the
/// user-facing text rendered over the NDI video. The expected values are
/// produced by `presenter-server`:
/// - `""` / `"connected"` → no NDI activity / a stream is flowing (no text)
/// - `"no-signal"` → source configured but broadcaster silent/not producing;
///   show a calm "Waiting for video source…" placeholder (#448 — NOT an error)
/// - `"connecting"` → pipeline starting; show "Connecting…"
/// - `"disconnected"` → pipeline lost frames; show "Signal Lost — Reconnecting…"
/// - `"failed: <reason>"` → pipeline.start() returned Err; show the reason so
///   the operator can actually see what's wrong.
pub fn ndi_status_text(status: &str) -> String {
    if status == "no-signal" {
        "Waiting for video source…".to_string()
    } else if status == "disconnected" {
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
    use super::{ndi_overlay_kind, ndi_status_text, should_show_neutral_cover, NdiOverlayKind};

    // ── #500: the Neutral covering placeholder must reflect whether frames are
    // ACTUALLY presenting, not just the lagging server status ────────────────
    //
    // Live on prod 2026-06-29 (v0.4.170), a late-joining stage client (the
    // operator preview iframe) held `ndi_status="connecting"` for up to ~30s
    // while the WHEP `<video>` was already decoding (1280x720, readyState=4,
    // "VIDEO · 52 MS") — yet the gray "Connecting…" cover hid that live video
    // because it was gated ONLY on the server status. The cover must drop the
    // moment frames are presenting (frames_live), and reappear when they stop.

    #[test]
    fn neutral_cover_hidden_when_frames_are_live() {
        // connecting + frames live ⇒ NO cover (the late-join bug: the WHEP
        // video is already on screen; the gray cover must not hide it).
        assert!(!should_show_neutral_cover(true, "connecting", true));
        // no-signal + frames live ⇒ NO cover (presenting frames win over a
        // stale neutral status).
        assert!(!should_show_neutral_cover(true, "no-signal", true));
    }

    #[test]
    fn neutral_cover_shown_when_no_frames_yet() {
        // connecting / no-signal with NO frames ⇒ cover (the genuine pre-video
        // state — #448: hide the bare <video> + native play-arrow).
        assert!(should_show_neutral_cover(true, "connecting", false));
        assert!(should_show_neutral_cover(true, "no-signal", false));
    }

    #[test]
    fn neutral_cover_requires_active_ndi() {
        // No active source ⇒ never the neutral cover (the "No video source
        // configured" placeholder covers that case instead).
        assert!(!should_show_neutral_cover(false, "connecting", false));
        assert!(!should_show_neutral_cover(false, "no-signal", true));
    }

    #[test]
    fn error_state_is_never_a_neutral_cover_regardless_of_frames() {
        // A genuine failure/disconnect is the RED ERROR overlay, NOT the neutral
        // cover — independent of frames_live. This pins that the #500 frames
        // gate never suppresses the error overlay (a failed source has no
        // frames anyway, so frames_live is false in practice — but assert both).
        assert_eq!(ndi_overlay_kind("failed"), NdiOverlayKind::Error);
        assert_eq!(ndi_overlay_kind("disconnected"), NdiOverlayKind::Error);
        assert!(!should_show_neutral_cover(true, "failed", false));
        assert!(!should_show_neutral_cover(true, "failed", true));
        assert!(!should_show_neutral_cover(true, "disconnected", false));
        // `connected` (a stream flowing → None kind) is also never the cover.
        assert!(!should_show_neutral_cover(true, "connected", false));
    }

    // ── #448: off/silent source is a NEUTRAL state, not a red error ──────────
    //
    // Live on prod 2026-06-22 (Resolume 'cg' OFF), the ndi-fullscreen layout
    // painted a configured-but-silent source as a RED "NDI pipeline failed:
    // … broadcaster is silent" overlay. A source that is simply off is an
    // EXPECTED state — it must render as a calm gray "waiting" placeholder.

    #[test]
    fn no_signal_status_is_neutral_not_error() {
        // The off/silent source publishes `no-signal` (#448). It MUST be a
        // NEUTRAL placeholder — never the red error overlay.
        assert_eq!(ndi_overlay_kind("no-signal"), NdiOverlayKind::Neutral);
    }

    #[test]
    fn no_signal_status_text_is_waiting_placeholder() {
        assert_eq!(ndi_status_text("no-signal"), "Waiting for video source…");
    }

    #[test]
    fn connecting_status_is_neutral() {
        // The pipeline-starting state is not an error either → neutral.
        assert_eq!(ndi_overlay_kind("connecting"), NdiOverlayKind::Neutral);
    }

    #[test]
    fn genuine_failure_status_is_error() {
        // A REAL pipeline failure (e.g. encoder build failure) stays RED.
        assert_eq!(
            ndi_overlay_kind("failed: no hardware H264 encoder registered"),
            NdiOverlayKind::Error,
        );
        assert_eq!(ndi_overlay_kind("failed"), NdiOverlayKind::Error);
    }

    #[test]
    fn disconnected_status_is_error() {
        // A previously-good signal that was lost is a genuine problem → RED.
        assert_eq!(ndi_overlay_kind("disconnected"), NdiOverlayKind::Error);
    }

    #[test]
    fn flowing_or_idle_status_has_no_overlay() {
        assert_eq!(ndi_overlay_kind("connected"), NdiOverlayKind::None);
        assert_eq!(ndi_overlay_kind(""), NdiOverlayKind::None);
        assert_eq!(ndi_overlay_kind("streaming"), NdiOverlayKind::None);
        assert_eq!(ndi_overlay_kind("totally-new-state"), NdiOverlayKind::None);
    }

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
