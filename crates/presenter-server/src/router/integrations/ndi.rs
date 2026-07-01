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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiTimeDto {
    /// The server's GStreamer PIPELINE-CLOCK time, in milliseconds — NOT
    /// `SystemTime::now()` wall-clock. See the `ndi_time` doc below.
    server_time_ms: f64,
}

/// `GET /ndi/time` (#510, T3 of the NDI true-latency rework) — returns the
/// server's GStreamer **pipeline-clock** time, the EXACT clock domain the
/// RTCP Sender Reports encode (`rtpbin ntp-time-source=clock-time`, set in
/// `consumers.rs`). The browser's clock-offset estimator
/// (`presenter-ui`'s `ndi_clock_offset` module) does a periodic NTP-style
/// round trip against this route to compute
/// `offset_browser→serverPipelineClock`: since both this endpoint and the SR
/// live in the SAME clock domain, a later metric (#512, T4) can convert
/// `estimatedPlayoutTimestamp` into that domain without needing an absolute
/// wall-clock sync (dantesync) on the critical path. See
/// `docs/superpowers/specs/2026-06-30-ndi-true-latency-design.md` §3.
///
/// Deliberately does NOT require an active NDI source/pipeline —
/// `gst::SystemClock::obtain()` is a process-wide singleton valid as soon as
/// `gstreamer::init()` has run once (idempotent, lazily triggered here on the
/// first call if no NDI pipeline has started yet).
#[instrument(skip_all)]
pub(crate) async fn ndi_time() -> Result<Json<NdiTimeDto>, AppError> {
    let server_time_ms = presenter_ndi::pipeline_clock_now_ms()
        .map_err(|e| AppError::service_unavailable(format!("pipeline clock unavailable: {e}")))?;
    Ok(Json(NdiTimeDto { server_time_ms }))
}

/// `GET /ndi/ice-servers` (#502) — returns the browser-ready WebRTC ICE
/// servers (Cloudflare Realtime TURN). The browser sets these on its
/// `RTCPeerConnection` so a relay candidate exists when the direct LAN path is
/// unreachable (Tailscale subnet route / remote client). Returns `[]` when TURN
/// is unconfigured — the browser then uses LAN host candidates only, exactly as
/// before. Credentials are minted short-lived server-side; the long-lived TURN
/// key never reaches the browser.
#[instrument(skip_all)]
pub(crate) async fn ndi_ice_servers(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.turn().ice_servers().await)
}

/// `GET /ndi/snapshot/:source_id` — diagnostic route exposing the live
/// pipeline state for a single NDI source.
///
/// Returns JSON (camelCase) with `encoderCount`, `consumerCount`, and a
/// per-session `sessions` array. Used by the Playwright fanout E2E test
/// to assert `encoderCount=1` (one shared encoder — the single 720p H264
/// stream every consumer shares, never one per consumer) + `consumerCount=2`
/// when two browser tabs are connected to the same NDI source, and as an
/// operator/incident-debugging tool for checking pipeline health without
/// tailing logs.
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
    /// Negotiated video codec mimeType from getStats ("video/H264" for every
    /// consumer — there is a single shared 720p H264 stream).
    pub codec: Option<String>,
    /// Stream profile the display requested ("default"/"compat"). The server
    /// always serves the single shared 720p H264 stream regardless of this
    /// value (see `StreamProfile::from_query`); it is logged only to record
    /// which mode a display's WASM watchdog was in when it reported health.
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
    pub jitter_buffer_target_delay: Option<f64>,
    pub frames_rendered: Option<f64>,
    pub pause_count: Option<f64>,
    pub total_pauses_duration: Option<f64>,
    pub total_freezes_duration: Option<f64>,
    pub key_frames_decoded: Option<f64>,
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
    /// Legacy beacon flag from the retired plain-JS "lite" stage experiment
    /// (#379). The standard WASM stage never sets it, so it is always absent
    /// (`None`); retained for backward-compatible beacon parsing.
    pub lite: Option<bool>,
    /// (#509 T0 probe) `estimatedPlayoutTimestamp` from the SAME inbound-rtp
    /// getStats snapshot — the field T4's true server→display metric would read.
    /// `null` = the WebView doesn't expose it (metric permanently n/a there);
    /// `0.0` = present but pre-first-SR (must not be read as a real timestamp).
    /// Logged raw every beacon so advancement across consecutive beacons is
    /// readable server-side; also classified via `classify_playout`.
    pub estimated_playout_timestamp: Option<f64>,
    /// (#509 T0 probe) The inbound-rtp stat's OWN Unix-epoch `.timestamp` (ms)
    /// from the same snapshot — the epoch reference `estimatedPlayoutTimestamp`
    /// is checked against to prove it shares the Unix-epoch domain.
    pub report_timestamp: Option<f64>,
    /// (#509 T0 probe) Full browser `navigator.userAgent` — records the exact
    /// WebView/Chrome version per real stage TV (SD1, Hyundai), which decides
    /// whether the playout field is available on the devices that matter.
    pub user_agent: Option<String>,
}

/// Classification of a display's `estimatedPlayoutTimestamp` for the #509 (T0)
/// device-capability probe: which real TVs expose a trustworthy value — the
/// field T4's true server→display metric would be defined on. `report_timestamp`
/// is the SAME inbound-rtp getStats snapshot's own Unix-epoch `.timestamp` (ms),
/// the epoch reference the playout value is compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayoutClass {
    /// Field absent (the WebView doesn't expose `estimatedPlayoutTimestamp`) →
    /// the metric would be permanently `n/a` on this device (repeats #479).
    Absent,
    /// Present but non-finite (NaN / ±Inf) → unusable.
    NonFinite,
    /// Present but a literal `0` — pre-first-RTCP-SR. `Reflect::get(...).as_f64()`
    /// returns `Some(0.0)` for a literal 0, so a naive check would read it as a
    /// real timestamp and surface a bogus huge latency; classified separately.
    Zero,
    /// Finite, non-zero, and in the SAME Unix-epoch domain as the report's
    /// `.timestamp` → trustworthy (T4's metric can be built on it here).
    ValidUnixEpoch,
    /// Finite, non-zero, but a different domain than the report's `.timestamp`
    /// (or no report to compare against) → present but domain-suspect.
    ValidOtherDomain,
}

impl PlayoutClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PlayoutClass::Absent => "absent",
            PlayoutClass::NonFinite => "nonfinite",
            PlayoutClass::Zero => "zero",
            PlayoutClass::ValidUnixEpoch => "valid-unix-epoch",
            PlayoutClass::ValidOtherDomain => "valid-other-domain",
        }
    }
}

/// Max |playout − report| (ms) for the two to count as the same epoch domain.
/// `estimatedPlayoutTimestamp` leads the report time only by the buffer depth
/// (tens of ms); an epoch mismatch (NTP-1900 vs Unix-1970, or the TV's own
/// unsynced clock) is off by years, so a generous 5-minute window cleanly
/// separates "same domain" from "wrong domain".
const PLAYOUT_EPOCH_MATCH_WINDOW_MS: f64 = 300_000.0;

/// Classify a display's `estimatedPlayoutTimestamp` for the T0 probe. Pure so it
/// is unit-tested directly (RED→GREEN) and reused by T4's trust predicate.
pub(crate) fn classify_playout(
    estimated_playout_ms: Option<f64>,
    report_timestamp_ms: Option<f64>,
) -> PlayoutClass {
    let Some(v) = estimated_playout_ms else {
        return PlayoutClass::Absent;
    };
    if !v.is_finite() {
        return PlayoutClass::NonFinite;
    }
    if v == 0.0 {
        return PlayoutClass::Zero;
    }
    match report_timestamp_ms {
        Some(r) if r.is_finite() && (v - r).abs() <= PLAYOUT_EPOCH_MATCH_WINDOW_MS => {
            PlayoutClass::ValidUnixEpoch
        }
        _ => PlayoutClass::ValidOtherDomain,
    }
}

/// Stage displays POST a compact getStats summary every 15s. Log-only (MVP):
/// journald keeps the history, so "the stage was laggy at 19:40" is
/// answerable from data (fps, jitter buffer, freezes per display).
#[instrument(skip_all)]
pub(crate) async fn ndi_client_stats(Json(beacon): Json<NdiClientStatsBeacon>) -> StatusCode {
    let playout_class =
        classify_playout(beacon.estimated_playout_timestamp, beacon.report_timestamp).as_str();
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
        jitter_buffer_target_delay = beacon.jitter_buffer_target_delay,
        frames_rendered = beacon.frames_rendered,
        pause_count = beacon.pause_count,
        total_pauses_duration = beacon.total_pauses_duration,
        total_freezes_duration = beacon.total_freezes_duration,
        key_frames_decoded = beacon.key_frames_decoded,
        lite = beacon.lite,
        estimated_playout_timestamp = beacon.estimated_playout_timestamp,
        report_timestamp = beacon.report_timestamp,
        playout_class,
        user_agent = beacon.user_agent.as_deref(),
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

    #[test]
    fn client_stats_beacon_parses_t0_probe_fields() {
        // #509 T0: the device-capability probe fields the stage now sends.
        let beacon: NdiClientStatsBeacon = serde_json::from_str(
            r#"{
                "sourceId":"src-1",
                "estimatedPlayoutTimestamp":1750000000080.0,
                "reportTimestamp":1750000000000.0,
                "userAgent":"Mozilla/5.0 (Linux; Android 11; SmartTV) Chrome/90"
            }"#,
        )
        .expect("beacon JSON with T0 probe fields parses");
        assert_eq!(
            beacon.estimated_playout_timestamp,
            Some(1_750_000_000_080.0)
        );
        assert_eq!(beacon.report_timestamp, Some(1_750_000_000_000.0));
        assert_eq!(
            beacon.user_agent.as_deref(),
            Some("Mozilla/5.0 (Linux; Android 11; SmartTV) Chrome/90")
        );
        // A literal 0 playout (pre-first-SR) must round-trip as Some(0.0), not None.
        let zero: NdiClientStatsBeacon =
            serde_json::from_str(r#"{"sourceId":"src-1","estimatedPlayoutTimestamp":0.0}"#)
                .expect("beacon with zero playout parses");
        assert_eq!(zero.estimated_playout_timestamp, Some(0.0));
        // Older clients omit them → all optional.
        let bare: NdiClientStatsBeacon =
            serde_json::from_str(r#"{"sourceId":"src-1"}"#).expect("bare beacon parses");
        assert_eq!(bare.estimated_playout_timestamp, None);
        assert_eq!(bare.report_timestamp, None);
        assert_eq!(bare.user_agent, None);
    }

    #[test]
    fn classify_playout_absent_when_field_missing() {
        // The WebView doesn't expose estimatedPlayoutTimestamp → permanently n/a.
        assert_eq!(classify_playout(None, Some(1.75e12)), PlayoutClass::Absent);
    }

    #[test]
    fn classify_playout_zero_is_not_a_real_timestamp() {
        // The Some(0.0) gotcha: pre-first-SR literal 0 must NOT read as valid.
        assert_eq!(
            classify_playout(Some(0.0), Some(1.75e12)),
            PlayoutClass::Zero
        );
    }

    #[test]
    fn classify_playout_nonfinite_is_unusable() {
        assert_eq!(
            classify_playout(Some(f64::NAN), Some(1.75e12)),
            PlayoutClass::NonFinite
        );
    }

    #[test]
    fn classify_playout_valid_when_in_report_epoch() {
        // Playout leads the report by the buffer depth (tens of ms) → same domain.
        let report = 1_750_000_000_000.0;
        assert_eq!(
            classify_playout(Some(report + 80.0), Some(report)),
            PlayoutClass::ValidUnixEpoch
        );
    }

    #[test]
    fn classify_playout_other_domain_when_far_from_report_or_no_report() {
        let report = 1_750_000_000_000.0;
        // A tiny monotonic-ms value is finite+nonzero but a different domain.
        assert_eq!(
            classify_playout(Some(1000.0), Some(report)),
            PlayoutClass::ValidOtherDomain
        );
        // No report to compare against → domain unconfirmable.
        assert_eq!(
            classify_playout(Some(report), None),
            PlayoutClass::ValidOtherDomain
        );
    }

    #[tokio::test]
    async fn ndi_time_returns_finite_monotonic_pipeline_clock() {
        // #510 T3: the offset estimator's NTP-style round trip depends on
        // /ndi/time returning a sane, ADVANCING value — a stuck or garbage
        // clock would silently poison every downstream offset sample.
        let Json(first) = ndi_time().await.expect("first /ndi/time call");
        assert!(
            first.server_time_ms.is_finite() && first.server_time_ms >= 0.0,
            "expected a finite, non-negative pipeline-clock ms value, got {}",
            first.server_time_ms
        );
        let Json(second) = ndi_time().await.expect("second /ndi/time call");
        assert!(
            second.server_time_ms >= first.server_time_ms,
            "pipeline clock must never go backwards: first={} second={}",
            first.server_time_ms,
            second.server_time_ms
        );
    }

    #[tokio::test]
    async fn ice_servers_empty_when_turn_unconfigured() {
        // #502: with no PRESENTER_TURN_KEY_* env (the test/CI default), the
        // endpoint returns an empty array so the browser uses LAN host
        // candidates only — exactly today's behavior.
        let state = fresh_state().await;
        let Json(value) = ndi_ice_servers(State(state)).await;
        assert_eq!(value, serde_json::json!([]));
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
