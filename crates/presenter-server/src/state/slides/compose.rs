//! Pure bible-slide composition: the live-mode composer (`compose_bible_slides`)
//! and the AI item-stream composer (`compose_bible_items_into_slides`), plus the
//! data types and private formatting helpers they share. No `AppState` access.

use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
use presenter_core::{BiblePassage, BibleTranslation, Slide, SlideContent, SlideGroup, SlideText};
use std::collections::{BTreeSet, HashMap};

/// Extracts the short translation code from a full translation code.
/// E.g., "eng-kjv" → "KJV", "sk-roh" → "ROH"
fn translation_short_code(code: &str) -> String {
    code.rsplit('-').next().unwrap_or(code).to_uppercase()
}

/// Typed input for the AI-facing bible composer. A stream of these is
/// produced by the LLM after it edits DB verses against the sermon text,
/// and the server composes slides out of the stream respecting the
/// character limit — same splitting rules as live mode.
#[derive(Debug, Clone)]
pub(crate) enum BibleItem {
    Verse {
        number: u32,
        text: String,
        book: String,
        chapter: u32,
        translation: String,
    },
    Emphasis {
        text: String,
    },
}

/// A slide produced by `compose_bible_items_into_slides`. Plain data —
/// the tool handler wraps it into `BiblePresentationSlide` for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComposedBibleSlide {
    pub main: String,
    pub main_reference: String,
}

/// Format a sorted set of verse numbers as a reference suffix.
///
/// - Empty set → "" (caller skips the reference entirely).
/// - Single verse → "17".
/// - Contiguous range → "17-20".
/// - Non-contiguous → "1, 3, 5" (flat comma-list, no mixed range syntax).
fn format_verse_range(verses: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = verses.iter().copied().collect();
    if v.is_empty() {
        return String::new();
    }
    let min = v[0];
    let max = v[v.len() - 1];
    let count = v.len() as u32;
    if min == max {
        format!("{}", min)
    } else if max - min + 1 == count {
        format!("{}-{}", min, max)
    } else {
        v.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Compose a stream of `BibleItem` into slides. Same greedy-packing rule
/// as `compose_bible_slides`: accumulate verses into one slide until the
/// next verse would overflow the character limit, then flush. Emphasis
/// items and translation/book/chapter changes force a slide break.
///
/// If a single verse item is longer than `character_limit`, it is emitted
/// as its own oversized slide — the validator's `MainExceedsCharacterLimit`
/// rule catches this downstream and the LLM sees a rule-keyed error.
pub(crate) fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedBibleSlide> {
    let limit = character_limit as usize;
    let mut slides: Vec<ComposedBibleSlide> = Vec::new();

    // Pass 1: collect every verse number per (book, chapter, translation)
    // group across the whole items[] stream. Slides flushed in pass 2 use
    // this group's full verse list for the reference label, so all slides
    // of one passage display the same reference (issue #292).
    let mut group_verses: HashMap<(String, u32, String), BTreeSet<u32>> = HashMap::new();
    for item in items {
        if let BibleItem::Verse {
            number,
            book,
            chapter,
            translation,
            ..
        } = item
        {
            group_verses
                .entry((book.clone(), *chapter, translation.clone()))
                .or_default()
                .insert(*number);
        }
    }

    // Accumulator for the current verse slide.
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_numbers: Vec<u32> = Vec::new();
    let mut cur_group: Option<(String, u32, String)> = None; // (book, chapter, translation)

    // Flush the current verse accumulator into a slide. The reference is
    // derived from the GROUP's full verse set (built in pass 1), not from
    // cur_numbers, so every slide of one passage displays the same label.
    //
    // Uses let-else pattern matching instead of expect()/unwrap() so there
    // is no panic path — if an invariant is ever broken by a future
    // refactor (e.g., group set without lines), the flush is a no-op
    // rather than a crash.
    let flush_verses =
        |slides: &mut Vec<ComposedBibleSlide>,
         lines: &mut Vec<String>,
         numbers: &mut Vec<u32>,
         group: &mut Option<(String, u32, String)>,
         group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>| {
            if lines.is_empty() {
                *group = None;
                return;
            }
            let Some((book, chapter, translation)) = group.take() else {
                lines.clear();
                numbers.clear();
                return;
            };
            if numbers.is_empty() {
                lines.clear();
                return;
            }
            let main = lines.join("\n");
            let reference = match group_verses.get(&(book.clone(), chapter, translation.clone())) {
                Some(verses) => format!(
                    "{} {}:{} ({})",
                    book,
                    chapter,
                    format_verse_range(verses),
                    translation
                ),
                None => String::new(),
            };
            slides.push(ComposedBibleSlide {
                main,
                main_reference: reference,
            });
            lines.clear();
            numbers.clear();
        };

    for item in items {
        match item {
            BibleItem::Emphasis { text } => {
                flush_verses(
                    &mut slides,
                    &mut cur_lines,
                    &mut cur_numbers,
                    &mut cur_group,
                    &group_verses,
                );
                slides.push(ComposedBibleSlide {
                    main: text.clone(),
                    main_reference: String::new(),
                });
            }
            BibleItem::Verse {
                number,
                text,
                book,
                chapter,
                translation,
            } => {
                // Translation / book / chapter change forces a slide break.
                if let Some((cur_book, cur_chapter, cur_tr)) = &cur_group {
                    if cur_book != book || cur_chapter != chapter || cur_tr != translation {
                        flush_verses(
                            &mut slides,
                            &mut cur_lines,
                            &mut cur_numbers,
                            &mut cur_group,
                            &group_verses,
                        );
                    }
                }

                let line = format!("{}. {}", number, text);
                let existing_len: usize = cur_lines.iter().map(String::len).sum();
                let prospective = if cur_lines.is_empty() {
                    line.len()
                } else {
                    // existing lines joined by "\n" = existing_len + (cur_lines.len() - 1)
                    // plus "\n" + new line = + 1 + line.len()
                    // total = existing_len + cur_lines.len() + line.len()
                    existing_len + cur_lines.len() + line.len()
                };

                if prospective > limit && !cur_lines.is_empty() {
                    flush_verses(
                        &mut slides,
                        &mut cur_lines,
                        &mut cur_numbers,
                        &mut cur_group,
                        &group_verses,
                    );
                }

                cur_lines.push(line);
                cur_numbers.push(*number);
                cur_group = Some((book.clone(), *chapter, translation.clone()));
            }
        }
    }

    flush_verses(
        &mut slides,
        &mut cur_lines,
        &mut cur_numbers,
        &mut cur_group,
        &group_verses,
    );
    slides
}

pub(crate) fn compose_bible_slides(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[BiblePassage],
    secondary_lookup: &HashMap<u16, BiblePassage>,
    character_limit: u32,
    full_verse_start: u16,
    full_verse_end: u16,
) -> anyhow::Result<Vec<Slide>> {
    let mut slides: Vec<Slide> = Vec::new();
    if main_passages.is_empty() {
        return Ok(slides);
    }

    let book = main_passages[0].reference.book.clone();
    let book_code = main_passages[0].reference.book_code.clone();
    let book_number = main_passages[0].reference.book_number;
    let chapter = main_passages[0].reference.chapter;

    // Build the full reference label that will appear on all slides (includes translation code)
    let main_short_code = translation_short_code(&main_translation.code);
    let full_reference_label = if full_verse_start == full_verse_end {
        format!(
            "{} {}:{} ({})",
            book, chapter, full_verse_start, main_short_code
        )
    } else {
        format!(
            "{} {}:{}-{} ({})",
            book, chapter, full_verse_start, full_verse_end, main_short_code
        )
    };

    // Build translation reference label if secondary translation is present
    let translation_reference_label = secondary_translation.map(|t| {
        let secondary_short_code = translation_short_code(&t.code);
        if full_verse_start == full_verse_end {
            format!(
                "{} {}:{} ({})",
                book, chapter, full_verse_start, secondary_short_code
            )
        } else {
            format!(
                "{} {}:{}-{} ({})",
                book, chapter, full_verse_start, full_verse_end, secondary_short_code
            )
        }
    });

    let mut current_main = String::new();
    let mut current_tr = String::new();
    let mut verses_meta: Vec<(u16, u16)> = Vec::new();

    let push_slide = |slides: &mut Vec<Slide>,
                      main: String,
                      tr: String,
                      verses: &[(u16, u16)]|
     -> anyhow::Result<()> {
        if main.trim().is_empty() {
            return Ok(());
        }
        let content = SlideContent::new(
            SlideText::new(&main)?,
            SlideText::new(&tr)?,
            SlideText::new(&full_reference_label)?,
            Some(SlideGroup::new(&full_reference_label)),
        );
        let metadata = SlideMetadata::new().with_bible(BibleSlideMetadata {
            translation_code: main_translation.code.clone(),
            secondary_translation_code: secondary_translation.map(|t| t.code.clone()),
            book: book.clone(),
            book_code: book_code.clone(),
            book_number,
            chapter,
            verses: verses
                .iter()
                .map(|(s, e)| BibleSlideVerseRef::new(*s, *e))
                .collect(),
            main_reference_label: Some(full_reference_label.clone()),
            translation_reference_label: translation_reference_label.clone(),
        });
        slides.push(Slide::new(slides.len() as u32, content).with_metadata(Some(metadata)));
        Ok(())
    };

    for p in main_passages {
        let label = format!("{}. ", p.reference.verse_start);
        let line = format!("{}{}", label, p.text);
        let prospective_len =
            current_main.len() + if current_main.is_empty() { 0 } else { 1 } + line.len();
        if prospective_len > character_limit as usize && !current_main.is_empty() {
            // flush
            push_slide(
                &mut slides,
                current_main.clone(),
                current_tr.clone(),
                &verses_meta,
            )?;
            current_main.clear();
            current_tr.clear();
            verses_meta.clear();
        }
        if !current_main.is_empty() {
            current_main.push('\n');
        }
        current_main.push_str(&line);
        // translation (secondary)
        if let Some(sec) = secondary_lookup.get(&p.reference.verse_start) {
            let tr_label = format!("{}. ", sec.reference.verse_start);
            let tr_line = format!("{}{}", tr_label, sec.text);
            if !current_tr.is_empty() {
                current_tr.push('\n');
            }
            current_tr.push_str(&tr_line);
        }
        verses_meta.push((p.reference.verse_start, p.reference.verse_end));
    }

    // final flush
    if !current_main.is_empty() {
        push_slide(&mut slides, current_main, current_tr, &verses_meta)?;
    }

    Ok(slides)
}
