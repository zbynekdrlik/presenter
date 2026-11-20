//! The guest's live transcript through Soniox's real-time WebSocket API
//! (#852 C3, spec §22, plan `2026-11-14-listen-c3-soniox.md`): `wire` = the
//! documented JSON (config, control, responses, error classes), `config` =
//! the env settings read once at startup. The session machine, the
//! connection and the provider task follow in later lanes (C3-4, C3-5).

pub(crate) mod config;
pub(crate) mod wire;
