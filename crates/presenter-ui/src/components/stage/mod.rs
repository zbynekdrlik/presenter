pub mod api_stage;
pub mod bible_layout;
pub mod bible_overlay;
pub mod camera_crew;
pub mod ndi_fullscreen;
pub mod ndi_video;
pub mod preach_layout;
pub mod status_bar;
pub mod timer_layout;
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
