//! Persistence layer for Presenter.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::assigning_clones,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::map_unwrap_or,
    clippy::unnecessary_mut_passed,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::redundant_closure,
    clippy::manual_let_else,
    clippy::clone_on_copy,
    clippy::unnecessary_wraps,
    clippy::match_same_arms,
    clippy::needless_pass_by_value
)]

pub mod audit;
pub mod entities;
mod repository;

pub use audit::{ResolumePushAuditEntry, SettingsAuditEntry, SettingsAuditSource};
pub use repository::{DatabaseSettings, Repository};
