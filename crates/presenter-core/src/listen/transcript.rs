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
    /// Its first tagged token's language (Soniox language identification,
    /// #852 T1): the reader can hide the Slovak interpreter's words.
    pub language: Option<String>,
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

/// `text` opens with punctuation that belongs to the word before it.
fn attaches_to_previous(text: &str) -> bool {
    text.starts_with(['.', ',', ';', ':', '!', '?', '…', ')', '”', '"', '’', '\''])
}

fn session_ms(offset_ms: u64, ms: Option<u64>) -> Option<u64> {
    ms.map(|ms| offset_ms.saturating_add(ms))
}

#[derive(Debug, Default)]
struct OpenSentence {
    text: String,
    start_ms: Option<u64>,
    end_ms: u64,
    words: usize,
    language: Option<String>,
    /// The connection of the last final appended.
    connection: Option<u32>,
    /// The newest connection with a final in this sentence.
    newest: Option<u32>,
}

/// Turns token batches into closed sentences and a live tail. One per
/// transcription session: after [`flush`](Self::flush) the next session may
/// number its connections from the start again.
#[derive(Debug, Default)]
pub struct TranscriptAssembler {
    connection: u32,
    open: OpenSentence,
    /// The open sentence ends with sentence punctuation: the next final
    /// token decides (a new word closes it, a continuation like `5.13` not).
    pending_close: bool,
    /// The non-final tokens of the current connection's last batch.
    partial: String,
    partial_start_ms: Option<u64>,
    partial_language: Option<String>,
    /// The newest session ms seen; a token without times takes it.
    last_ms: u64,
    reported: TailText,
}

impl TranscriptAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, batch: TokenBatch) -> AssemblyOutput {
        let current = batch.connection >= self.connection;
        self.connection = self.connection.max(batch.connection);
        let mut closed = Vec::new();
        let mut partial = String::new();
        let mut partial_start_ms = None;
        let mut partial_language = None;
        for token in &batch.tokens {
            if token.is_translation || token.text.is_empty() {
                continue;
            }
            if token.text == END_MARKER || token.text == FIN_MARKER {
                // A late marker of a draining older connection never closes
                // a sentence that already holds a newer connection's words.
                let ours = self.open.newest.is_none_or(|c| c <= batch.connection);
                if token.is_final && ours {
                    self.close_into(&mut closed, batch.received_at);
                }
                continue;
            }
            if token.is_final {
                self.push_final(token, &batch, &mut closed);
            } else if current {
                if partial_start_ms.is_none() {
                    partial_start_ms = session_ms(batch.offset_ms, token.start_ms);
                }
                if partial_language.is_none() {
                    partial_language.clone_from(&token.language);
                }
                partial.push_str(&token.text);
            }
        }
        if current {
            self.partial = partial;
            self.partial_start_ms = partial_start_ms;
            self.partial_language = partial_language;
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

    /// The session ended: the open sentence closes, the tail empties and the
    /// connection numbering starts again.
    pub fn flush(&mut self, at: DateTime<Utc>) -> AssemblyOutput {
        let mut closed = Vec::new();
        self.close_into(&mut closed, at);
        self.clear_partial();
        self.connection = 0;
        self.last_ms = 0;
        AssemblyOutput {
            closed,
            tail: self.tail_change(),
        }
    }

    fn push_final(
        &mut self,
        token: &AsrToken,
        batch: &TokenBatch,
        closed: &mut Vec<ClosedSentence>,
    ) {
        let spaced = token.text.starts_with(char::is_whitespace)
            || self.open.text.ends_with(char::is_whitespace);
        // A connection's first token carries no leading space: a final of
        // another connection than the open sentence's last one starts a word.
        let other_connection = self.open.connection.is_some_and(|c| c != batch.connection)
            && !attaches_to_previous(&token.text);
        let starts_word = spaced || other_connection;
        if !self.open.text.is_empty()
            && starts_word
            && (self.pending_close || self.open.words >= MAX_SENTENCE_WORDS)
        {
            self.close_into(closed, batch.received_at);
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
        if !self.open.text.is_empty() && other_connection && !spaced {
            self.open.text.push(' ');
        }
        let start = session_ms(batch.offset_ms, token.start_ms)
            .or(session_ms(batch.offset_ms, token.end_ms))
            .unwrap_or(self.last_ms);
        let end = session_ms(batch.offset_ms, token.end_ms).unwrap_or(start);
        self.last_ms = self.last_ms.max(end);
        self.open.start_ms.get_or_insert(start);
        self.open.end_ms = self.open.end_ms.max(end);
        if self.open.language.is_none() {
            self.open.language.clone_from(&token.language);
        }
        self.open.connection = Some(batch.connection);
        self.open.newest = self.open.newest.max(Some(batch.connection));
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
        closed.push(ClosedSentence {
            text: text.to_owned(),
            start_ms: open.start_ms.unwrap_or(open.end_ms),
            end_ms: open.end_ms,
            closed_at: at,
            language: open.language,
        });
    }

    fn clear_partial(&mut self) {
        self.partial.clear();
        self.partial_start_ms = None;
        self.partial_language = None;
    }

    fn tail_change(&mut self) -> Option<TailText> {
        let mut partial_text = if self.open.text.is_empty() {
            self.partial.trim_start().to_owned()
        } else {
            self.partial.clone()
        };
        // The current connection's tail after another connection's finals.
        let other_connection = self.open.connection.is_some_and(|c| c != self.connection);
        if other_connection
            && !partial_text.is_empty()
            && !partial_text.starts_with(char::is_whitespace)
            && !attaches_to_previous(&partial_text)
            && !self.open.text.ends_with(char::is_whitespace)
        {
            partial_text.insert(0, ' ');
        }
        let tail = TailText {
            final_text: self.open.text.clone(),
            partial_text,
            start_ms: self.open.start_ms.or(self.partial_start_ms),
            language: self
                .open
                .language
                .clone()
                .or_else(|| self.partial_language.clone()),
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
