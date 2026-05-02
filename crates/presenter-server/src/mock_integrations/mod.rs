//! In-process mock listeners for outbound integrations (Resolume HTTP,
//! AbleSet HTTP). Compiled only when the `mock-integrations` feature is
//! enabled — dev builds use this; prod builds omit it entirely.
//!
//! Closes issue #279: prevents dev from sending live commands to
//! production hardware. Submodules added in Tasks 3 (resolume) and 4
//! (ableset).

pub mod request_log;
