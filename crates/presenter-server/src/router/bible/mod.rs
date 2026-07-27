//! Bible feature: translation admin, browsing, broadcast/trigger endpoints,
//! slide resolution, and presentation CRUD. Split into per-concern
//! submodules (#590 — `router/bible.rs` crossed the 800-line warning cap) —
//! same pattern as `router/integrations/`: each submodule is a
//! self-contained handler group with `pub(crate)` items so `router.rs`'s
//! route table can reference them directly.

pub(super) mod broadcast;
pub(super) mod browse;
pub(super) mod presentations;
pub(super) mod resolve;
pub(super) mod translations;
