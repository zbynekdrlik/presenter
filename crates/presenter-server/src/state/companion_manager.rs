//! Companion (Bitfocus) integration state owned by [`AppState`].
//!
//! Groups the four Companion-related fields that were previously inline on
//! `AppState`:
//!
//! - `token`: optional shared auth token for the Companion websocket
//! - `enabled`: runtime feature flag (atomic, hot-swappable via settings)
//! - `port`: runtime listen port (atomic, hot-swappable via settings)
//! - `server`: the running websocket server handle manager
//!
//! Pure relocation: the atomics and the server-manager handle keep their exact
//! types and semantics, so the load/store ordering (`Ordering::SeqCst`) and the
//! reconfigure/rollback flow in `AppState::set_companion_settings` are
//! unchanged.

use std::sync::{
    atomic::{AtomicBool, AtomicU16},
    Arc,
};

use super::companion::CompanionServerManager;

/// Owns the Companion integration's runtime state.
///
/// `Clone` shares the atomics and the server handle (all `Arc`-backed),
/// matching the previous inline semantics on `AppState`.
#[derive(Clone)]
pub(crate) struct CompanionManager {
    /// Optional shared auth token for the Companion websocket.
    pub(crate) token: Option<String>,
    /// Runtime feature flag — hot-swappable via settings.
    pub(crate) enabled: Arc<AtomicBool>,
    /// Runtime listen port — hot-swappable via settings.
    pub(crate) port: Arc<AtomicU16>,
    /// The running websocket server handle manager.
    pub(crate) server: CompanionServerManager,
}

impl CompanionManager {
    /// Build the manager from the resolved startup token/enabled/port.
    /// Mirrors the previous inline initialisation in
    /// `AppState::new_with_heartbeat`.
    pub(crate) fn new(token: Option<String>, enabled: bool, port: u16) -> Self {
        Self {
            token,
            enabled: Arc::new(AtomicBool::new(enabled)),
            port: Arc::new(AtomicU16::new(port)),
            server: CompanionServerManager::default(),
        }
    }
}
