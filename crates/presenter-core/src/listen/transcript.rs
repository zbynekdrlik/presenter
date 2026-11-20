//! Token → sentence assembly for the guest's live transcript (#852 C3,
//! spec §22.6, plan Decision 7). Pure: the Soniox session feeds it one
//! [`TokenBatch`] per response and publishes what [`AssemblyOutput`] reports.
//!
//! - Final tokens append to the open sentence; non-final tokens are the live
//!   tail, replaced by every batch of the current connection.
//! - Sub-word tokens join as written (their own leading spaces); the first
//!   token of a sentence is left-trimmed.
//! - A sentence closes at `. ! ? …` (a closing quote or bracket allowed) when
//!   the NEXT final token starts a new word, at [`MAX_SENTENCE_WORDS`] (at a
//!   word boundary), at [`END_MARKER`] / [`FIN_MARKER`] or at [`flush`]; a
//!   pause alone never closes one. Markers are never text.
//! - Token times are per connection; session ms = `offset_ms` + token ms.
//! - A batch of a connection older than the current one (a draining rollover
//!   connection) contributes its finals only.
//!
//! [`flush`]: TranscriptAssembler::flush

use chrono::{DateTime, Utc};

/// Soniox's endpoint marker (only with endpoint detection).
pub const END_MARKER: &str = "<end>";
/// Soniox's answer to a `finalize` request (the silence gate, a rollover).
pub const FIN_MARKER: &str = "<fin>";
/// Songplayer's cap: a sentence closes at the next word boundary after this.
pub const MAX_SENTENCE_WORDS: usize = 20;

/// One token as the provider sent it; times in ms of its connection.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AsrToken {
    pub text: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub confidence: f32,
    pub is_final: bool,
    pub language: Option<String>,
    /// A translation token (`translation_status: "translation"`): ignored.
    pub is_translation: bool,
}

/// One provider response.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenBatch {
    /// Increasing per session: a reconnect or a rollover opens the next one.
    pub connection: u32,
    /// The session ms of the connection's first audio byte.
    pub offset_ms: u64,
    pub tokens: Vec<AsrToken>,
    pub received_at: DateTime<Utc>,
}

/// A sentence that will not change any more; times are session ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSentence {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub closed_at: DateTime<Utc>,
    pub language: Option<String>,
}

/// The open sentence: its final words and the live rest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TailText {
    pub final_text: String,
    pub partial_text: String,
    pub start_ms: Option<u64>,
}

/// What one call changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssemblyOutput {
    pub closed: Vec<ClosedSentence>,
    /// `Some` = the tail changed (an empty tail = nothing open).
    pub tail: Option<TailText>,
}

/// `text` ends a sentence: `. ! ? …`, then optional closing quotes / `)`.
pub fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['”', '"', '’', '\'', ')'])
        .ends_with(['.', '!', '?', '…'])
}

fn session_ms(offset_ms: u64, ms: Option<u64>) -> Option<u64> {
    ms.map(|ms| offset_ms.saturating_add(ms))
}

#[derive(Debug, Default)]
struct OpenSentence {
    text: String,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
    words: usize,
    language: Option<String>,
}

/// Turns token batches into closed sentences and a live tail.
#[derive(Debug, Default)]
pub struct TranscriptAssembler {
    connection: u32,
    open: OpenSentence,
    /// The open sentence ends with sentence punctuation: the next final
    /// token decides (a new word closes it, a continuation like `5.13` not).
    pending_close: bool,
    partial: String,
    partial_start_ms: Option<u64>,
    reported: TailText,
}

impl TranscriptAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, batch: TokenBatch) -> AssemblyOutput {
        let current = batch.connection >= self.connection;
        if batch.connection > self.connection {
            self.connection = batch.connection;
        }
        let mut closed = Vec::new();
        let mut partial = String::new();
        let mut partial_start_ms = None;
        for token in &batch.tokens {
            if token.is_translation || token.text.is_empty() {
                continue;
            }
            if token.text == END_MARKER || token.text == FIN_MARKER {
                if token.is_final {
                    self.close_into(&mut closed, batch.received_at);
                }
                continue;
            }
            if token.is_final {
                self.push_final(token, batch.offset_ms, batch.received_at, &mut closed);
            } else if current {
                if partial_start_ms.is_none() {
                    partial_start_ms = session_ms(batch.offset_ms, token.start_ms);
                }
                partial.push_str(&token.text);
            }
        }
        if current {
            self.partial = partial;
            self.partial_start_ms = partial_start_ms;
        }
        AssemblyOutput {
            closed,
            tail: self.tail_change(),
        }
    }

    /// A connection ended (close, reconnect, rollover): its live tail goes
    /// (the resend recreates it), its finals stay in the open sentence.
    pub fn connection_ended(&mut self, connection: u32) -> AssemblyOutput {
        if connection >= self.connection {
            self.clear_partial();
        }
        AssemblyOutput {
            closed: Vec::new(),
            tail: self.tail_change(),
        }
    }

    /// The session ended: the open sentence closes, the tail empties.
    pub fn flush(&mut self, at: DateTime<Utc>) -> AssemblyOutput {
        let mut closed = Vec::new();
        self.close_into(&mut closed, at);
        self.clear_partial();
        AssemblyOutput {
            closed,
            tail: self.tail_change(),
        }
    }

    fn push_final(
        &mut self,
        token: &AsrToken,
        offset_ms: u64,
        at: DateTime<Utc>,
        closed: &mut Vec<ClosedSentence>,
    ) {
        let starts_word = token.text.starts_with(char::is_whitespace)
            || self.open.text.ends_with(char::is_whitespace);
        if !self.open.text.is_empty()
            && starts_word
            && (self.pending_close || self.open.words >= MAX_SENTENCE_WORDS)
        {
            self.close_into(closed, at);
        }
        let text = if self.open.text.is_empty() {
            token.text.trim_start()
        } else {
            token.text.as_str()
        };
        if text.is_empty() {
            return;
        }
        if (self.open.text.is_empty() || starts_word) && !text.trim().is_empty() {
            self.open.words += 1;
        }
        let start = session_ms(offset_ms, token.start_ms);
        let end = session_ms(offset_ms, token.end_ms).or(start);
        if self.open.start_ms.is_none() {
            self.open.start_ms = start.or(end);
        }
        if end.is_some() {
            self.open.end_ms = end;
        }
        if self.open.language.is_none() {
            self.open.language.clone_from(&token.language);
        }
        self.open.text.push_str(text);
        self.pending_close = ends_sentence(&self.open.text);
    }

    fn close_into(&mut self, closed: &mut Vec<ClosedSentence>, at: DateTime<Utc>) {
        let open = std::mem::take(&mut self.open);
        self.pending_close = false;
        let text = open.text.trim_end();
        if text.is_empty() {
            return;
        }
        let end_ms = open.end_ms.or(open.start_ms).unwrap_or(0);
        closed.push(ClosedSentence {
            text: text.to_owned(),
            start_ms: open.start_ms.unwrap_or(end_ms),
            end_ms,
            closed_at: at,
            language: open.language,
        });
    }

    fn clear_partial(&mut self) {
        self.partial.clear();
        self.partial_start_ms = None;
    }

    fn tail_change(&mut self) -> Option<TailText> {
        let partial_text = if self.open.text.is_empty() {
            self.partial.trim_start()
        } else {
            self.partial.as_str()
        };
        let tail = TailText {
            final_text: self.open.text.clone(),
            partial_text: partial_text.to_owned(),
            start_ms: self.open.start_ms.or(self.partial_start_ms),
        };
        if tail == self.reported {
            return None;
        }
        self.reported = tail.clone();
        Some(tail)
    }
}

#[cfg(test)]
mod tests;
