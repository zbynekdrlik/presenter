/// Auto-break a single-line string longer than `threshold` characters at the
/// rightmost space that keeps line 1 within the threshold (tail-break).
///
/// Returns the input unchanged when:
/// - the text already contains a newline (respect author-chosen breaks),
/// - `chars().count()` is `<= threshold`,
/// - no space produces a valid split (e.g. pathological single-word line).
///
/// Takes ownership so the common unchanged-path returns the input without
/// reallocation. Uses Unicode character counts, not bytes — safe for
/// Slovak/Czech diacritics.
///
/// Note: only line 1 is guaranteed to be within the threshold. On inputs with
/// a very long tail (e.g. `"A B very-long-single-word-of-35-chars"`), line 2
/// can exceed the threshold. Real worship lyrics do not hit this shape; it is
/// documented so callers know not to rely on line-2 length.
pub fn break_if_long(text: String, threshold: usize) -> String {
    if text.contains('\n') {
        return text;
    }
    if text.chars().count() <= threshold {
        return text;
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
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, threshold: usize) -> String {
        break_if_long(text.to_string(), threshold)
    }

    #[test]
    fn short_ascii_line_unchanged() {
        assert_eq!(run("Short line", 26), "Short line");
    }

    #[test]
    fn exactly_threshold_unchanged() {
        let s = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(s.chars().count(), 26);
        assert_eq!(run(s, 26), s);
    }

    #[test]
    fn breaks_at_rightmost_space_under_threshold() {
        let input = "Nad všetkých vyvyšený bude Pán";
        assert_eq!(input.chars().count(), 30);
        let out = run(input, 26);
        assert_eq!(out, "Nad všetkých vyvyšený bude\nPán");
    }

    #[test]
    fn multi_line_input_unchanged() {
        let input = "Line one\nLine two that is long enough";
        assert_eq!(run(input, 26), input);
    }

    #[test]
    fn single_word_longer_than_threshold_unchanged() {
        let input = "Supercalifragilisticexpialidocious";
        assert_eq!(input.chars().count(), 34);
        assert_eq!(run(input, 26), input);
    }

    #[test]
    fn diacritics_counted_as_one_char_each() {
        let input = "Pán je môj pastier som v bezp";
        assert_eq!(input.chars().count(), 29);
        let out = run(input, 26);
        assert_eq!(out, "Pán je môj pastier som v\nbezp");
    }

    #[test]
    fn empty_string_unchanged() {
        assert_eq!(run("", 26), "");
    }

    #[test]
    fn only_whitespace_unchanged() {
        assert_eq!(run("   ", 26), "   ");
    }

    #[test]
    fn long_tail_after_early_space() {
        // Documents: line 2 is NOT bounded by threshold. Line 1 ≤ 26 is guaranteed.
        // Spaces at positions 2 and 4; rightmost with prefix ≤ 26 is after "B".
        let input = "A B very-long-single-word-of-35-chr";
        assert!(input.chars().count() > 26);
        let out = run(input, 26);
        assert_eq!(out, "A B\nvery-long-single-word-of-35-chr");
    }
}
