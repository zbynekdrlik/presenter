//! Audio ingest building blocks (#852 C1a, spec §5): the VBAN packet codec,
//! the level meter and the per-stream frame-counter rule. Pure, no I/O — the
//! server's `listen` module owns the socket and the resampler.

pub mod level;
pub mod sequence;
pub mod vban;
#[cfg(any(test, feature = "test-helpers"))]
pub mod vban_encode;
