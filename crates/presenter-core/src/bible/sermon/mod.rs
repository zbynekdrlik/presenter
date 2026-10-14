//! Sermon preparation core (#851 spec §7.2–§7.3), shared by the server (slide
//! generation) and the WASM editor (instant display), so the two never
//! disagree. A sermon item is one contiguous verse range of one chapter; its
//! text is the database text snapshotted when the item was added
//! ([`SourceVerse`]). The preacher's only edits are struck tokens and
//! emphasized tokens per verse (ROZHODNUTÉ Q2: no rewording). Everything that
//! reaches the wall derives from [`render_item`], and the reference label
//! (`16a`, `16b-18a`, `5-7, 9`) is computed, never typed.
//!
//! Consumers import `presenter_core::bible::sermon::…` (no root re-export).

mod dates;
mod edits;
mod label;
mod render;
mod tokens;

pub use dates::{default_sermon_title, default_service_date};
pub use edits::{normalize_edits, retain_unchanged_edits, EditError};
pub use label::{item_label, translation_short_code, verse_suffix, VerseMark};
pub use render::{render_item, RenderedItem, RenderedVerse};
pub use tokens::{token_texts, tokenize, Token};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most verses one sermon item may hold (spec §7.1, decided default 13).
pub const MAX_ITEM_VERSES: u16 = 60;

/// One verse of an item's snapshot (`source_json`: `[{"verse":1,"text":"…"}]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceVerse {
    pub verse: u16,
    pub text: String,
}

/// The edits of one verse: token indices into [`tokenize`] of its source
/// text, sorted and deduplicated by [`normalize_edits`]. A struck token is
/// never emphasized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerseEdits {
    #[serde(default)]
    pub deleted: Vec<u16>,
    #[serde(default)]
    pub emphasized: Vec<u16>,
}

/// An item's edits keyed by verse number
/// (`edits_json`: `{"1":{"deleted":[7,8],"emphasized":[0]}}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemEdits(pub BTreeMap<u16, VerseEdits>);

impl ItemEdits {
    /// The edits of `verse`; `None` when the verse is untouched.
    pub fn verse(&self, verse: u16) -> Option<&VerseEdits> {
        self.0.get(&verse)
    }
}

#[cfg(test)]
mod tests;
