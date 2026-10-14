//! Tokens of a verse (#851 spec §7.2): maximal runs of non-whitespace, so
//! "nevidíme," is one token with its punctuation. Imported text is
//! whitespace-normalized, so the kept tokens joined by one space reproduce
//! it, and a snapshot never changes, so token indices stay valid forever.

/// Byte span of one token in its verse text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
}

/// The tokens of `text`, in order.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        match (ch.is_whitespace(), start) {
            (true, Some(begin)) => {
                tokens.push(Token {
                    start: begin,
                    end: index,
                });
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(begin) = start {
        tokens.push(Token {
            start: begin,
            end: text.len(),
        });
    }
    tokens
}

/// The token strings of `text`, in order.
pub fn token_texts(text: &str) -> Vec<&str> {
    tokenize(text)
        .into_iter()
        .filter_map(|token| text.get(token.start..token.end))
        .collect()
}
