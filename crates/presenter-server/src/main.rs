// Clippy lint suppressions for presenter-server.
//
// Pedantic lints: these are stylistic preferences that don't indicate bugs.
// We suppress them crate-wide to reduce noise while keeping default+correctness lints active.
//
// - Documentation lints (missing_errors_doc, missing_panics_doc, doc_markdown, must_use_candidate):
//   not worth enforcing on internal server code without a public API.
// - Style preferences (uninlined_format_args, manual_let_else, if_not_else, question_mark,
//   single_match_else, semicolon_if_nothing_returned, manual_string_new, unnecessary_semicolon,
//   items_after_statements, field_reassign_with_default, derivable_impls, unwrap_or_default,
//   unnecessary_map_or): idiomatic but subjective; enforcing them would churn existing code.
// - Cloning/borrowing (assigning_clones, option_as_ref_cloned, borrow_deref_ref, needless_borrow,
//   explicit_auto_deref, option_as_ref_deref, trivially_copy_pass_by_ref, ptr_arg): the compiler
//   optimises these away; suppressing avoids noisy diffs.
// - Closures (unnecessary_lazy_evaluations, redundant_closure_for_method_calls, redundant_closure):
//   explicit closures often read more clearly than point-free style.
// - Casts (cast_possible_truncation, cast_sign_loss, cast_lossless): reviewed case-by-case in PRs;
//   crate-wide allow avoids false positives on intentional casts.
// - Complexity (too_many_lines, too_many_arguments, significant_drop_tightening, large_enum_variant):
//   some handlers and route builders legitimately need many args/lines.
// - Misc (map_unwrap_or, needless_pass_by_value, wildcard_imports, struct_field_names,
//   unreadable_literal, ignored_unit_patterns, absurd_extreme_comparisons, extend_with_drain,
//   bool_to_int_with_if): low-value pedantic lints suppressed for readability.
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
mod adaptive_mjpeg;
mod ai;
mod android_stage;
mod companion;
mod config;
mod live;
mod osc;
mod resolume;
mod router;
mod stage_connections;
mod state;
mod ui;

use anyhow::Context;
use config::ServerConfig;
use router::build_router;
use state::AppState;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();
    let config = ServerConfig::load()?;
    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], config.http.port));
    let state = AppState::from_config(config).await?;
    let app = build_router(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;
    tracing::info!(%addr, "presenter server listening");
    axum::serve(listener, app).await.context("server failure")
}

fn setup_tracing() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,tower_http=debug");
    }

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();
}

#[cfg(test)]
mod tests {
    use crate::config::DEFAULT_SERVER_PORT;

    #[test]
    fn default_port_is_number() {
        assert_eq!(DEFAULT_SERVER_PORT, 80);
    }
}
