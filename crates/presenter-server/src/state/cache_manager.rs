//! In-memory caches owned by [`AppState`].
//!
//! Groups the three `Arc<RwLock<_>>` caches that were previously inline fields
//! on `AppState`:
//!
//! - `presentation`: cached presentation detail for stage display
//! - `group_color`: cached group name → hex color mapping
//! - `ableset`: cached AbleSet library-to-playlist mapping
//!
//! This is a pure container — it holds the same `Arc<RwLock<_>>` handles that
//! `AppState` used to hold directly, so the documented single-lock-at-a-time
//! acquisition policy (see the `state` module docs) is unchanged. Callers
//! acquire exactly one of these locks at a time, exactly as before; the only
//! difference is the field path (`state.caches.ableset` instead of
//! `state.ableset_cache`).

use std::{collections::HashMap, sync::Arc};

use presenter_core::{Presentation, PresentationId};
use tokio::sync::RwLock;

use super::ableset::AbleSetLibraryCache;

/// Owns the in-memory caches threaded through `AppState`.
///
/// `Clone` is cheap: every field is an `Arc`, so cloning shares the underlying
/// locks (matching the previous inline-`Arc` semantics on `AppState`).
#[derive(Clone)]
pub(crate) struct CacheManager {
    /// Cached presentation detail keyed by id (stage display fast path).
    pub(crate) presentation: Arc<RwLock<HashMap<PresentationId, Arc<Presentation>>>>,
    /// Cached group name → hex color mapping.
    pub(crate) group_color: Arc<RwLock<HashMap<String, String>>>,
    /// Cached AbleSet library-to-playlist mapping.
    pub(crate) ableset: Arc<RwLock<AbleSetLibraryCache>>,
}

impl CacheManager {
    /// Build a fresh, empty cache set. Mirrors the previous inline
    /// initialisation in `AppState::new_with_heartbeat`.
    pub(crate) fn new() -> Self {
        Self {
            presentation: Arc::new(RwLock::new(HashMap::new())),
            group_color: Arc::new(RwLock::new(HashMap::new())),
            ableset: Arc::new(RwLock::new(AbleSetLibraryCache::default())),
        }
    }
}
