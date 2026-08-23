use chrono::{DateTime, Duration, Utc};
use presenter_core::{NdiVideoDiag, StageClientSnapshot, StageClientStatus};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

/// #732: how quietly to log an UNCHANGED diagnostics snapshot. A change of
/// any dimension in [`DiagLogKey`] always logs; otherwise at most one line
/// per this interval, per stage connection.
const DIAG_LOG_MIN_INTERVAL_S: i64 = 30;

/// #732: the dimensions whose change forces an immediate diagnostics log line
/// (`paused`, `error_code`, `cover_visible`, and `video_width==0 vs >0`).
/// Everything else in the snapshot is stored/exposed but does not by itself
/// trigger a fresh log line — the 30 s floor covers steady-state drift.
type DiagLogKey = (Option<bool>, Option<u16>, Option<bool>, Option<bool>);

/// The [`DiagLogKey`] for a snapshot — `video_width` collapses to a
/// present/absent-frame boolean (0 vs >0), matching the ticket's log rule.
fn diag_log_key(diag: &NdiVideoDiag) -> DiagLogKey {
    (
        diag.paused,
        diag.error_code,
        diag.cover_visible,
        diag.video_width.map(|w| w > 0),
    )
}

/// #732 rate-limiter (pure — unit-tested): should this diagnostics snapshot be
/// logged now? Logs when it was never logged, when the key CHANGED, or when
/// `min_interval` has elapsed since the last log.
pub(crate) fn should_log_diag(
    prev_key: Option<&DiagLogKey>,
    prev_log_at: Option<DateTime<Utc>>,
    new_key: &DiagLogKey,
    now: DateTime<Utc>,
    min_interval: Duration,
) -> bool {
    match prev_key {
        None => true,
        Some(prev) if prev != new_key => true,
        _ => match prev_log_at {
            None => true,
            Some(at) => now.signed_duration_since(at) >= min_interval,
        },
    }
}

/// Result of recording a diagnostics snapshot: the updated per-connection
/// snapshot plus whether the rate-limiter says to emit a log line for it.
#[derive(Debug)]
pub struct DiagRecord {
    pub snapshot: StageClientSnapshot,
    pub should_log: bool,
}

#[derive(Debug)]
struct StageConnection {
    layout_code: String,
    last_heartbeat: DateTime<Utc>,
    pending_heartbeat: Option<(Uuid, DateTime<Utc>)>,
    last_round_trip: Option<Duration>,
    status: StageClientStatus,
    /// #732 diagnostics (see `StageClientSnapshot` / `NdiVideoDiag`).
    user_agent: Option<String>,
    ndi_video: Option<NdiVideoDiag>,
    last_diag_at: Option<DateTime<Utc>>,
    last_logged_key: Option<DiagLogKey>,
    last_log_at: Option<DateTime<Utc>>,
}

impl StageConnection {
    fn new(layout_code: &str, now: DateTime<Utc>) -> Self {
        Self {
            layout_code: layout_code.to_string(),
            last_heartbeat: now,
            pending_heartbeat: None,
            last_round_trip: None,
            status: StageClientStatus::Connecting,
            user_agent: None,
            ndi_video: None,
            last_diag_at: None,
            last_logged_key: None,
            last_log_at: None,
        }
    }

    fn snapshot(&self, id: Uuid) -> StageClientSnapshot {
        StageClientSnapshot {
            id,
            layout_code: self.layout_code.clone(),
            last_heartbeat: self.last_heartbeat,
            latency_ms: self
                .last_round_trip
                .and_then(|duration| duration.to_std().ok())
                .map(|std| std.as_millis().min(u32::MAX as u128) as u32),
            status: self.status,
            user_agent: self.user_agent.clone(),
            ndi_video: self.ndi_video.clone(),
            last_diag_at: self.last_diag_at,
        }
    }
}

#[derive(Debug, Default)]
pub struct StageConnectionTracker {
    connections: HashMap<Uuid, StageConnection>,
}

impl StageConnectionTracker {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        id: Uuid,
        layout_code: &str,
        now: DateTime<Utc>,
    ) -> StageClientSnapshot {
        let connection = StageConnection::new(layout_code, now);
        let snapshot = connection.snapshot(id);
        self.connections.insert(id, connection);
        snapshot
    }

    pub fn note_heartbeat_sent(&mut self, heartbeat_id: Uuid, now: DateTime<Utc>) {
        for connection in self.connections.values_mut() {
            connection.pending_heartbeat = Some((heartbeat_id, now));
        }
    }

    pub fn record_heartbeat_ack(
        &mut self,
        id: Uuid,
        heartbeat_id: Option<Uuid>,
        now: DateTime<Utc>,
    ) -> Option<StageClientSnapshot> {
        let connection = self.connections.get_mut(&id)?;
        connection.last_heartbeat = now;
        connection.status = StageClientStatus::Connected;
        if let (Some(expected_id), Some((pending_id, sent_at))) =
            (heartbeat_id, connection.pending_heartbeat)
        {
            if expected_id == pending_id {
                let round_trip = now.signed_duration_since(sent_at);
                let non_negative = if round_trip < Duration::zero() {
                    Duration::zero()
                } else {
                    round_trip
                };
                connection.last_round_trip = Some(non_negative);
                connection.pending_heartbeat = None;
            }
        }
        Some(connection.snapshot(id))
    }

    pub fn mark_disconnected(&mut self, id: Uuid) -> Option<StageClientSnapshot> {
        let connection = self.connections.get_mut(&id)?;
        connection.status = StageClientStatus::Disconnected;
        Some(connection.snapshot(id))
    }

    /// #732: store the connecting client's userAgent (set once on presence).
    /// A `None` never overwrites a previously-recorded UA.
    pub fn set_user_agent(&mut self, id: Uuid, user_agent: Option<String>) {
        if let (Some(connection), Some(ua)) = (self.connections.get_mut(&id), user_agent) {
            connection.user_agent = Some(ua);
        }
    }

    /// #732: store the latest NDI `<video>` diagnostics snapshot and decide,
    /// via the rate-limiter, whether to emit a log line. Returns `None` when
    /// the connection is unknown (e.g. a preview client that never registered).
    pub fn record_diag(
        &mut self,
        id: Uuid,
        diag: NdiVideoDiag,
        now: DateTime<Utc>,
    ) -> Option<DiagRecord> {
        let connection = self.connections.get_mut(&id)?;
        let key = diag_log_key(&diag);
        let should_log = should_log_diag(
            connection.last_logged_key.as_ref(),
            connection.last_log_at,
            &key,
            now,
            Duration::seconds(DIAG_LOG_MIN_INTERVAL_S),
        );
        connection.ndi_video = Some(diag);
        connection.last_diag_at = Some(now);
        if should_log {
            connection.last_logged_key = Some(key);
            connection.last_log_at = Some(now);
        }
        Some(DiagRecord {
            snapshot: connection.snapshot(id),
            should_log,
        })
    }

    pub fn poll_timeouts(
        &mut self,
        now: DateTime<Utc>,
        grace_interval: Duration,
        disconnect_after: Duration,
    ) -> Vec<(Uuid, StageClientStatus)> {
        let mut changed = Vec::new();
        for (id, connection) in &mut self.connections {
            if connection.status == StageClientStatus::Disconnected {
                continue;
            }
            let since = now.signed_duration_since(connection.last_heartbeat);
            let since = if since < Duration::zero() {
                Duration::zero()
            } else {
                since
            };

            if since >= disconnect_after {
                if connection.status != StageClientStatus::Disconnected {
                    connection.status = StageClientStatus::Disconnected;
                    changed.push((*id, StageClientStatus::Disconnected));
                }
            } else if since >= grace_interval {
                if connection.status != StageClientStatus::Reconnecting {
                    connection.status = StageClientStatus::Reconnecting;
                    changed.push((*id, StageClientStatus::Reconnecting));
                }
            } else if connection.status != StageClientStatus::Connected {
                connection.status = StageClientStatus::Connected;
                changed.push((*id, StageClientStatus::Connected));
            }
        }
        changed
    }

    pub fn snapshot(&self) -> Vec<StageClientSnapshot> {
        let mut snapshots: Vec<_> = self
            .connections
            .iter()
            .map(|(id, connection)| connection.snapshot(*id))
            .collect();
        snapshots.sort_by(|a, b| {
            a.layout_code
                .cmp(&b.layout_code)
                .then_with(|| a.id.cmp(&b.id))
        });
        snapshots
    }

    pub fn snapshot_for(&self, id: Uuid) -> Option<StageClientSnapshot> {
        self.connections
            .get(&id)
            .map(|connection| connection.snapshot(id))
    }
}

#[derive(Clone, Default)]
pub struct StageConnections {
    inner: Arc<RwLock<StageConnectionTracker>>,
}

impl StageConnections {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StageConnectionTracker::new())),
        }
    }

    pub async fn register(
        &self,
        id: Uuid,
        layout_code: &str,
        now: DateTime<Utc>,
    ) -> StageClientSnapshot {
        let mut guard = self.inner.write().await;
        guard.register(id, layout_code, now)
    }

    pub async fn note_heartbeat_sent(&self, heartbeat_id: Uuid, now: DateTime<Utc>) {
        let mut guard = self.inner.write().await;
        guard.note_heartbeat_sent(heartbeat_id, now);
    }

    pub async fn record_heartbeat_ack(
        &self,
        id: Uuid,
        heartbeat_id: Option<Uuid>,
        now: DateTime<Utc>,
    ) -> Option<StageClientSnapshot> {
        let mut guard = self.inner.write().await;
        guard.record_heartbeat_ack(id, heartbeat_id, now)
    }

    pub async fn mark_disconnected(&self, id: Uuid) -> Option<StageClientSnapshot> {
        let mut guard = self.inner.write().await;
        guard.mark_disconnected(id)
    }

    /// #732: store the connecting client's userAgent (once, on presence).
    pub async fn set_user_agent(&self, id: Uuid, user_agent: Option<String>) {
        let mut guard = self.inner.write().await;
        guard.set_user_agent(id, user_agent);
    }

    /// #732: store the latest NDI `<video>` diagnostics snapshot and return
    /// the rate-limiter's log decision.
    pub async fn record_diag(
        &self,
        id: Uuid,
        diag: NdiVideoDiag,
        now: DateTime<Utc>,
    ) -> Option<DiagRecord> {
        let mut guard = self.inner.write().await;
        guard.record_diag(id, diag, now)
    }

    pub async fn apply_timeouts(
        &self,
        now: DateTime<Utc>,
        grace_interval: Duration,
        disconnect_after: Duration,
    ) -> Vec<StageClientSnapshot> {
        let mut guard = self.inner.write().await;
        let changed = guard.poll_timeouts(now, grace_interval, disconnect_after);
        if changed.is_empty() {
            Vec::new()
        } else {
            changed
                .into_iter()
                .filter_map(|(id, _)| guard.snapshot_for(id))
                .collect()
        }
    }

    pub async fn snapshot(&self) -> Vec<StageClientSnapshot> {
        self.inner.read().await.snapshot()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StageHeartbeatConfig {
    pub interval: std::time::Duration,
    pub grace: std::time::Duration,
    pub disconnect_after: std::time::Duration,
}

impl StageHeartbeatConfig {
    pub fn new(
        interval: std::time::Duration,
        grace: std::time::Duration,
        disconnect_after: std::time::Duration,
    ) -> Self {
        Self {
            interval,
            grace,
            disconnect_after,
        }
    }

    pub fn default_values() -> Self {
        Self::new(
            std::time::Duration::from_millis(1_500),
            std::time::Duration::from_millis(4_500),
            std::time::Duration::from_millis(12_000),
        )
    }

    pub fn grace_duration(&self) -> Duration {
        Duration::from_std(self.grace).unwrap_or_else(|_| {
            let millis = self.grace.as_millis().min(i64::MAX as u128) as i64;
            Duration::milliseconds(millis)
        })
    }

    pub fn disconnect_duration(&self) -> Duration {
        Duration::from_std(self.disconnect_after).unwrap_or_else(|_| {
            let millis = self.disconnect_after.as_millis().min(i64::MAX as u128) as i64;
            Duration::milliseconds(millis)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::DEFAULT_STAGE_LAYOUT_CODE;

    #[test]
    fn registers_connection_and_reports_connected_after_heartbeat() {
        let mut tracker = StageConnectionTracker::new();
        let now = Utc::now();
        let id = Uuid::new_v4();
        tracker.register(id, DEFAULT_STAGE_LAYOUT_CODE, now);

        let initial = tracker.snapshot();
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].status, StageClientStatus::Connecting);
        assert_eq!(initial[0].layout_code, DEFAULT_STAGE_LAYOUT_CODE);
        assert_eq!(initial[0].latency_ms, None);

        let later = now + Duration::milliseconds(120);
        tracker.note_heartbeat_sent(Uuid::new_v4(), now);
        tracker.record_heartbeat_ack(id, None, later);

        let updated = tracker.snapshot();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].status, StageClientStatus::Connected);
        assert_eq!(updated[0].last_heartbeat, later);
    }

    #[test]
    fn latency_is_recorded_from_ack_round_trip() {
        let mut tracker = StageConnectionTracker::new();
        let now = Utc::now();
        let id = Uuid::new_v4();
        let heartbeat_id = Uuid::new_v4();
        tracker.register(id, "timer", now);
        tracker.note_heartbeat_sent(heartbeat_id, now);

        let ack_time = now + Duration::milliseconds(42);
        tracker.record_heartbeat_ack(id, Some(heartbeat_id), ack_time);

        let snapshot = tracker.snapshot_for(id).expect("snapshot");
        assert_eq!(snapshot.status, StageClientStatus::Connected);
        assert_eq!(snapshot.latency_ms, Some(42));
    }

    #[test]
    fn timeouts_escalate_status_from_reconnecting_to_disconnected() {
        let mut tracker = StageConnectionTracker::new();
        let start = Utc::now();
        let id = Uuid::new_v4();
        tracker.register(id, "timer", start);
        tracker.note_heartbeat_sent(Uuid::new_v4(), start);
        tracker.record_heartbeat_ack(id, None, start + Duration::milliseconds(80));

        tracker.poll_timeouts(
            start + Duration::milliseconds(600),
            Duration::milliseconds(300),
            Duration::milliseconds(900),
        );

        let reconnecting = tracker
            .snapshot()
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .expect("connection snapshot");
        assert_eq!(reconnecting.status, StageClientStatus::Reconnecting);

        tracker.poll_timeouts(
            start + Duration::milliseconds(1200),
            Duration::milliseconds(300),
            Duration::milliseconds(900),
        );

        let disconnected = tracker
            .snapshot()
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .expect("connection snapshot");
        assert_eq!(disconnected.status, StageClientStatus::Disconnected);
    }

    // ── #732 diagnostics rate-limiter + storage ─────────────────────

    fn diag(paused: bool, error_code: Option<u16>, cover: bool, width: u32) -> NdiVideoDiag {
        NdiVideoDiag {
            paused: Some(paused),
            error_code,
            cover_visible: Some(cover),
            video_width: Some(width),
            ..Default::default()
        }
    }

    #[test]
    fn should_log_diag_logs_first_snapshot_then_throttles_unchanged() {
        let now = Utc::now();
        let key = diag_log_key(&diag(false, None, false, 1280));
        // Never logged → log.
        assert!(should_log_diag(
            None,
            None,
            &key,
            now,
            Duration::seconds(30)
        ));
        // Same key, only 5 s later → throttled.
        assert!(!should_log_diag(
            Some(&key),
            Some(now),
            &key,
            now + Duration::seconds(5),
            Duration::seconds(30),
        ));
        // Same key, 30 s later → log (interval elapsed).
        assert!(should_log_diag(
            Some(&key),
            Some(now),
            &key,
            now + Duration::seconds(30),
            Duration::seconds(30),
        ));
    }

    #[test]
    fn should_log_diag_always_logs_on_key_change() {
        let now = Utc::now();
        let playing = diag_log_key(&diag(false, None, false, 1280));
        // paused flip → log immediately, even 1 ms later.
        let paused = diag_log_key(&diag(true, None, false, 1280));
        assert!(should_log_diag(
            Some(&playing),
            Some(now),
            &paused,
            now + Duration::milliseconds(1),
            Duration::seconds(30),
        ));
        // error_code appears → log.
        let errored = diag_log_key(&diag(false, Some(3), false, 1280));
        assert!(should_log_diag(
            Some(&playing),
            Some(now),
            &errored,
            now + Duration::milliseconds(1),
            Duration::seconds(30),
        ));
        // cover appears → log.
        let covered = diag_log_key(&diag(false, None, true, 1280));
        assert!(should_log_diag(
            Some(&playing),
            Some(now),
            &covered,
            now + Duration::milliseconds(1),
            Duration::seconds(30),
        ));
        // width 1280→0 (frame lost) → log.
        let noframe = diag_log_key(&diag(false, None, false, 0));
        assert!(should_log_diag(
            Some(&playing),
            Some(now),
            &noframe,
            now + Duration::milliseconds(1),
            Duration::seconds(30),
        ));
    }

    #[test]
    fn diag_log_key_collapses_width_to_present_absent() {
        // Any positive width shares the same key dimension; only 0-vs->0 flips.
        assert_eq!(
            diag_log_key(&diag(false, None, false, 1280)),
            diag_log_key(&diag(false, None, false, 640)),
        );
        assert_ne!(
            diag_log_key(&diag(false, None, false, 1280)),
            diag_log_key(&diag(false, None, false, 0)),
        );
    }

    #[test]
    fn record_diag_stores_snapshot_and_first_record_logs() {
        let mut tracker = StageConnectionTracker::new();
        let now = Utc::now();
        let id = Uuid::new_v4();
        tracker.register(id, "ndi-fullscreen", now);

        let record = tracker
            .record_diag(id, diag(false, None, false, 1280), now)
            .expect("record");
        assert!(record.should_log, "first diagnostics snapshot must log");
        let stored = record.snapshot.ndi_video.expect("ndi_video stored");
        assert_eq!(stored.video_width, Some(1280));
        assert!(record.snapshot.last_diag_at.is_some());

        // An unchanged second snapshot 1 s later is throttled.
        let record2 = tracker
            .record_diag(
                id,
                diag(false, None, false, 1280),
                now + Duration::seconds(1),
            )
            .expect("record");
        assert!(
            !record2.should_log,
            "unchanged snapshot within 30 s throttles"
        );
    }

    #[test]
    fn record_diag_for_unknown_connection_is_none() {
        let mut tracker = StageConnectionTracker::new();
        let record = tracker.record_diag(Uuid::new_v4(), diag(false, None, false, 0), Utc::now());
        assert!(record.is_none());
    }

    #[test]
    fn set_user_agent_stores_and_none_never_overwrites() {
        let mut tracker = StageConnectionTracker::new();
        let now = Utc::now();
        let id = Uuid::new_v4();
        tracker.register(id, "ndi-fullscreen", now);

        tracker.set_user_agent(id, Some("Chrome/90 (Vestel)".to_string()));
        assert_eq!(
            tracker.snapshot_for(id).unwrap().user_agent.as_deref(),
            Some("Chrome/90 (Vestel)"),
        );
        // A later None presence must not wipe the recorded UA.
        tracker.set_user_agent(id, None);
        assert_eq!(
            tracker.snapshot_for(id).unwrap().user_agent.as_deref(),
            Some("Chrome/90 (Vestel)"),
        );
    }
}
