/// Auto-break a single-line string longer than `threshold` characters at the
/// rightmost space that keeps line 1 within the threshold (tail-break).
///
/// Returns the input unchanged when:
/// - the text already contains a newline (respect author-chosen breaks),
/// - `chars().count()` is `<= threshold`,
/// - no space produces a valid split (e.g. pathological single-word line).
///
/// Uses Unicode character counts, not bytes — safe for Slovak/Czech diacritics.
pub fn break_if_long(text: &str, threshold: usize) -> String {
    if text.contains('\n') {
        return text.to_string();
    }
    if text.chars().count() <= threshold {
        return text.to_string();
    }

    let mut best_split: Option<usize> = None;
    let mut char_count: usize = 0;
    for (byte_idx, ch) in text.char_indices() {
        if ch == ' ' && char_count <= threshold {
            best_split = Some(byte_idx);
        }
        char_count += 1;
    }

    match best_split {
        Some(i) => {
            let (head, tail) = text.split_at(i);
            let tail_trimmed = tail.strip_prefix(' ').unwrap_or(tail);
            format!("{head}\n{tail_trimmed}")
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ascii_line_unchanged() {
        assert_eq!(break_if_long("Short line", 26), "Short line");
    }

    #[test]
    fn exactly_threshold_unchanged() {
        let s = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(s.chars().count(), 26);
        assert_eq!(break_if_long(s, 26), s);
    }

    #[test]
    fn breaks_at_rightmost_space_under_threshold() {
        let input = "Nad všetkých vyvyšený bude Pán";
        assert_eq!(input.chars().count(), 30);
        let out = break_if_long(input, 26);
        assert_eq!(out, "Nad všetkých vyvyšený bude\nPán");
    }

    #[test]
    fn multi_line_input_unchanged() {
        let input = "Line one\nLine two that is long enough";
        assert_eq!(break_if_long(input, 26), input);
    }

    #[test]
    fn single_word_longer_than_threshold_unchanged() {
        let input = "Supercalifragilisticexpialidocious";
        assert_eq!(input.chars().count(), 34);
        assert_eq!(break_if_long(input, 26), input);
    }

    #[test]
    fn diacritics_counted_as_one_char_each() {
        let input = "Pán je môj pastier som v bezp";
        assert_eq!(input.chars().count(), 29);
        let out = break_if_long(input, 26);
        assert_eq!(out, "Pán je môj pastier som v\nbezp");
    }

    #[test]
    fn empty_string_unchanged() {
        assert_eq!(break_if_long("", 26), "");
    }

    #[test]
    fn only_whitespace_unchanged() {
        assert_eq!(break_if_long("   ", 26), "   ");
    }
}
