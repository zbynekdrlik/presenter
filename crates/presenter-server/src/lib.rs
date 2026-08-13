//! Library root for `presenter-server`.
//!
//! Historically this crate was binary-only (everything lived under
//! `main.rs`'s private module tree). #680 needed a SECOND binary
//! (`src/bin/ai_eval.rs`, the AI-eval harness driver, feature `ai-eval`)
//! that reuses the REAL agent/tool code paths (`ai::agent::run_agent`,
//! `ai::tools::execute_tool`, the real bible packer/validator) instead of
//! re-implementing them — and a second `[[bin]]` target is, in Cargo's
//! model, an entirely separate crate root from `main.rs`, with NO access
//! to `main.rs`'s private modules regardless of living in the same
//! package. This `lib.rs` is the fix: the module tree moved here (once),
//! `main.rs` became a thin entry point that links against it like any
//! other consumer, and `src/bin/ai_eval/` links against the exact same
//! compiled code. See #680's design comment on the issue for the full
//! rationale + the alternatives considered.
//!
//! Only the modules a second binary genuinely needs are `pub` here
//! (`ai`, `state`, `config`, `router`, `mock_integrations`) — everything
//! else stays exactly as private as it was under `main.rs`. `AppState`'s
//! own `pub` methods (already fully `pub` before this split, e.g.
//! `create_library`, `refresh_default_bible_translations`) are callable
//! from any crate that has the `AppState` type, with no need to import
//! the private submodule whose `impl AppState` block happens to define
//! them — so widening `state` itself to `pub mod` did not require also
//! widening `state::bible`, `state::presentations`, etc.
//!
//! Clippy suppressions below are carried over VERBATIM from `main.rs`
//! (that file, pre-split, held the entire module tree) — see the
//! original comment block there (still present, trimmed) for the
//! per-group rationale.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::assigning_clones,
    clippy::uninlined_format_args,
    clippy::option_as_ref_cloned,
    clippy::unnecessary_lazy_evaluations,
    clippy::cast_possible_truncation,
    clippy::map_unwrap_or,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value,
    clippy::items_after_statements,
    clippy::manual_let_else,
    clippy::redundant_closure_for_method_calls,
    clippy::doc_markdown,
    clippy::match_same_arms,
    clippy::borrow_deref_ref,
    clippy::needless_borrow,
    clippy::explicit_auto_deref,
    clippy::wildcard_imports,
    clippy::struct_field_names,
    clippy::semicolon_if_nothing_returned,
    clippy::redundant_closure,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::unreadable_literal,
    clippy::if_not_else,
    clippy::manual_string_new,
    clippy::ignored_unit_patterns,
    clippy::large_enum_variant,
    clippy::field_reassign_with_default,
    clippy::question_mark,
    clippy::option_as_ref_deref,
    clippy::unused_self,
    clippy::derivable_impls,
    clippy::trivially_copy_pass_by_ref,
    clippy::unnecessary_semicolon,
    clippy::unnecessary_wraps,
    clippy::unwrap_or_default,
    clippy::absurd_extreme_comparisons,
    clippy::extend_with_drain,
    clippy::bool_to_int_with_if,
    clippy::ptr_arg,
    clippy::unnecessary_map_or
)]

mod ableset;
pub mod ai;
mod android_stage;
mod companion;
pub mod config;
mod live;
#[cfg(feature = "mock-integrations")]
pub mod mock_integrations;
mod osc;
mod resolume;
pub mod router;
mod stage_connections;
pub mod state;
mod turn;
mod ui;

#[cfg(test)]
mod tests {
    use crate::config::DEFAULT_SERVER_PORT;

    #[test]
    fn default_port_is_number() {
        assert_eq!(DEFAULT_SERVER_PORT, 80);
    }
}
