//! Client-reported per-session frame stats stored on a WHEP session (#768 D6).
//!
//! The stage measures how the video actually PRESENTS (presented fps, largest
//! present gap, jitter-buffer depth, frames decoded, whether frames are live)
//! only client-side. During the 2026-09-13 incident none of that left the box
//! per WHEP session, so a stuttering TV was invisible without physical
//! presence. The stage now POSTs a compact sample every ~5s to
//! `POST /ndi/sessions/{session_id}/client-stats`; the server stores the
//! latest sample (with an arrival `Instant`) on the matching `WhepSession`
//! and exposes it in `GET /ndi/snapshot/{id}` under `sessions[].client`,
//! including an `ageMs` staleness field.

use std::time::Instant;

use super::NdiPipeline;

impl NdiPipeline {
    /// Store a client-reported frame-stats sample on the matching WHEP session
    /// (#768 D6). Returns `true` when the session exists (sample stored),
    /// `false` when this pipeline has no such session (the manager then tries
    /// the next pipeline / maps to 404).
    ///
    /// Locks only this pipeline's `sessions` map (a per-pipeline tokio Mutex,
    /// NOT the manager's `active` map) and the trivial per-session
    /// `client_stats` std Mutex — never held across an await.
    pub(crate) async fn record_client_stats(
        &self,
        session_id: &str,
        sample: ClientStatsSample,
    ) -> bool {
        let sessions = self.sessions.lock().await;
        match sessions.get(session_id) {
            Some(session) => {
                *session
                    .client_stats
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(sample);
                true
            }
            None => false,
        }
    }
}

/// One client-reported frame-stats sample, stamped with its server arrival
/// time. Stored (latest wins) on a `WhepSession`; `Copy` so the snapshot
/// reader can lift it out from under the lock without cloning strings.
#[derive(Debug, Clone, Copy)]
pub struct ClientStatsSample {
    /// Render-side frames/s the display PRESENTED over its last report window
    /// (rVFC callback rate) — distinct from the server's `pushedFps`.
    pub presented_fps: f64,
    /// Largest inter-present gap (ms) the display observed — the render-side
    /// stall the server's decode-blind counters cannot see.
    pub max_present_gap_ms: f64,
    /// Jitter-buffer depth (ms) from the display's getStats, when available.
    pub jitter_buffer_ms: Option<f64>,
    /// Cumulative frames the display's decoder produced (getStats).
    pub frames_decoded: f64,
    /// Whether the display currently considers frames live (its shared
    /// `ndi_frames_live` gate).
    pub frames_live: bool,
    /// Server-side arrival time — the `ageMs` reference in the snapshot.
    pub received_at: Instant,
}

impl ClientStatsSample {
    /// Render the stored sample for the diagnostic snapshot at `now`, computing
    /// `ageMs` from the arrival time. `saturating_duration_since` keeps `ageMs`
    /// at 0 for an out-of-order clock rather than panicking.
    pub fn to_snapshot(self, now: Instant) -> ClientStatsSnapshot {
        ClientStatsSnapshot {
            presented_fps: self.presented_fps,
            max_present_gap_ms: self.max_present_gap_ms,
            jitter_buffer_ms: self.jitter_buffer_ms,
            frames_decoded: self.frames_decoded,
            frames_live: self.frames_live,
            age_ms: now.saturating_duration_since(self.received_at).as_millis() as u64,
        }
    }
}

/// Serialized client sample for `GET /ndi/snapshot/{id}` `sessions[].client`.
/// `ageMs` is how long ago the sample arrived (staleness) — a large value
/// means the display stopped reporting.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatsSnapshot {
    pub presented_fps: f64,
    pub max_present_gap_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_buffer_ms: Option<f64>,
    pub frames_decoded: f64,
    pub frames_live: bool,
    pub age_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample(received_at: Instant) -> ClientStatsSample {
        ClientStatsSample {
            presented_fps: 30.0,
            max_present_gap_ms: 40.0,
            jitter_buffer_ms: Some(12.0),
            frames_decoded: 900.0,
            frames_live: true,
            received_at,
        }
    }

    #[test]
    fn to_snapshot_carries_values_and_computes_age() {
        let base = Instant::now();
        let snap = sample(base).to_snapshot(base + Duration::from_millis(1_500));
        assert_eq!(snap.presented_fps, 30.0);
        assert_eq!(snap.max_present_gap_ms, 40.0);
        assert_eq!(snap.jitter_buffer_ms, Some(12.0));
        assert_eq!(snap.frames_decoded, 900.0);
        assert!(snap.frames_live);
        assert!(
            (1_490..=1_510).contains(&snap.age_ms),
            "ageMs must reflect ~1500ms since arrival, got {}",
            snap.age_ms
        );
    }

    #[test]
    fn age_grows_with_a_later_read() {
        let base = Instant::now();
        let s = sample(base);
        let early = s.to_snapshot(base + Duration::from_millis(500)).age_ms;
        let late = s.to_snapshot(base + Duration::from_millis(5_000)).age_ms;
        assert!(late > early, "a later read must report a larger ageMs");
    }

    #[test]
    fn out_of_order_clock_saturates_age_to_zero() {
        let base = Instant::now();
        // A read "before" arrival must not panic; ageMs saturates to 0.
        let snap = sample(base + Duration::from_secs(1)).to_snapshot(base);
        assert_eq!(snap.age_ms, 0);
    }

    #[test]
    fn absent_jitter_buffer_is_omitted_from_json() {
        let base = Instant::now();
        let mut s = sample(base);
        s.jitter_buffer_ms = None;
        let json = serde_json::to_string(&s.to_snapshot(base)).expect("serialize");
        assert!(
            !json.contains("jitterBufferMs"),
            "None jitterBufferMs must be skipped, got: {json}"
        );
        assert!(json.contains("presentedFps"), "camelCase keys, got: {json}");
        assert!(json.contains("ageMs"), "ageMs present, got: {json}");
    }
}
