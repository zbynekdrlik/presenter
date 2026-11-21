//! The guest's live transcript through Soniox's real-time WebSocket API
//! (#852 C3, spec §22, plan `2026-11-14-listen-c3-soniox.md`): `wire` = the
//! documented JSON (config, control, responses, error classes), `config` =
//! the env settings read once at startup, `machine` = the sans-I/O session
//! machine (C3-4), `conn` = one WebSocket connection as a task and
//! `session` = the provider task that drives the machine (C3-5).

pub(crate) mod config;
pub(crate) mod conn;
pub(crate) mod machine;
pub(crate) mod session;
#[cfg(test)]
mod tests;
pub(crate) mod wire;
