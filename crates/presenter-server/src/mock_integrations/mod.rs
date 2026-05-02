#![cfg(feature = "mock-integrations")]
//! In-process mock listeners for outbound integrations (Resolume HTTP,
//! AbleSet HTTP). Compiled only when the `mock-integrations` feature is
//! enabled — dev builds use this; prod builds omit it entirely.
//!
//! Closes issue #279: prevents dev from sending live commands to
//! production hardware.

use std::sync::Arc;

// pub mod ableset; // added in Task 4
pub mod request_log;
pub mod resolume;

/// Start all mock listeners. Called from main.rs under
/// `#[cfg(feature = "mock-integrations")]`.
pub async fn start_all() -> anyhow::Result<()> {
    let log = Arc::new(request_log::RequestLog::new());
    resolume::spawn(log.clone()).await?;
    // ableset::spawn(log).await?; // added in Task 4
    let _ = log; // placeholder to silence unused-arc warning until Task 4
    Ok(())
}
