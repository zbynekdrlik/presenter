//! Song-text parser for the clipboard-paste "create presentation" flow.
//!
//! Splits a pasted song into one [`SlideInput`] per section. Triggers:
//! - Metadata lines (`Title:`, `Misc N`, `^[A-Z]`) → filter out.
//! - Group lines (`Verse N`, `Chorus`, `Bridge`, …) → flush current slide,
//!   start a new one with this line as the group name.
//! - Blank lines → flush current slide if non-empty.
//! - Anything else → append to the current slide's main text.
//!
//! See `docs/superpowers/specs/2026-04-29-clipboard-import-section-split-design.md`.

use crate::api::presentations::SlideInput;

/// Parse a pasted song into one [`SlideInput`] per section.
pub fn parse_song_text(text: &str) -> Vec<SlideInput> {
    let mut slides: Vec<SlideInput> = Vec::new();
    let mut current_group: Option<String> = None;
    let mut current_main: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        // Originally-blank lines (whitespace only) flush the current slide
        // — even if sanitization would later collapse a non-blank line to
        // empty, that's a different signal.
        if raw_line.trim().is_empty() {
            if current_group.is_some() || !current_main.is_empty() {
                flush_slide(&mut slides, &mut current_group, &mut current_main);
            }
            continue;
        }

        // Strip Unicode control + zero-width chars so stray bytes from any
        // pasted source don't pollute slides.
        let sanitized = sanitize_line(raw_line);
        let trimmed = sanitized.trim();

        // If sanitization left only whitespace, the line was entirely a
        // stray control byte — skip silently without flushing.
        if trimmed.is_empty() {
            continue;
        }

        if is_metadata_line(trimmed) {
            continue;
        }

        if is_group_line(trimmed) {
            flush_slide(&mut slides, &mut current_group, &mut current_main);
            current_group = Some(trimmed.to_string());
            continue;
        }

        // Push the sanitized line (NOT trimmed) so leading indentation
        // on indented lyric lines is preserved.
        current_main.push(sanitized);
    }

    flush_slide(&mut slides, &mut current_group, &mut current_main);
    slides
}

fn flush_slide(
    slides: &mut Vec<SlideInput>,
    group: &mut Option<String>,
    main_lines: &mut Vec<String>,
) {
    let main = main_lines.join("\n").trim_end().to_string();
    let took_group = group.take();
    main_lines.clear();
    if main.is_empty() && took_group.is_none() {
        return;
    }
    slides.push(SlideInput {
        main,
        translation: None,
        stage: None,
        group: took_group,
    });
}

/// Recognised section/group headings (the ProPresenter-style group names).
/// Accepts optional trailing digits / whitespace (e.g. `Verse 2`, `Chorus`).
/// Input must already be trimmed (the only call site trims before calling).
const GROUP_PATTERNS: &[&str] = &[
    "verse",
    "chorus",
    "bridge",
    "intro",
    "outro",
    "pre-chorus",
    "prechorus",
    "tag",
    "interlude",
    "refrain",
    "hook",
    "coda",
    "ending",
    "instrumental",
];

fn is_group_line(line: &str) -> bool {
    let line_lower = line.to_ascii_lowercase();
    GROUP_PATTERNS.iter().any(|pattern| {
        line_lower.strip_prefix(pattern).is_some_and(|rest| {
            let rest = rest.trim();
            rest.is_empty()
                || rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_whitespace())
        })
    })
}

/// ProPresenter-style metadata / control markers we want to silently drop:
/// - `Title:` prefix (case-insensitive)
/// - `Misc <digits>` (case-insensitive)
/// - ProPresenter control marker `^X` (exactly two bytes: caret + one ASCII uppercase letter; `^BB` is NOT filtered)
fn is_metadata_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    // Cheapest check first: ^B / ^A / ^C etc. — exactly two chars,
    // caret + uppercase ASCII letter. No allocation needed.
    let bytes = line.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'^' && bytes[1].is_ascii_uppercase() {
        return true;
    }
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("title:") {
        return true;
    }
    // "Misc 1", "MISC 12", etc.
    if let Some(rest) = lower.strip_prefix("misc ") {
        if !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_whitespace())
        {
            return true;
        }
    }
    false
}

/// Strip Unicode control chars (Cc category) and zero-width chars (BOM,
/// U+200B–200D) from a line. Used to clean lyric lines before pushing
/// them onto a slide so stray bytes from any pasted source (web pages,
/// Word, ProPresenter) don't render as a tofu/square character on stage.
fn sanitize_line(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control() && !is_zero_width(*c))
        .collect()
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}')
}

/// Extract the song title from the first `Title:` line in the input.
///
/// Returns the trimmed content after the colon, with a leading 1-3 digit
/// numeric prefix padded to 3 digits via [`pad_title_number`]. Returns
/// [`None`] if no `Title:` line is found.
pub fn extract_title(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = trimmed.get(..6)?;
        if !prefix.eq_ignore_ascii_case("title:") {
            return None;
        }
        Some(pad_title_number(trimmed[6..].trim()))
    })
}

/// Word-aware greedy wrapping per `\n`-separated line, so every line in
/// every slide's `main` fits within `line_limit` codepoints. Lines that
/// already fit pass through unchanged. Single words longer than
/// `line_limit` are hard-broken at the limit boundary. `line_limit == 0`
/// is treated as "no wrapping" (defensive — keeps signature stable if
/// the operator clears the setting).
/// Runs of whitespace within a line are collapsed to single spaces.
pub fn wrap_long_lines(slides: Vec<SlideInput>, line_limit: usize) -> Vec<SlideInput> {
    if line_limit == 0 {
        return slides;
    }
    slides
        .into_iter()
        .map(|slide| {
            let wrapped: Vec<String> = slide
                .main
                .split('\n')
                .flat_map(|line| wrap_one_line(line, line_limit))
                .collect();
            SlideInput {
                main: wrapped.join("\n"),
                ..slide
            }
        })
        .collect()
}

/// Start a fresh wrapped line with `word`. If the word fits within
/// `limit`, push it onto `current`. Otherwise hard-break it: push
/// complete `limit`-sized pieces to `out` and leave the trailing
/// remainder in `current` so subsequent words can keep accumulating.
fn start_new_line(out: &mut Vec<String>, current: &mut String, word: &str, limit: usize) {
    if word.chars().count() > limit {
        hard_break_into(out, current, word, limit);
    } else {
        current.push_str(word);
    }
}

fn wrap_one_line(line: &str, limit: usize) -> Vec<String> {
    if line.chars().count() <= limit {
        return vec![line.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        let word_len = word.chars().count();
        if current.is_empty() {
            start_new_line(&mut out, &mut current, word, limit);
        } else if current.chars().count() + 1 + word_len <= limit {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            start_new_line(&mut out, &mut current, word, limit);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Hard-break an oversize word at codepoint boundaries. Pushes complete
/// `limit`-codepoint pieces to `out` and leaves the trailing remainder
/// in `current` so subsequent words can still accumulate onto the same
/// line.
fn hard_break_into(out: &mut Vec<String>, current: &mut String, word: &str, limit: usize) {
    let mut piece_start = 0;
    let mut chars_in_piece = 0;
    for (byte_idx, _) in word.char_indices() {
        if chars_in_piece == limit {
            out.push(word[piece_start..byte_idx].to_string());
            piece_start = byte_idx;
            chars_in_piece = 0;
        }
        chars_in_piece += 1;
    }
    if piece_start < word.len() {
        *current = word[piece_start..].to_string();
    }
}

/// Pad a leading 1-3 digit numeric prefix (followed by whitespace) to 3
/// digits with leading zeros so songs sort correctly.
///
/// - `"76 Arriba"`        → `"076 Arriba"`
/// - `"6 Foo"`            → `"006 Foo"`
/// - `"102 10000 Armád"`  → `"102 10000 Armád"`  (already 3 digits)
/// - `"1234 Bar"`         → `"1234 Bar"`          (4+ digits, untouched)
/// - `"Arriba"`           → `"Arriba"`            (no leading number)
/// - `"76Arriba"`         → `"76Arriba"`          (no whitespace separator)
fn pad_title_number(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(space_idx) = trimmed.find(char::is_whitespace) {
        let (prefix, rest) = trimmed.split_at(space_idx);
        if !prefix.is_empty() && prefix.len() <= 3 && prefix.chars().all(|c| c.is_ascii_digit()) {
            return format!("{prefix:0>3}{rest}");
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_no_slides() {
        assert!(parse_song_text("").is_empty());
        assert!(parse_song_text("   \n\n  ").is_empty());
    }

    #[test]
    fn user_pasted_song_splits_into_one_slide_per_section() {
        // Regression guard for issue #275.
        let input = "Title: 326 Všetko, čo v sebe držím \n\
            Misc 1\n\
            ^B\n\
            Verse 1\n\
            Všetko, čo v sebe držím: s túžbami, nádejami aj\n\
            s vysnívaným!\n\
            Ty ma chceš Pane... aj s tým všetkým, čo mám ja.\n\
            \n\
            Chorus\n\
            Mám Spasiteľa, žije vo mne, ó, ó,\n\
            chcem viac, chcem Teba spoznávať viac.\n\
            \n\
            Verse 2\n\
            Všetko, čo v sebe držím: s túžbami, nádejami aj\n\
            s vysnívaným!\n\
            \n\
            Interlude\n\
            \n\
            Outro\n\
            Nikto mi Ťa už nemôže vziať!\n\
            Misc 1\n\
            ^B\n";
        let slides = parse_song_text(input);
        let groups: Vec<Option<&str>> = slides.iter().map(|s| s.group.as_deref()).collect();
        assert_eq!(
            groups,
            vec![
                Some("Verse 1"),
                Some("Chorus"),
                Some("Verse 2"),
                Some("Interlude"),
                Some("Outro"),
            ],
            "expected one slide per section, no Title/Misc/^B leakage"
        );
        // No slide's main contains Title:, Misc, or ^B.
        for s in &slides {
            let main_lower = s.main.to_ascii_lowercase();
            assert!(
                !main_lower.contains("title:")
                    && !main_lower.contains("misc ")
                    && !s.main.contains("^B"),
                "slide leaked metadata: {:?}",
                s
            );
        }
        // Outro slide should contain only the Nikto... lyric (Misc/^B trailing
        // metadata filtered).
        let outro = slides
            .iter()
            .find(|s| s.group.as_deref() == Some("Outro"))
            .expect("outro slide present");
        assert_eq!(outro.main, "Nikto mi Ťa už nemôže vziať!");
    }

    #[test]
    fn group_line_in_middle_of_paragraph_starts_new_slide() {
        // No blank line between Verse and Chorus.
        let input = "Verse 1\nlyric a\nlyric b\nChorus\nlyric c";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric a\nlyric b");
        assert_eq!(slides[1].group.as_deref(), Some("Chorus"));
        assert_eq!(slides[1].main, "lyric c");
    }

    #[test]
    fn metadata_lines_are_filtered() {
        let input = "Title: My Song\n\nVerse 1\nlyric";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric");
    }

    #[test]
    fn caret_uppercase_filtered_but_caret_alone_is_text() {
        let s1 = parse_song_text("Verse 1\n^B\nlyric");
        // ^B filtered, slide has just "lyric"
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].main, "lyric");

        let s2 = parse_song_text("Verse 1\n^\nlyric");
        // Bare ^ is NOT filtered — treated as text.
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].main, "^\nlyric");
    }

    #[test]
    fn misc_n_filtered_but_miscellaneous_is_text() {
        let s1 = parse_song_text("Verse 1\nMisc 1\nlyric");
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].main, "lyric");

        let s2 = parse_song_text("Verse 1\nMiscellaneous things\nlyric");
        // "Miscellaneous" is NOT filtered — treated as text.
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].main, "Miscellaneous things\nlyric");
    }

    #[test]
    fn empty_interlude_emits_slide_with_empty_main() {
        let input = "Interlude\n\nChorus\nlyric";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].group.as_deref(), Some("Interlude"));
        assert_eq!(slides[0].main, "");
        assert_eq!(slides[1].group.as_deref(), Some("Chorus"));
        assert_eq!(slides[1].main, "lyric");
    }

    #[test]
    fn loose_paragraphs_without_groups_split_on_blank_lines() {
        let input = "Stanza 1\nline 2\n\nStanza 3\nline 4";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert!(slides[0].group.is_none());
        assert_eq!(slides[0].main, "Stanza 1\nline 2");
        assert!(slides[1].group.is_none());
        assert_eq!(slides[1].main, "Stanza 3\nline 4");
    }

    #[test]
    fn group_line_is_case_insensitive() {
        let input = "chorus\nlower\n\nCHORUS\nupper\n\nVerse 2\nnumbered";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].group.as_deref(), Some("chorus"));
        assert_eq!(slides[1].group.as_deref(), Some("CHORUS"));
        assert_eq!(slides[2].group.as_deref(), Some("Verse 2"));
    }

    #[test]
    fn lyric_indentation_preserved() {
        let input = "Verse 1\n  indented\n    more indent\nflush";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].main, "  indented\n    more indent\nflush");
    }

    #[test]
    fn extract_title_pads_two_digit_prefix() {
        assert_eq!(
            extract_title("Title: 76 Arriba\nMisc 1\n\nVerse 1\nlyric"),
            Some("076 Arriba".to_string())
        );
    }

    #[test]
    fn extract_title_pads_one_digit_prefix() {
        assert_eq!(extract_title("Title: 6 Foo"), Some("006 Foo".to_string()));
    }

    #[test]
    fn extract_title_leaves_three_digit_prefix_unchanged() {
        assert_eq!(
            extract_title("Title: 102 10000 Armád"),
            Some("102 10000 Armád".to_string())
        );
    }

    #[test]
    fn extract_title_leaves_four_digit_prefix_unchanged() {
        assert_eq!(
            extract_title("Title: 1234 Bar"),
            Some("1234 Bar".to_string())
        );
    }

    #[test]
    fn extract_title_no_leading_number_unchanged() {
        assert_eq!(extract_title("Title: Arriba"), Some("Arriba".to_string()));
    }

    #[test]
    fn extract_title_no_space_after_digits_unchanged() {
        // No whitespace separator → not a song-number prefix; leave alone.
        assert_eq!(
            extract_title("Title: 76Arriba"),
            Some("76Arriba".to_string())
        );
    }

    #[test]
    fn extract_title_returns_none_when_absent() {
        assert_eq!(extract_title("Verse 1\nlyric"), None);
    }

    #[test]
    fn extract_title_is_case_insensitive() {
        assert_eq!(extract_title("TITLE: foo"), Some("foo".to_string()));
        assert_eq!(extract_title("title: foo"), Some("foo".to_string()));
    }

    #[test]
    fn extract_title_strips_surrounding_whitespace() {
        assert_eq!(
            extract_title("Title:   foo bar  "),
            Some("foo bar".to_string())
        );
    }

    #[test]
    fn parse_song_text_strips_control_chars_from_lyric_lines() {
        // STX (\x02), BOM (\u{FEFF}), and ZWSP (\u{200B}) all stripped;
        // visible content survives.
        let input = "Verse 1\n\u{FEFF}lyric\u{200B} one\x02";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric one");
    }

    #[test]
    fn parse_song_text_skips_lines_that_become_empty_after_strip() {
        // A line that is ONLY a control byte should not produce a slide
        // and should not flush an in-progress section.
        let input = "Verse 1\nlyric a\n\x02\nlyric b";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1, "stray \\x02 line should not flush");
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric a\nlyric b");
    }

    #[test]
    fn wrap_long_lines_word_wraps_at_limit() {
        let slide = SlideInput {
            main: "Ak Boh je za mňa,  kto je proti mne".to_string(),
            translation: None,
            stage: None,
            group: Some("Bridge".to_string()),
        };
        let out = wrap_long_lines(vec![slide], 32);
        assert_eq!(out.len(), 1);
        // Should wrap to >=2 lines, each ≤ 32 chars.
        let lines: Vec<&str> = out[0].main.split('\n').collect();
        assert!(lines.len() >= 2, "expected wrapping, got: {:?}", lines);
        for line in &lines {
            assert!(
                line.chars().count() <= 32,
                "wrapped line '{line}' exceeds 32 chars: {}",
                line.chars().count()
            );
        }
        // The original group should be preserved.
        assert_eq!(out[0].group.as_deref(), Some("Bridge"));
    }

    #[test]
    fn wrap_long_lines_passes_through_short_lines() {
        let slide = SlideInput {
            main: "short line\nanother short".to_string(),
            translation: None,
            stage: None,
            group: Some("Verse 1".to_string()),
        };
        let out = wrap_long_lines(vec![slide.clone()], 32);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].main, slide.main);
    }

    #[test]
    fn wrap_long_lines_hard_breaks_oversize_word() {
        // Single 50-char word at limit 32 → 2 pieces (32 + 18).
        let big_word: String = "a".repeat(50);
        let slide = SlideInput {
            main: big_word,
            translation: None,
            stage: None,
            group: None,
        };
        let out = wrap_long_lines(vec![slide], 32);
        let lines: Vec<&str> = out[0].main.split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 32);
        assert_eq!(lines[1].chars().count(), 18);
    }

    #[test]
    fn wrap_long_lines_counts_unicode_codepoints_not_bytes() {
        // Slovak diacritics — each char is one codepoint (multi-byte UTF-8).
        // 30 'á' chars = 30 codepoints = 60 bytes. At limit 32, this should
        // pass through (≤ limit by codepoint), not wrap.
        let line: String = "á".repeat(30);
        let slide = SlideInput {
            main: line.clone(),
            translation: None,
            stage: None,
            group: None,
        };
        let out = wrap_long_lines(vec![slide], 32);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].main, line, "30 Slovak codepoints fit at limit 32");
    }
}
