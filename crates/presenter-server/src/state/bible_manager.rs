//! Bible broadcast / slide-output state owned by [`AppState`].
//!
//! Groups the Bible-related fields that were previously inline on `AppState`:
//!
//! - `broadcast`: current active Bible passage broadcast (legacy `/bible/active`)
//! - `slide_output`: single-source-of-truth Bible slide output
//! - `nlt`: the NLT API client serving the remote `eng-nlt` translation (#826)
//! - `ingestion_override` (test-only): swaps the ingestion service in tests
//!
//! Pure relocation: the two `Arc<RwLock<_>>` handles keep their exact types, so
//! the documented single-lock-at-a-time acquisition policy (see the `state`
//! module docs) is unchanged. `clear_bible_broadcast` still acquires `broadcast`
//! and `slide_output` in two separate scoped blocks (one lock held at a time),
//! exactly as before.

use std::sync::Arc;

use crate::bible_remote::NltClient;
use presenter_core::{BibleBroadcast, BibleSlideOutput};
use tokio::sync::RwLock;

/// Owns the Bible broadcast / slide-output state.
///
/// `Clone` shares the `Arc<RwLock<_>>` handles (and, in tests, the override
/// `Arc`), matching the previous inline semantics on `AppState`.
#[derive(Clone)]
pub(crate) struct BibleManager {
    /// Current active Bible passage broadcast (legacy `/bible/active`).
    pub(crate) broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
    /// Single-source-of-truth Bible slide output.
    pub(crate) slide_output: Arc<RwLock<Option<BibleSlideOutput>>>,
    /// #826: the NLT API client (with its in-memory verse cache), shared by
    /// every clone — see `state/bible_source.rs`.
    pub(crate) nlt: Arc<NltClient>,
    /// Test-only ingestion override (swaps the real ingestion service).
    #[cfg(test)]
    pub(crate) ingestion_override: Option<Arc<dyn super::seed::TestBibleIngestion + Send + Sync>>,
}

impl BibleManager {
    /// Build a fresh manager with empty broadcast/slide-output state.
    /// Mirrors the previous inline initialisation in
    /// `AppState::new_with_heartbeat`.
    pub(crate) fn new() -> Self {
        Self {
            broadcast: Arc::new(RwLock::new(None)),
            slide_output: Arc::new(RwLock::new(None)),
            nlt: Arc::new(NltClient::from_env()),
            #[cfg(test)]
            ingestion_override: None,
        }
    }
}
