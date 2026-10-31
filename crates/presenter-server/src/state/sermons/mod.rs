//! Sermon preparation on the server (#851 spec §7). A write loads the sermon,
//! computes the new item list in memory, generates the slides from the items
//! (`generate.rs`) and commits items + slides in ONE repository transaction
//! guarded by the expected revision (`commit.rs`): a stale write is
//! `RepositoryError::Conflict` → 409 and is never half-applied. Then
//! `LiveEvent::BibleSlidesChanged` is published and one INFO
//! `presenter::bible::sermon` line is logged. Item text is a snapshot, so a
//! Bible re-import never changes a sermon (spec §5.7).

mod commit;
mod generate;

use chrono::NaiveDate;

/// Who wrote (spec §13): the Cloudflare Access e-mail when the request carried
/// one, and the client IP by the #793 rule. Both are labels, never
/// permissions (spec §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonOrigin {
    pub(crate) author: Option<String>,
    pub(crate) client_ip: String,
}

impl SermonOrigin {
    /// A write the server makes on its own (the character-limit pass).
    pub(crate) fn system() -> Self {
        Self {
            author: None,
            client_ip: "system".to_string(),
        }
    }
}

/// The `op` field of the `presenter::bible::sermon` line (spec §13). A unit
/// adds its variant together with the write that constructs it (`dead_code`
/// under `-D warnings` otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SermonOp {
    Regenerate,
}

impl SermonOp {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Regenerate => "regenerate",
        }
    }
}

/// The server-local date: "today" for `/bible/sermons/today`, the default
/// service date and the character-limit pass. The only clock read of the
/// sermon code; everything below it takes the date as a parameter.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests;
