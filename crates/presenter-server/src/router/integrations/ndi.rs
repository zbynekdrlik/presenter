use axum::http::StatusCode;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiSourceDto {
    name: String,
}

#[instrument(skip_all)]
pub(crate) async fn discover_ndi_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<NdiSourceDto>>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sources = manager.discover_sources(0)?;
    Ok(Json(
        sources
            .into_iter()
            .map(|s| NdiSourceDto { name: s.name })
            .collect(),
    ))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// `GET /ndi/snapshot/:source_id` — diagnostic route exposing the live
/// pipeline state for a single NDI source.
///
/// Returns JSON (camelCase) with `encoderCount`, `consumerCount`, and a
/// per-session `sessions` array. Used by the Playwright fanout E2E test
/// to assert `encoderCount=2` (one encoder per PROFILE — 720p default +
/// 640×480 compat — never per consumer) + `consumerCount=2` when two
/// browser tabs are connected to the same NDI source, and as an operator/
/// incident-debugging tool for checking pipeline health without tailing
/// logs.
///
/// 404 — source is not currently active (no pipeline exists for this id).
/// 503 — NDI SDK not available on this host.
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn ndi_snapshot(
    axum::extract::Path(source_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let snap = manager
        .pipeline_snapshot(&source_id)
        .await
        .ok_or_else(|| AppError::not_found("NDI source not active"))?;
    Ok(Json(
        serde_json::to_value(snap).expect("PipelineSnapshot serializes"),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiClientStatsBeacon {
    pub source_id: String,
    /// Persistent random per-display id (localStorage `ndiDisplayId`) — the
    /// attribution key that makes per-TV health traceable across sessions.
    pub display_id: Option<String>,
    /// Negotiated video codec mimeType from getStats (now "video/H264" for
    /// every consumer — both stream profiles are H264).
    pub codec: Option<String>,
    /// Stream profile the display requested ("default"/"compat") — the
    /// field that attributes weak-TV health to the 640×480 compat branch,
    /// since `codec` no longer distinguishes the branches.
    pub profile: Option<String>,
    /// Physical screen size as "WxH" — tells TV models apart in the logs.
    pub screen: Option<String>,
    pub frames_decoded: Option<f64>,
    pub fps: Option<f64>,
    pub jitter_buffer_ms: Option<f64>,
    pub freeze_count: Option<f64>,
    pub frames_dropped: Option<f64>,
    // DIAG (temporary): raw cumulative inbound-rtp counters to locate the
    // synchronized ~20s hitch mechanism — deltas reveal delivery (RTP arriving?)
    // vs decode vs jitter-buffer-readjust.
    pub packets_received: Option<f64>,
    pub packets_lost: Option<f64>,
    pub frames_received: Option<f64>,
    pub jitter_buffer_delay: Option<f64>,
    pub jitter_buffer_emitted: Option<f64>,
    /// Largest inter-present gap (ms) the display observed this beacon
    /// interval — the RENDER-side metric the decode-side fields above are
    /// blind to. A frame can decode on time yet be PRESENTED late (WebView
    /// compositor / main-thread hitch); this captures the user-visible
    /// "lag every ~20s" that `framesDecoded`/`fps` cannot. A high value here
    /// with healthy `fps` means the stall is presentation-side, not decode.
    pub max_present_gap_ms: Option<f64>,
    /// Count of inter-present gaps > 100ms this beacon interval (perceptible
    /// hitches). 0 = smooth presentation; rising = repeated render-side stalls.
    pub present_gaps_over100: Option<f64>,
    /// Render-side fps for this interval (frames PRESENTED / interval-seconds,
    /// from the rVFC callback rate) — distinct from `fps` (decode-side
    /// getStats). presentedFps below decode fps points at the compositor.
    pub presented_fps: Option<f64>,
    /// `true` when the beacon comes from the lite plain-JS stage page
    /// (`/stage/lite`, weak-TV experiment #379) instead of the WASM stage —
    /// lets the logs attribute decode health to the page variant.
    pub lite: Option<bool>,
}

/// Stage displays POST a compact getStats summary every 15s. Log-only (MVP):
/// journald keeps the history, so "the stage was laggy at 19:40" is
/// answerable from data (fps, jitter buffer, freezes per display).
#[instrument(skip_all)]
pub(crate) async fn ndi_client_stats(Json(beacon): Json<NdiClientStatsBeacon>) -> StatusCode {
    tracing::info!(
        display_id = beacon.display_id.as_deref(),
        source_id = %beacon.source_id,
        codec = beacon.codec.as_deref(),
        profile = beacon.profile.as_deref(),
        screen = beacon.screen.as_deref(),
        frames_decoded = beacon.frames_decoded,
        fps = beacon.fps,
        jitter_buffer_ms = beacon.jitter_buffer_ms,
        freeze_count = beacon.freeze_count,
        frames_dropped = beacon.frames_dropped,
        max_present_gap_ms = beacon.max_present_gap_ms,
        present_gaps_over100 = beacon.present_gaps_over100,
        presented_fps = beacon.presented_fps,
        packets_received = beacon.packets_received,
        packets_lost = beacon.packets_lost,
        frames_received = beacon.frames_received,
        jitter_buffer_delay = beacon.jitter_buffer_delay,
        jitter_buffer_emitted = beacon.jitter_buffer_emitted,
        lite = beacon.lite,
        "NDI stage-display client stats beacon"
    );
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    /// Build a fresh in-memory AppState that may or may not have a real NDI
    /// manager attached depending on whether libndi is loadable on the host.
    async fn fresh_state() -> AppState {
        AppState::in_memory().await.expect("in-memory AppState")
    }

    #[test]
    fn client_stats_beacon_parses_lite_field() {
        let beacon: NdiClientStatsBeacon =
            serde_json::from_str(r#"{"sourceId":"src-1","profile":"compat","lite":true}"#)
                .expect("beacon JSON with lite field parses");
        assert_eq!(beacon.lite, Some(true));
        // WASM-stage beacons don't send the field — it must stay optional.
        let wasm_beacon: NdiClientStatsBeacon =
            serde_json::from_str(r#"{"sourceId":"src-1"}"#).expect("beacon without lite parses");
        assert_eq!(wasm_beacon.lite, None);
    }

    #[test]
    fn client_stats_beacon_parses_present_gap_fields() {
        // A full beacon as the stage client now sends it: decode-side getStats
        // fields PLUS the render-side present-gap metrics (camelCase wire keys).
        let beacon: NdiClientStatsBeacon = serde_json::from_str(
            r#"{
                "sourceId":"src-1",
                "framesDecoded":900.0,
                "fps":30.0,
                "maxPresentGapMs":312.5,
                "presentGapsOver100":4.0,
                "presentedFps":29.7
            }"#,
        )
        .expect("beacon JSON with present-gap fields parses");
        assert_eq!(beacon.max_present_gap_ms, Some(312.5));
        assert_eq!(beacon.present_gaps_over100, Some(4.0));
        assert_eq!(beacon.presented_fps, Some(29.7));

        // Older clients / the proxy path may omit them — they must stay
        // optional so a beacon without present-gap data still parses.
        let no_gap: NdiClientStatsBeacon = serde_json::from_str(r#"{"sourceId":"src-1"}"#)
            .expect("beacon without present-gap fields parses");
        assert_eq!(no_gap.max_present_gap_ms, None);
        assert_eq!(no_gap.present_gaps_over100, None);
        assert_eq!(no_gap.presented_fps, None);
    }

    #[tokio::test]
    async fn ndi_snapshot_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let result = ndi_snapshot(
            axum::extract::Path("00000000-0000-0000-0000-000000000000".to_string()),
            State(state),
        )
        .await;
        assert!(result.is_err(), "expected Err for unknown source");
        let resp = result.unwrap_err().into_response();
        assert!(
            matches!(
                resp.status(),
                StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE,
            ),
            "expected 404 or 503, got {}",
            resp.status(),
        );
    }
}
