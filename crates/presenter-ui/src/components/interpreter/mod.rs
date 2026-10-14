//! Interpreter display areas (#852 spec §8.2). S1b ships the šepkár tips
//! area; C5a adds the transcript, the Bible area and the status-bar controls
//! next to it.
mod tips;

pub use tips::{TipsArea, TipsModel};
