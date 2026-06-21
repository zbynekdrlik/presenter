//! Stream-profile fallback mode + the bounded once-per-pageload profile switch.
//!
//! The stage video can request one of two stream "profiles" via the WHEP POST
//! URL: `default` (no `?profile=`) or `compat` (`?profile=compat`). The server
//! now serves ONE 720p H264 stream regardless (see `StreamProfile::from_query`),
//! so the compat flip is a no-op server-side — its only effect is that changing
//! the URL forces a reconnect that re-establishes a stuck session.
//!
//! This module owns the in-memory + localStorage profile-mode state, the
//! at-most-once-per-pageload switch (`switch_profile_mode_once`), the
//! proven-mode persistence (`persist_proven_profile_mode`), and the
//! frame-based profile-fallback check (`maybe_profile_fallback`). Split out of
//! `ndi_watchdog.rs` to keep that file under the size cap (#418).

use std::cell::Cell;
use std::rc::Rc;

use leptos::web_sys::{RtcIceConnectionState, RtcPeerConnection};

use super::ndi_frame_stats::FrameStats;
use super::ndi_watchdog::Watchdog;

/// localStorage key for the stream-profile fallback mode. Absent or
/// `"default"` = WHEP POST without a profile query; `"compat"` = the WHEP
/// POST URL carries `?profile=compat`.
///
/// NOTE: the server now serves ONE 720p H264 stream regardless of
/// `?profile=` (see `StreamProfile::from_query`), so the compat flip is a
/// no-op server-side — it does NOT switch to any 640×480 / VP8 branch (that
/// branch never shipped). The flip is retained ONLY because changing the URL
/// forces a reconnect, and that reconnect re-establishes a stuck session.
///
/// The KEY deliberately keeps its historical name ("ndiCodecMode") so
/// deployed TVs don't grow a second orphaned entry; the retired "vp8" value
/// some of them still store parses as default mode and self-heals through
/// the normal fallback → proven-mode flow.
const PROFILE_MODE_KEY: &str = "ndiCodecMode";

/// Access the window's localStorage (None when unavailable, e.g. sandboxed).
pub(crate) fn local_storage() -> Option<leptos::web_sys::Storage> {
    leptos::web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

thread_local! {
    /// In-memory profile mode for THIS page load, seeded from localStorage on
    /// first use. `None` = not yet seeded. Connect attempts read this, NOT
    /// localStorage directly: a fallback switch flips it in memory only —
    /// the sticky localStorage value is written exclusively by
    /// `persist_proven_profile_mode` once a mode actually decodes (so the
    /// persisted value is always a PROVEN one, never a guess mid-ping-pong).
    static PROFILE_MODE_COMPAT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// At most ONE profile switch per page load. One Vestel TV alternated
    /// modes repeatedly when its wall-clock-based decode check misfired;
    /// bounding the switch to once-per-pageload kills the ping-pong.
    static PROFILE_SWITCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when the stream-profile fallback mode is "compat". Any other value
/// (including absent and the retired "vp8") means the default 720p stream.
pub(crate) fn profile_mode_is_compat() -> bool {
    PROFILE_MODE_COMPAT.with(|cell| {
        if let Some(v) = cell.get() {
            return v;
        }
        let stored = local_storage()
            .and_then(|s| s.get_item(PROFILE_MODE_KEY).ok().flatten())
            .as_deref()
            == Some("compat");
        cell.set(Some(stored));
        stored
    })
}

/// Flip the in-memory profile mode (default → compat or compat → default)
/// and return the new mode name — at most ONCE per page load. Returns
/// `None` when the one-shot switch was already spent (no further toggling
/// until reload). Deliberately does NOT touch localStorage: only a mode
/// that goes on to present `PROVEN_MODE_FRAMES` frames within
/// `PROVEN_MODE_WINDOW_MS` of the first frame gets persisted (see
/// `record_presented_frame`).
fn switch_profile_mode_once() -> Option<&'static str> {
    if PROFILE_SWITCHED.with(|c| c.replace(true)) {
        return None;
    }
    let new_compat = !profile_mode_is_compat();
    PROFILE_MODE_COMPAT.with(|c| c.set(Some(new_compat)));
    Some(profile_mode_name(new_compat))
}

/// The wire/storage name of a profile mode: "compat" or "default".
pub(crate) fn profile_mode_name(compat: bool) -> &'static str {
    if compat {
        "compat"
    } else {
        "default"
    }
}

/// Persist the CURRENT profile mode to localStorage. Called once a session
/// presents `PROVEN_MODE_FRAMES` frames WITHIN `PROVEN_MODE_WINDOW_MS` of
/// the first presented frame — the mode demonstrably decodes AT A USABLE
/// RATE on this display, so it is safe to make sticky across reloads.
///
/// The rate gate is load-bearing: 100 frames at <10fps must NOT prove a
/// mode. A Vestel TV limping along at 0.3-1.7 fps (the VP8-era crawl)
/// still reaches 100 presented frames eventually (~100s at 1fps), and
/// persisting then locked the broken mode in forever. Callers
/// (`record_presented_frame`) enforce the window; an unproven mode is
/// simply left unpersisted — the existing stored value is never cleared —
/// so the next page load retries.
pub(crate) fn persist_proven_profile_mode() {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(
            PROFILE_MODE_KEY,
            profile_mode_name(profile_mode_is_compat()),
        );
    }
}

/// Profile-fallback check (frame-based): a session that is ICE-connected with
/// ZERO presented frames `NO_DECODE_FALLBACK_MS` after connect has a dead
/// decoder (the broken Vestel H264 OMX symptom: connected, RTP flowing,
/// nothing presented). Switch the profile mode — bounded to ONCE per page
/// load, killing the mode ping-pong — and fire `on_failure` so the
/// reconnect requests the other profile (compat mode adds
/// `?profile=compat` to the WHEP POST URL — see `ndi_video::whep_url`).
pub(crate) fn maybe_profile_fallback<F: Fn() + 'static>(
    now: f64,
    stats: &FrameStats,
    pc: &RtcPeerConnection,
    active: &Rc<Cell<bool>>,
    on_failure: &Rc<F>,
) {
    if now - stats.started_at.get() < Watchdog::NO_DECODE_FALLBACK_MS {
        return;
    }
    // Only a CONNECTED session gets a profile verdict: pre-connect states mean
    // media never had a chance (ICE problems are the ICE listener's job).
    if !matches!(
        pc.ice_connection_state(),
        RtcIceConnectionState::Connected | RtcIceConnectionState::Completed
    ) {
        return;
    }
    let Some(new_mode) = switch_profile_mode_once() else {
        // One-shot spent this page load — keep waiting, never ping-pong.
        return;
    };
    leptos::logging::warn!(
        "profile fallback: 0 frames presented {}s after connect — switching to profile mode {new_mode} (once per page load)",
        Watchdog::NO_DECODE_FALLBACK_MS / 1000.0
    );
    active.set(false);
    (on_failure)();
}
