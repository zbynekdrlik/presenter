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
/// A verse is NEVER split mid-text (issue #394): consecutive `Verse` items that
/// share the same verse number are merged back into one whole verse, and a lone
/// verse longer than `character_limit` is kept WHOLE on its own slide — the
/// validator accepts such a lone oversized verse (autofit shrinks it for
/// display) rather than rejecting it.
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
    let mut acc = VerseAccumulator::default();

    for item in items {
        match item {
            BibleItem::Emphasis { text } => {
                acc.flush(&mut slides, &group_verses);
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
                if let Some((cur_book, cur_chapter, cur_tr)) = &acc.group {
                    if cur_book != book || cur_chapter != chapter || cur_tr != translation {
                        acc.flush(&mut slides, &group_verses);
                    }
                }

                // Issue #394: the LLM's oversized-single-verse recovery path may
                // emit ONE logical verse as several consecutive `Verse` items
                // that share the SAME verse number. Merge those back into the
                // current line so the verse is never split mid-text — a whole
                // verse always lands intact on a slide.
                if acc.numbers.last() == Some(number) && !acc.lines.is_empty() {
                    // If the fragment would push this slide over the limit while
                    // OTHER (earlier-numbered) verses already share it, flush
                    // those earlier verses first so this growing verse ends up
                    // WHOLE on its own slide — never an oversized multi-verse
                    // slide that the validator would then reject. (flush_keeping_last
                    // is a no-op when the verse is alone on the slide, so no extra
                    // line-count guard is needed here.)
                    if acc.would_overflow_merge(text.len(), limit) {
                        acc.flush_keeping_last(&mut slides, &group_verses);
                    }
                    if let Some(last) = acc.lines.last_mut() {
                        last.push(' ');
                        last.push_str(text);
                        acc.group = Some((book.clone(), *chapter, translation.clone()));
                        continue;
                    }
                }

                let line = format!("{number}. {text}");
                if acc.would_overflow(line.len(), limit) {
                    acc.flush(&mut slides, &group_verses);
                }

                acc.lines.push(line);
                acc.numbers.push(*number);
                acc.group = Some((book.clone(), *chapter, translation.clone()));
            }
        }
    }

    acc.flush(&mut slides, &group_verses);
    slides
}

/// Mutable accumulator for the verses being packed onto the current slide.
/// `group` is `(book, chapter, translation)` for the in-progress slide.
#[derive(Default)]
struct VerseAccumulator {
    lines: Vec<String>,
    numbers: Vec<u32>,
    group: Option<(String, u32, String)>,
}

impl VerseAccumulator {
    /// True when appending `new_line_len` chars as a NEW line would push the
    /// joined slide past `limit` AND there is already content to flush. A lone
    /// verse (empty accumulator) is NEVER reported as overflowing — it is kept
    /// whole on its own slide even when it alone exceeds the limit (issue #394;
    /// the validator accepts a lone oversized verse, autofit shrinks it).
    fn would_overflow(&self, new_line_len: usize, limit: usize) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let existing_len: usize = self.lines.iter().map(String::len).sum();
        // joined existing lines = existing_len + (len - 1) separators; adding a
        // "\n" + new line = + 1 + new_line_len -> existing_len + len + new_line_len.
        let prospective = existing_len + self.lines.len() + new_line_len;
        prospective > limit
    }

    /// True when MERGING `frag_len` chars (plus a joining space) into the last
    /// line would push the joined slide past `limit`. Used to decide whether a
    /// same-number continuation fragment should force the EARLIER verses on the
    /// slide onto their own slide first (so the growing verse stays whole on its
    /// own slide rather than producing a rejected oversized multi-verse slide).
    fn would_overflow_merge(&self, frag_len: usize, limit: usize) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let existing_len: usize = self.lines.iter().map(String::len).sum();
        // current joined length + " " + fragment.
        let prospective = existing_len + (self.lines.len() - 1) + 1 + frag_len;
        prospective > limit
    }

    /// Build the reference label for the current group from its full verse set.
    fn group_reference(
        group: &(String, u32, String),
        group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>,
    ) -> String {
        let (book, chapter, translation) = group;
        match group_verses.get(group) {
            Some(verses) => format!(
                "{} {}:{} ({})",
                book,
                chapter,
                format_verse_range(verses),
                translation
            ),
            None => String::new(),
        }
    }

    /// Flush every accumulated line EXCEPT the last into one slide, keeping the
    /// last line (and its verse number) in the accumulator. Used when a
    /// same-number continuation fragment would overflow a slide that also holds
    /// earlier verses: the earlier verses flush, the growing verse stays.
    fn flush_keeping_last(
        &mut self,
        slides: &mut Vec<ComposedBibleSlide>,
        group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>,
    ) {
        if self.lines.len() < 2 {
            return;
        }
        let Some(group) = self.group.clone() else {
            return;
        };
        let last_line = self.lines.pop().unwrap_or_default();
        let last_number = self.numbers.pop();
        let main = self.lines.join("\n");
        slides.push(ComposedBibleSlide {
            main,
            main_reference: Self::group_reference(&group, group_verses),
        });
        self.lines.clear();
        self.numbers.clear();
        self.lines.push(last_line);
        if let Some(n) = last_number {
            self.numbers.push(n);
        }
        // group stays set — the kept verse continues in the same group.
    }

    /// Flush the accumulated verses into one slide. The reference is derived
    /// from the GROUP's full verse set (pass 1), not from `numbers`, so every
    /// slide of one passage displays the same label.
    ///
    /// Uses let-else so there is no panic path — if an invariant is broken by a
    /// future refactor (e.g., group set without lines), the flush is a no-op
    /// rather than a crash.
    fn flush(
        &mut self,
        slides: &mut Vec<ComposedBibleSlide>,
        group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>,
    ) {
        if self.lines.is_empty() {
            self.group = None;
            return;
        }
        let Some((book, chapter, translation)) = self.group.take() else {
            self.lines.clear();
            self.numbers.clear();
            return;
        };
        if self.numbers.is_empty() {
            self.lines.clear();
            return;
        }
        let main = self.lines.join("\n");
        let reference = Self::group_reference(&(book, chapter, translation), group_verses);
        slides.push(ComposedBibleSlide {
            main,
            main_reference: reference,
        });
        self.lines.clear();
        self.numbers.clear();
    }
}

/// Build a `Book Ch:V (CODE)` (or `Book Ch:V-V (CODE)`) reference label for a
/// passage range in the given translation. Used for both the main label and an
/// optional secondary-translation label on live-mode bible slides.
fn build_reference_label(
    book: &str,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
    translation_code: &str,
) -> String {
    let short_code = translation_short_code(translation_code);
    if verse_start == verse_end {
        format!("{book} {chapter}:{verse_start} ({short_code})")
    } else {
        format!("{book} {chapter}:{verse_start}-{verse_end} ({short_code})")
    }
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

    // The full reference label appears on all slides (includes translation
    // code). A secondary translation gets its own label.
    let full_reference_label = build_reference_label(
        &book,
        chapter,
        full_verse_start,
        full_verse_end,
        &main_translation.code,
    );
    let translation_reference_label = secondary_translation
        .map(|t| build_reference_label(&book, chapter, full_verse_start, full_verse_end, &t.code));

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
