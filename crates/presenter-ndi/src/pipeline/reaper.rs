//! WHEP zombie-session reaper (#388) + the soft consumer-cap gate.
//!
//! Stage-display TVs reboot/reload WITHOUT sending a WHEP DELETE, so their
//! server-side `WhepSession` strands forever. There are TWO reapers:
//!
//! 1. **RTCP-liveness reap** (`reap_stale_sessions`) — the LOAD-BEARING fix.
//!    gst webrtcbin NEVER flips `connection-state` for a peer that vanished
//!    without closing (no server-side liveness timeout), so a stranded session
//!    sits at `Connected` forever. The only reliable signal is the peer's RTCP
//!    receiver reports: a live consumer sends them continuously, so the
//!    per-session RR fingerprint keeps CHANGING; a vanished peer stops, so the
//!    fingerprint freezes and webrtcbin drops the `remote-inbound-rtp` stats. A
//!    session whose fingerprint has not changed for > `stale_after` is reaped.
//!
//! 2. **Connection-state reap** (`reap_dead_sessions`) — the cheap fast path.
//!    Removes sessions whose last-seen `connection-state` is
//!    Disconnected/Failed/Closed (the rare cases webrtcbin DOES report, e.g. an
//!    explicit ICE failure). Inert for the silent-vanish case above — which is
//!    exactly why reaper v2 exists.
//!
//! Both run on the WHEP join path (`reap_and_check_cap`, stale FIRST) so a map
//! full of zombies self-heals when a new display joins, and a 30s periodic
//! supervisor sweep runs the stale reap so slots free even with no new join.

use gstreamer as gst;
use gstreamer::prelude::*;

use super::{AddConsumerError, NdiPipeline, MAX_CONSUMERS_PER_SOURCE};
use crate::whep_session::{LivenessState, WhepConnectionState, WhepSession};

impl NdiPipeline {
    /// Enforce the soft consumer cap BEFORE allocating any GStreamer
    /// resources.
    ///
    /// Soft cap: this check + the session insert in `add_consumer` are NOT
    /// atomic. Two concurrent add_consumer calls at count=MAX-1 can both pass
    /// and momentarily reach count=MAX+1. Acceptable per spec ("soft cap").
    async fn check_consumer_cap(&self) -> Result<(), AddConsumerError> {
        let sessions = self.sessions.lock().await;
        if sessions.len() >= MAX_CONSUMERS_PER_SOURCE {
            // Structured cap-rejection log (#388 observability): the WHEP POST
            // is about to 503. Pre-#388 the only WHEP log line was "POST → 201"
            // on SUCCESS, so 503-rejected joins were INVISIBLE in the ledgers —
            // that blind spot masked the 2026-06-12 cap exhaustion as a "client
            // mount race". `reason="cap"` makes every rejection greppable.
            tracing::warn!(
                reason = "cap",
                current_count = sessions.len(),
                max = MAX_CONSUMERS_PER_SOURCE,
                "WHEP consumer rejected — per-source cap reached (503)"
            );
            return Err(AddConsumerError::CapReached {
                max: MAX_CONSUMERS_PER_SOURCE,
            });
        }
        Ok(())
    }

    /// Join-path stale window (#388): a session whose peer has sent no RTCP
    /// receiver report for this long is a vanished-peer zombie. 60 s matches the
    /// periodic-sweep window in the supervisor — long enough that a momentary
    /// RTCP gap on a healthy link never trips it, short enough that the cap
    /// self-heals on the next join after a TV reboots.
    pub(super) const JOIN_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

    /// The WHEP join gate, shared by production `add_consumer` and the test
    /// stub (`add_consumer_stub`): FIRST reap zombies (BOTH RTCP-stale and
    /// connection-state-dead), THEN enforce the consumer cap. Reaping before
    /// the cap check makes a pipeline full of zombies self-heal exactly when a
    /// new display tries to join — the 2026-06-12 incident (twice): TVs
    /// power-cycled without WHEP DELETE left 8 zombie sessions behind, the cap
    /// rejected every new consumer with 503, and all displays sat on
    /// "Connecting…" until a manual restart.
    ///
    /// The RTCP-stale reap is the load-bearing fix: gst webrtcbin NEVER flips
    /// connection-state for a vanished peer, so the state-based `reap_dead_
    /// sessions` is INERT in production. The stale reap runs FIRST; the cheap
    /// state-based reap stays as a fast path for the rare cases where webrtcbin
    /// DOES report Disconnected/Failed/Closed (e.g. an explicit ICE failure).
    pub(super) async fn reap_and_check_cap(&self) -> Result<(), AddConsumerError> {
        let stale = self.reap_stale_sessions(Self::JOIN_STALE_AFTER).await;
        let dead = self.reap_dead_sessions().await;
        let reaped = stale + dead;
        if reaped > 0 {
            tracing::info!(
                reaped,
                rtcp_stale = stale,
                state_dead = dead,
                "freed consumer slots by reaping zombie WHEP sessions before the cap check"
            );
        }
        self.check_consumer_cap().await
    }

    /// Remove every session whose last-seen webrtcbin `connection-state` is
    /// `Disconnected`, `Failed` or `Closed` — clients that vanished without a
    /// WHEP DELETE (power-cycled stage TVs). `New`/`Connecting` are
    /// mid-negotiation (a display mid-join is NOT a zombie) and `Connected`
    /// is alive, so none of those are touched; a vanished client leaves
    /// `Connected` on its own via webrtcbin's ICE timeout (~5-15 s).
    ///
    /// Returns how many sessions were removed. Teardown runs off the async
    /// thread via `spawn_blocking`, same as `remove_consumer`.
    pub async fn reap_dead_sessions(&self) -> usize {
        let dead: Vec<WhepSession> = {
            let mut sessions = self.sessions.lock().await;
            let dead_ids: Vec<String> = sessions
                .iter()
                .filter(|(_, session)| session_is_dead(session))
                .map(|(id, _)| id.clone())
                .collect();
            dead_ids
                .iter()
                .filter_map(|id| sessions.remove(id))
                .collect()
        };
        let count = dead.len();
        for session in dead {
            reap_one_session(session).await;
        }
        count
    }

    /// Reap every session whose peer has sent no RTCP receiver report for longer
    /// than `stale_after` — the #388 RTCP-liveness reaper. This is the
    /// LOAD-BEARING production fix: gst webrtcbin never flips connection-state
    /// for a vanished peer, so `reap_dead_sessions` is inert in production; the
    /// peer's RR fingerprint is the only reliable server-side liveness signal (a
    /// live consumer sends RRs continuously → fingerprint changes; a vanished
    /// peer's RR freezes).
    ///
    /// For each session it reads `peer_rr_fingerprint` (off the async thread —
    /// get-stats blocks on a promise), then under the session-map lock:
    ///   - fingerprint changed → refresh `last_bytes` + `last_progress = now`
    ///     (the peer sent a fresh RTCP RR → alive);
    ///   - fingerprint unchanged / `0` / unavailable AND
    ///     `now - last_progress > stale_after` → mark for reap.
    ///
    /// Returns how many sessions were removed. Teardown runs off the async
    /// thread via `spawn_blocking`, same as `reap_dead_sessions`.
    pub async fn reap_stale_sessions(&self, stale_after: std::time::Duration) -> usize {
        // Phase 1: snapshot (id, webrtcbin) under the lock — cheap clone.
        let probes: Vec<(String, gst::Element)> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, s)| (id.clone(), s.webrtcbin.clone()))
                .collect()
        };
        // Phase 2 (blocking): read each webrtcbin's peer-RR fingerprint — the
        // get-stats promise wait must NOT block the async thread.
        let reads: Vec<(String, Option<u64>)> = tokio::task::spawn_blocking(move || {
            probes
                .into_iter()
                .map(|(id, webrtcbin)| (id, peer_rr_fingerprint(&webrtcbin)))
                .collect()
        })
        .await
        .unwrap_or_default();
        // Phase 3: update liveness + collect the stale ids under the lock.
        let now = std::time::Instant::now();
        let stale: Vec<WhepSession> = {
            let mut sessions = self.sessions.lock().await;
            let stale_ids: Vec<String> = reads
                .into_iter()
                .filter(|(id, _)| sessions.contains_key(id))
                .filter(|(id, fingerprint)| {
                    let session = &sessions[id];
                    liveness_is_stale(&session.liveness, *fingerprint, now, stale_after)
                })
                .map(|(id, _)| id)
                .collect();
            stale_ids
                .iter()
                .filter_map(|id| sessions.remove(id))
                .collect()
        };
        let count = stale.len();
        for session in stale {
            reap_one_stale_session(session).await;
        }
        count
    }
}

/// True when a session's last reported `connection-state` marks the client
/// gone: `Disconnected`, `Failed` or `Closed`. See `reap_dead_sessions` for
/// why `New`/`Connecting`/`Connected` are explicitly NOT dead.
fn session_is_dead(session: &WhepSession) -> bool {
    matches!(
        *session
            .connection_state
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
        WhepConnectionState::Disconnected
            | WhepConnectionState::Failed
            | WhepConnectionState::Closed
    )
}

/// Tear one reaped zombie down off the async thread (blocking GStreamer
/// Null-state transition — the same `spawn_blocking` pattern as
/// `remove_consumer`) and log the forensic counters: a zombie typically
/// shows a huge `buffers_pushed` (frames pushed all night to nobody).
async fn reap_one_session(session: WhepSession) {
    let session_id = session.session_id.clone();
    let connection_state = *session
        .connection_state
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let pushed = session.link.pushed();
    let dropped = session.link.dropped();
    if let Err(e) = tokio::task::spawn_blocking(move || drop(session)).await {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "reaped zombie session's teardown task failed"
        );
    }
    tracing::info!(
        session_id = %session_id,
        connection_state = ?connection_state,
        buffers_pushed = pushed,
        buffers_dropped = dropped,
        "Reaped dead WHEP session (client vanished without DELETE)"
    );
}

/// Decide whether a session is RTCP-stale (#388), updating its liveness tracker
/// in place. Returns true ⇒ the session should be reaped.
///
/// `fingerprint` is a hash of the peer's RTCP RR-volatile stats (see
/// `peer_rr_fingerprint`). It CHANGES every sample while the peer is alive
/// (each RR carries fresh round-trip/jitter/RR-block values) and freezes — or
/// drops to the `0` sentinel when webrtcbin removes the `remote-inbound-rtp`
/// stats — once the peer vanishes.
///
/// - `fingerprint = Some(fp)` and `fp != last_bytes` AND `fp != 0` → a fresh RR
///   arrived; record it + `last_progress = now`, return false (alive).
/// - otherwise (unchanged fingerprint, `0` sentinel, or get-stats unavailable)
///   → NO progress; stale only if `now - last_progress > stale_after`.
///
/// `last_progress` advances ONLY on a fresh RR, so the stale window (60 s) is
/// measured from the peer's last receiver report. A live peer reports far more
/// often than once per 60 s, so it is never reaped; a transient get-stats miss
/// is harmless for the same reason. A vanished peer reports never again and
/// ages out. NOTE: the `0` sentinel is deliberately NOT treated as progress —
/// before the first RR a brand-new consumer is protected by the `new()` grace
/// window, not by a fingerprint match.
///
/// Holds only the session's own `std::sync::Mutex<LivenessState>` for a trivial
/// compare-and-swap — never across an await. Poison is recovered.
fn liveness_is_stale(
    liveness: &std::sync::Mutex<LivenessState>,
    fingerprint: Option<u64>,
    now: std::time::Instant,
    stale_after: std::time::Duration,
) -> bool {
    let mut state = liveness.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(fp) = fingerprint {
        if fp != 0 && fp != state.last_bytes {
            state.last_bytes = fp;
            state.last_progress = now;
            return false;
        }
    }
    now.duration_since(state.last_progress) > stale_after
}

/// Tear one RTCP-stale zombie down off the async thread and log the forensic
/// counters with `reason="rtcp-stale"` (#388 observability) — a zombie typically
/// shows a huge `buffers_pushed` (frames pushed all night to a peer that
/// vanished without WHEP DELETE) against a frozen RR fingerprint.
async fn reap_one_stale_session(session: WhepSession) {
    let session_id = session.session_id.clone();
    let last_rr = session
        .liveness
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .last_bytes;
    let pushed = session.link.pushed();
    let dropped = session.link.dropped();
    if let Err(e) = tokio::task::spawn_blocking(move || drop(session)).await {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "reaped RTCP-stale session's teardown task failed"
        );
    }
    tracing::info!(
        session_id = %session_id,
        reason = "rtcp-stale",
        last_rr_fingerprint = last_rr,
        buffers_pushed = pushed,
        buffers_dropped = dropped,
        "Reaped RTCP-stale WHEP session (peer stopped sending RTCP receiver \
         reports past the stale window — vanished without WHEP DELETE)"
    );
}

/// Fingerprint of the peer's RTCP receiver-report state (#388) — the liveness
/// signal that distinguishes a live WHEP consumer from a vanished-peer zombie.
///
/// A live consumer sends RTCP RRs continuously, so webrtcbin's
/// `remote-inbound-rtp` stats (the peer's view of OUR stream: round-trip-time,
/// jitter, and the RR block `rb-*` inside `gst-rtpsource-stats`) update every
/// sample. The instant the peer vanishes those RRs stop: webrtcbin freezes the
/// values and soon DROPS the `remote-inbound-rtp` struct entirely. We hash the
/// RR-volatile fields into one u64 — it CHANGES while alive, FREEZES (or falls
/// to the `0` sentinel when the struct is gone) when dead.
///
/// Why not a byte counter: on gst 1.24 the `transport` stats carry no received
/// counter, and the only nested counters that advance (`octets-received` /
/// `packets-received` on the internal SENDER source) track what WE send — so
/// they advance for a zombie too (we keep encoding to nobody). The RR fields
/// are the ONLY stats driven exclusively by the peer. (Verified live on dev2
/// gst 1.24.2: a connected Chrome consumer's fingerprint changes every sample
/// across a 95s hold; killing the page without WHEP DELETE freezes it and the
/// session is reaped within one stale window.)
///
/// Returns `Some(0)` when connected but no RR is present yet (the sentinel:
/// "not alive THIS sample" — never counted as progress), `Some(fp)` when an RR
/// is present, and None only when get-stats times out (500 ms bound). Mirrors
/// the get-stats idiom of `rtcp_remote_inbound`. NOTE: exercised by the live
/// functional check; unit tests inject `LivenessState` via the test seam.
fn peer_rr_fingerprint(webrtcbin: &gst::Element) -> Option<u64> {
    let (tx, rx) = std::sync::mpsc::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Ok(Some(stats)) = reply {
            let _ = tx.send(stats.to_owned());
        }
    });
    webrtcbin.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    let stats = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .ok()?;
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut saw_rr = false;
    for (_field, value) in stats.iter() {
        let Ok(s) = value.get::<gst::Structure>() else {
            continue;
        };
        // The peer's RR lives in `remote-inbound-rtp` (round-trip-time/jitter)
        // and the RR block inside any struct's nested `gst-rtpsource-stats`.
        if s.has_field("round-trip-time") {
            saw_rr = true;
            hash_f64_bits(&mut hasher, &s, "round-trip-time");
            hash_f64_bits(&mut hasher, &s, "jitter");
        }
        if let Ok(nested) = s.get::<gst::Structure>("gst-rtpsource-stats") {
            // The RR block: these advance only as the peer reports reception of
            // OUR stream. `have-rb=true` while the peer is sending RRs.
            if nested.get::<bool>("have-rb").unwrap_or(false) {
                saw_rr = true;
                for f in [
                    "rb-exthighestseq",
                    "rb-jitter",
                    "rb-lsr",
                    "rb-dlsr",
                    "rb-round-trip",
                ] {
                    if let Some(v) = stats_u64(&nested, f) {
                        v.hash(&mut hasher);
                    }
                }
            }
        }
    }
    Some(if saw_rr { hasher.finish() } else { 0 })
}

/// Mix a stats field's f64 bit-pattern into the fingerprint hasher (round-trip
/// time / jitter are doubles; hashing their raw bits is exact and order-stable).
fn hash_f64_bits(hasher: &mut impl std::hash::Hasher, s: &gst::Structure, field: &str) {
    if let Ok(v) = s.get::<f64>(field) {
        std::hash::Hash::hash(&v.to_bits(), hasher);
    }
}

/// Read a numeric stats field as u64, tolerating the u64/i64/u32 representation
/// webrtcbin uses for different counters across GStreamer versions.
fn stats_u64(s: &gst::Structure, field: &str) -> Option<u64> {
    s.get::<u64>(field)
        .ok()
        .or_else(|| s.get::<i64>(field).ok().map(|v| v.max(0) as u64))
        .or_else(|| s.get::<u32>(field).ok().map(u64::from))
}
