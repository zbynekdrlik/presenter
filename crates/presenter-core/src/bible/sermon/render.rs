//! The text and the label of a sermon item after its edits (#851 spec §7.3):
//!
//! - an end cut on the last verse of a run is `16a`, its trailing `, ; : – —`
//!   goes; a start cut on the first verse of a run is `16b`, original case;
//! - a cut the label cannot express (an inner verse, the inner edge of a run's
//!   first or last verse, one verse cut at both ends) gets a leading `… `
//!   and/or a trailing ` …`, never a trailing ellipsis after `. ! ?`;
//! - a gap with kept words on both sides reads ` … `;
//! - a wholly struck verse is omitted and splits the label (`5-7, 9`);
//! - an emphasized token is upper case ("vieru" → "VIERU").

use super::label::{verse_suffix, VerseMark};
use super::{token_texts, ItemEdits, SourceVerse, VerseEdits};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One verse as it goes on the wall. The slide line stays
/// `"{verse}. {text}"`: the integer verse prefixes it, the label carries the
/// letter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedVerse {
    pub verse: u16,
    pub text: String,
}

/// An item after its edits. `verses` is empty when every token is struck (the
/// item produces no slides); `suffix` then names the whole source range, so
/// the editor can still label the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedItem {
    pub verses: Vec<RenderedVerse>,
    pub suffix: String,
}

impl RenderedItem {
    pub fn is_empty(&self) -> bool {
        self.verses.is_empty()
    }

    /// `"1. text\n2. text"`: the slides' line format.
    pub fn numbered_text(&self) -> String {
        self.verses
            .iter()
            .map(|verse| format!("{}. {}", verse.verse, verse.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const END_CUT_TRIM: &[char] = &[',', ';', ':', '–', '—'];

/// One verse with its kept tokens, before its place in a run is known.
struct KeptVerse {
    verse: u16,
    start_cut: bool,
    end_cut: bool,
    body: String,
}

/// The item's verses and label suffix after `edits` (normalized or not: an
/// index past the verse's end is simply never matched).
pub fn render_item(source: &[SourceVerse], edits: &ItemEdits) -> RenderedItem {
    let kept: Vec<KeptVerse> = source
        .iter()
        .filter_map(|verse| keep_verse(verse, edits.verse(verse.verse)))
        .collect();
    if kept.is_empty() {
        let marks: Vec<VerseMark> = source.iter().map(|v| VerseMark::plain(v.verse)).collect();
        return RenderedItem {
            verses: Vec::new(),
            suffix: verse_suffix(&marks),
        };
    }
    let marks = label_marks(&kept);
    let verses = kept
        .iter()
        .zip(&marks)
        .map(|(verse, mark)| RenderedVerse {
            verse: verse.verse,
            text: finish_text(verse, mark),
        })
        .collect();
    RenderedItem {
        verses,
        suffix: verse_suffix(&marks),
    }
}

fn keep_verse(source: &SourceVerse, edits: Option<&VerseEdits>) -> Option<KeptVerse> {
    let tokens = token_texts(&source.text);
    let deleted: BTreeSet<u16> = edits
        .map(|e| e.deleted.iter().copied().collect())
        .unwrap_or_default();
    let emphasized: BTreeSet<u16> = edits
        .map(|e| e.emphasized.iter().copied().collect())
        .unwrap_or_default();
    let mut body = String::new();
    let mut gap = false;
    for (index, token) in tokens.iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        if deleted.contains(&index) {
            gap = !body.is_empty();
            continue;
        }
        if !body.is_empty() {
            body.push_str(if gap { " … " } else { " " });
        }
        if emphasized.contains(&index) {
            body.push_str(&token.to_uppercase());
        } else {
            body.push_str(token);
        }
        gap = false;
    }
    if body.is_empty() {
        return None;
    }
    let last = u16::try_from(tokens.len().saturating_sub(1)).unwrap_or(u16::MAX);
    Some(KeptVerse {
        verse: source.verse,
        start_cut: deleted.contains(&0),
        end_cut: deleted.contains(&last),
        body,
    })
}

/// Which cuts the label can express: `b` on the first verse of a run, `a` on
/// the last; nothing on a single verse cut at both ends.
fn label_marks(kept: &[KeptVerse]) -> Vec<VerseMark> {
    let follows = |a: &KeptVerse, b: &KeptVerse| a.verse.checked_add(1) == Some(b.verse);
    kept.iter()
        .enumerate()
        .map(|(index, verse)| {
            let first = index
                .checked_sub(1)
                .and_then(|previous| kept.get(previous))
                .is_none_or(|previous| !follows(previous, verse));
            let last = kept.get(index + 1).is_none_or(|next| !follows(verse, next));
            if first && last && verse.start_cut && verse.end_cut {
                return VerseMark::plain(verse.verse);
            }
            VerseMark {
                verse: verse.verse,
                start_cut: first && verse.start_cut,
                end_cut: last && verse.end_cut,
            }
        })
        .collect()
}

fn finish_text(verse: &KeptVerse, mark: &VerseMark) -> String {
    let mut text = if verse.end_cut {
        trim_end_cut(&verse.body)
    } else {
        verse.body.clone()
    };
    if verse.start_cut && !mark.start_cut {
        text = format!("… {text}");
    }
    if verse.end_cut && !mark.end_cut && !text.ends_with(['.', '!', '?']) {
        text = format!("{text} …");
    }
    text
}

/// The body without its trailing `, ; : – —` (and the spaces they leave);
/// unchanged when nothing else would remain.
fn trim_end_cut(body: &str) -> String {
    let mut text = body;
    loop {
        let trimmed = text.trim_end_matches(END_CUT_TRIM).trim_end();
        if trimmed.len() == text.len() {
            break;
        }
        text = trimmed;
    }
    if text.is_empty() {
        body.to_string()
    } else {
        text.to_string()
    }
}
