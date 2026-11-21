//! Interpreter display areas (#852 spec §8.2). S1b ships the šepkár tips
//! area; C3-9 the EN transcript column; C5a adds the SK rows, the Bible area
//! and the status-bar controls next to them.
mod tips;
mod transcript;

pub use tips::{TipsArea, TipsModel};
pub use transcript::{TranscriptColumn, TranscriptModel};
