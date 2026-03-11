use chrono::{DateTime, Duration, Utc};
use presenter_core::{StageClientSnapshot, StageClientStatus};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug)]
struct StageConnection {
    layout_code: String,
    last_heartbeat: DateTime<Utc>,
    pending_heartbeat: Option<(Uuid, DateTime<Utc>)>,
    last_round_trip: Option<Duration>,
    status: StageClientStatus,
}

impl StageConnection {
    fn new(layout_code: &str, now: DateTime<Utc>) -> Self {
        Self {
            layout_code: layout_code.to_string(),
            last_heartbeat: now,
            pending_heartbeat: None,
            last_round_trip: None,
            status: StageClientStatus::Connecting,
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

    pub fn interval_ms(&self) -> u64 {
        self.interval.as_millis().min(u64::MAX as u128) as u64
    }

    pub fn grace_ms(&self) -> u64 {
        self.grace.as_millis().min(u64::MAX as u128) as u64
    }

    pub fn disconnect_ms(&self) -> u64 {
        self.disconnect_after.as_millis().min(u64::MAX as u128) as u64
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
}
