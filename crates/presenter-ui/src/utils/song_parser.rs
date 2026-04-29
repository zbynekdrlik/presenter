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
        let trimmed = raw_line.trim();

        if is_metadata_line(trimmed) {
            continue;
        }

        if is_group_line(trimmed) {
            flush_slide(&mut slides, &mut current_group, &mut current_main);
            current_group = Some(trimmed.to_string());
            continue;
        }

        if trimmed.is_empty() {
            // Only flush if we've accumulated content; otherwise ignore.
            if current_group.is_some() || !current_main.is_empty() {
                flush_slide(&mut slides, &mut current_group, &mut current_main);
            }
            continue;
        }

        current_main.push(raw_line.to_string());
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
    let line_lower = line.to_lowercase();
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
    let lower = line.to_lowercase();
    if lower.starts_with("title:") {
        return true;
    }
    // ^B / ^A / ^C etc. — exactly two chars: caret + uppercase ASCII letter.
    let bytes = line.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'^' && bytes[1].is_ascii_uppercase() {
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
            assert!(
                !s.main.to_lowercase().contains("title:")
                    && !s.main.to_lowercase().contains("misc ")
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
}
