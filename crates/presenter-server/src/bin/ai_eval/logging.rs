//! `ai_eval`'s own tracing/log filter (#688).
//!
//! `AppState::in_memory()` boots fresh once per corpus case (30 boots in a
//! 30-case run), and `seed::build_state_for_case` re-ingests the full 5-file
//! default bible translation set from scratch on every bible-authoring /
//! adversarial case (23/30 in the #662 smoke-run). Both re-fire the SAME
//! handful of known-benign `tracing::warn!` sites every single time —
//! ~880 WARN lines over 30 cases in that smoke-run, even with `RUST_LOG=warn`
//! set explicitly (issuecomment-5279674449, item 5). This module builds a
//! filter that demotes exactly those sources, ON TOP OF the caller's own
//! base level, so a real `RUST_LOG=warn` (or whatever the operator sets)
//! still holds for everything else.
//!
//! This NEVER touches `presenter-server`'s own production log levels: the
//! filter here is only ever constructed inside `ai_eval`'s own `main()` —
//! `presenter-server`'s production `main.rs` builds its subscriber
//! completely independently and never calls anything in this module.

use anyhow::Context;
use tracing_subscriber::EnvFilter;

/// Directives appended after the caller's own base level. Each was verified
/// SAFE to silence for its whole target scope by reading (not assuming) every
/// site inside that scope and every caller that could reach it during an
/// actual corpus-case run — see the per-directive notes below. Not exhaustive
/// against every possible future addition; see the `state::ndi_probe` note
/// for why that one is a single-line custom target rather than a module-wide
/// one.
///
/// - `presenter_bible::parsers=error` — data-quality warnings fired while
///   parsing the 5 default bible translations ("skipping verse with
///   unmapped book code", "dropping passage without book/chapter context",
///   "skipping MySword row without book code", etc. — `presenter-bible/src/
///   parsers.rs`). This module's SOLE purpose is bible-content parsing —
///   nothing else lives there this filter could accidentally hide — and
///   `ai_eval` re-ingests the SAME 5 translations from scratch on every
///   bible-authoring/adversarial case, so these are the identical,
///   deterministic content quirks re-logged 23 times over in a 30-case run,
///   not a harness/environment signal. A genuine ingestion FAILURE (a
///   missing/unreadable archive) still surfaces structurally through
///   `refresh_default_bible_translations`'s `anyhow::Result` ->
///   `Trace.seed_failed`/`error` (#662 defect 1) and `main.rs`'s own
///   `eprintln!` per-case reporting — neither is log-level-gated, so
///   demoting this target cannot hide that class of failure.
/// - `presenter_server::android_stage=error` — `AndroidStageRegistry::new()`'s
///   "launcher URL is unset" WARN (`android_stage.rs`), fired once per fresh
///   `AppState::in_memory()` boot because `PRESENTER_ANDROID_STAGE_URL` is
///   never set for `ai_eval` (no Android TVs in an eval run — always
///   expected there). Grep-verified: no `ai/tools/*` code path this harness
///   drives ever touches `AndroidStageRegistry` beyond construction
///   (`sync_android_stage_displays` against a fresh, display-less DB calls
///   `set_displays(vec![])`, a no-op that spawns zero device workers) — so
///   every OTHER warn! in that module (the per-device launch/install worker
///   loop) is structurally unreachable in an eval run too.
/// - `presenter_server::state::ndi_probe=error` — the ONE line inside
///   `state/mod.rs` given this narrow custom target (see that file), instead
///   of demoting the whole (large, shared) `presenter_server::state` module.
///   `state` is this app's biggest catch-all module — bible.rs,
///   presentations.rs, slides.rs, integrations.rs, etc. all live under it —
///   so a module-wide directive would risk silently swallowing an unrelated
///   WARN some OTHER state submodule adds later. The custom target keeps the
///   blast radius to exactly the one line that actually fires during
///   `AppState::in_memory()` boot.
const EVAL_NOISE_DIRECTIVES: &str = "presenter_bible::parsers=error,\
     presenter_server::android_stage=error,\
     presenter_server::state::ndi_probe=error";

/// Build the filter `main()` installs: the caller's own base level (their
/// `RUST_LOG`, or the harness's "info" default set in `main()`) plus the
/// noise directives above. Takes the base level as a plain parameter —
/// never reads the env var itself — so this stays a pure, directly
/// unit-testable function with no shared-process-env-var races across
/// parallel `cargo test` threads.
pub fn build_eval_log_filter(base_rust_log: &str) -> anyhow::Result<EnvFilter> {
    let spec = format!("{base_rust_log},{EVAL_NOISE_DIRECTIVES}");
    EnvFilter::try_new(&spec).with_context(|| format!("invalid ai_eval log filter spec: {spec}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_callers_base_level_and_adds_all_three_noise_directives() {
        let filter = build_eval_log_filter("info").expect("valid base level");
        let rendered = filter.to_string();
        assert!(rendered.contains("info"), "base level dropped: {rendered}");
        assert!(
            rendered.contains("presenter_bible::parsers=error"),
            "bible-parser noise directive missing: {rendered}"
        );
        assert!(
            rendered.contains("presenter_server::android_stage=error"),
            "android_stage noise directive missing: {rendered}"
        );
        assert!(
            rendered.contains("presenter_server::state::ndi_probe=error"),
            "ndi_probe noise directive missing: {rendered}"
        );
    }

    /// The #662 smoke-run's own reproduction: an operator who explicitly set
    /// `RUST_LOG=warn` (trying to quiet the harness down) still saw ~880 WARN
    /// lines, because the noisy sites fire AT warn level, not below it. This
    /// is the regression this filter must hold against: the noise
    /// directives apply on top of "warn" too, not only the "info" default.
    #[test]
    fn demotes_the_noise_sources_even_when_the_caller_already_asked_for_warn() {
        let filter = build_eval_log_filter("warn").expect("valid base level");
        let rendered = filter.to_string();
        assert!(rendered.contains("warn"), "base level dropped: {rendered}");
        assert!(rendered.contains("presenter_bible::parsers=error"));
        assert!(rendered.contains("presenter_server::android_stage=error"));
        assert!(rendered.contains("presenter_server::state::ndi_probe=error"));
    }

    #[test]
    fn rejects_a_malformed_base_level() {
        let result = build_eval_log_filter("presenter_server=definitely_not_a_level");
        assert!(
            result.is_err(),
            "expected a parse error for an unrecognized level keyword"
        );
    }
}
