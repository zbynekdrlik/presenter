//! Shared in-memory ring buffer recording mock requests.
//!
//! Capped at 1024 entries; oldest entries are dropped. Reset on process
//! restart (no persistence). Read via `GET /__mock/log` on either mock.

use std::collections::VecDeque;
use std::sync::Mutex;

use axum::{extract::State, response::Json};
use chrono::{DateTime, Utc};
use serde::Serialize;

const MAX_ENTRIES: usize = 1024;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub at: DateTime<Utc>,
    pub mock: &'static str,
    pub method: String,
    pub path: String,
    pub body_preview: Option<String>,
}

pub struct RequestLog {
    entries: Mutex<VecDeque<LogEntry>>,
}

impl RequestLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
        }
    }

    pub fn record(
        &self,
        mock: &'static str,
        method: &str,
        path: &str,
        body_preview: Option<String>,
    ) {
        let entry = LogEntry {
            at: Utc::now(),
            mock,
            method: method.to_string(),
            path: path.to_string(),
            body_preview,
        };
        let mut entries = self.entries.lock().expect("request_log mutex poisoned");
        if entries.len() == MAX_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .expect("request_log mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

/// Axum handler: `GET /__mock/log` returns the current ring buffer as
/// JSON. Mounted by both mocks so either can serve it.
pub async fn log_handler(State(log): State<std::sync::Arc<RequestLog>>) -> Json<Vec<LogEntry>> {
    Json(log.snapshot())
}
