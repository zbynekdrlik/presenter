//! Pure bible-slide composition: the live-mode composer (`compose_bible_slides`)
//! and the AI item-stream composer (`compose_bible_items_into_slides`), plus the
//! data types and private formatting helpers they share. No `AppState` access.

use presenter_core::bible::sermon::{translation_short_code, verse_suffix, VerseMark};
use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
use presenter_core::{BiblePassage, BibleTranslation, Slide, SlideContent, SlideGroup, SlideText};
use std::collections::{BTreeSet, HashMap};

/// Typed input for the AI-facing bible composer. A stream of these is
/// produced by the LLM after it edits DB verses against the sermon text,
/// and the server composes slides out of the stream respecting the
/// character limit, counted in characters like live mode (#828). AI slides
/// carry no translation text, so only the main text is measured.
// `pub` (not `pub(crate)`): re-exported `pub` from `state::slides` so the
// #680 `ai_eval` Layer-1 scorer (a separate crate root) can replay a
// captured tool call's items through the REAL composer — see
// `state/slides.rs`'s re-export comment. `pub(crate) use`/`pub use` alone
// cannot widen an item past its OWN declared visibility, hence bumping it
// here too.
#[derive(Debug, Clone)]
pub enum BibleItem {
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
pub struct ComposedBibleSlide {
    pub main: String,
    pub main_reference: String,
}

/// Format a sorted set of verse numbers as a reference suffix through the core
/// label formatter (#939): runs of consecutive verses (`"17"`, `"17-20"`,
/// `"1-11, 20-22"`), never a verse-by-verse comma list. Empty set → "".
fn format_verse_range(verses: &BTreeSet<u32>) -> String {
    let marks: Vec<VerseMark> = verses
        .iter()
        .map(|n| VerseMark::plain(u16::try_from(*n).unwrap_or(u16::MAX)))
        .collect();
    verse_suffix(&marks)
}

/// One passage of an items stream: its book, chapter and translation and the
/// verse numbers it holds.
struct Passage {
    book: String,
    chapter: u32,
    translation: String,
    verses: BTreeSet<u32>,
}

impl Passage {
    /// "1. Samuelova 4:1-11 (SEB)" — the label every slide of the passage shows.
    fn reference(&self) -> String {
        format!(
            "{} {}:{} ({})",
            self.book,
            self.chapter,
            format_verse_range(&self.verses),
            self.translation
        )
    }

    /// True when `number` of `book`/`chapter`/`translation` continues this
    /// passage: the same verse again (a #394 continuation fragment) or the
    /// next one.
    fn continues_with(&self, number: u32, book: &str, chapter: u32, translation: &str) -> bool {
        let Some(last) = self.verses.last().copied() else {
            return false;
        };
        self.book == book
            && self.chapter == chapter
            && self.translation == translation
            && (number == last || last.checked_add(1) == Some(number))
    }
}

/// Pass 1 (#939): split the stream into passages. A passage is a run of verse
/// items of one book, chapter and translation whose numbers repeat or step by
/// one; an emphasis item between them does not end it (#292). A gap or a step
/// back starts a NEW passage — the owner's 1Sa 4:1-11 + 4:20-22 are two
/// passages, never one "4:1, 2, …, 22" label. Returns the passage index of
/// every item (`None` for an emphasis) and the passages.
fn split_passages(items: &[BibleItem]) -> (Vec<Option<usize>>, Vec<Passage>) {
    let mut passages: Vec<Passage> = Vec::new();
    let mut index_of_item: Vec<Option<usize>> = Vec::with_capacity(items.len());
    for item in items {
        let BibleItem::Verse {
            number,
            book,
            chapter,
            translation,
            ..
        } = item
        else {
            index_of_item.push(None);
            continue;
        };
        let continues = passages
            .last()
            .is_some_and(|p| p.continues_with(*number, book, *chapter, translation));
        if !continues {
            passages.push(Passage {
                book: book.clone(),
                chapter: *chapter,
                translation: translation.clone(),
                verses: BTreeSet::new(),
            });
        }
        let index = passages.len() - 1;
        if let Some(passage) = passages.get_mut(index) {
            passage.verses.insert(*number);
        }
        index_of_item.push(Some(index));
    }
    (index_of_item, passages)
}

/// Compose a stream of `BibleItem` into slides. The greedy-packing rule of
/// `compose_bible_slides`, on the main text only (AI slides have no
/// translation text): accumulate verses into one slide until the next verse
/// would overflow the character limit (characters, never bytes — #828), then
/// flush. Emphasis items and a new passage (translation/book/chapter change or
/// a gap in the verse numbers, #939) force a slide break.
///
/// A verse is NEVER split mid-text (issue #394): consecutive `Verse` items that
/// share the same verse number are merged back into one whole verse, and a lone
/// verse longer than `character_limit` is kept WHOLE on its own slide — the
/// validator accepts such a lone oversized verse (autofit shrinks it for
/// display) rather than rejecting it.
pub fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedBibleSlide> {
    let limit = character_limit as usize;
    let mut slides: Vec<ComposedBibleSlide> = Vec::new();

    // Pass 1: split the stream into passages (#939). Slides flushed in pass 2
    // use their passage's full verse set for the reference label, so all
    // slides of one passage display the same reference (issue #292).
    let (passage_of_item, passages) = split_passages(items);

    // Accumulator for the current verse slide.
    let mut acc = VerseAccumulator::default();

    for (item, passage) in items.iter().zip(passage_of_item) {
        match item {
            BibleItem::Emphasis { text } => {
                acc.flush(&mut slides, &passages);
                slides.push(ComposedBibleSlide {
                    main: text.clone(),
                    main_reference: String::new(),
                });
            }
            BibleItem::Verse { number, text, .. } => {
                // A new passage (another translation / book / chapter, or a
                // gap in the verse numbers — #939) forces a slide break.
                if acc.group.is_some() && acc.group != passage {
                    acc.flush(&mut slides, &passages);
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
                    if acc.would_overflow_merge(text.chars().count(), limit) {
                        acc.flush_keeping_last(&mut slides, &passages);
                    }
                    if let Some(last) = acc.lines.last_mut() {
                        last.push(' ');
                        last.push_str(text);
                        acc.group = passage;
                        continue;
                    }
                }

                let line = format!("{number}. {text}");
                if acc.would_overflow(line.chars().count(), limit) {
                    acc.flush(&mut slides, &passages);
                }

                acc.lines.push(line);
                acc.numbers.push(*number);
                acc.group = passage;
            }
        }
    }

    acc.flush(&mut slides, &passages);
    slides
}

/// Mutable accumulator for the verses being packed onto the current slide.
/// `group` is the index (into the pass-1 passages) of the in-progress slide's
/// passage.
#[derive(Default)]
struct VerseAccumulator {
    lines: Vec<String>,
    numbers: Vec<u32>,
    group: Option<usize>,
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
        let existing_len: usize = self.lines.iter().map(|line| line.chars().count()).sum();
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
        let existing_len: usize = self.lines.iter().map(|line| line.chars().count()).sum();
        // current joined length + " " + fragment.
        let prospective = existing_len + (self.lines.len() - 1) + 1 + frag_len;
        prospective > limit
    }

    /// Build the reference label for a passage from its full verse set.
    fn group_reference(group: usize, passages: &[Passage]) -> String {
        passages
            .get(group)
            .map(Passage::reference)
            .unwrap_or_default()
    }

    /// Flush every accumulated line EXCEPT the last into one slide, keeping the
    /// last line (and its verse number) in the accumulator. Used when a
    /// same-number continuation fragment would overflow a slide that also holds
    /// earlier verses: the earlier verses flush, the growing verse stays.
    fn flush_keeping_last(&mut self, slides: &mut Vec<ComposedBibleSlide>, passages: &[Passage]) {
        if self.lines.len() < 2 {
            return;
        }
        let Some(group) = self.group else {
            return;
        };
        let last_line = self.lines.pop().unwrap_or_default();
        let last_number = self.numbers.pop();
        let main = self.lines.join("\n");
        slides.push(ComposedBibleSlide {
            main,
            main_reference: Self::group_reference(group, passages),
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
    /// from the PASSAGE's full verse set (pass 1), not from `numbers`, so every
    /// slide of one passage displays the same label.
    ///
    /// Uses let-else so there is no panic path — if an invariant is broken by a
    /// future refactor (e.g., group set without lines), the flush is a no-op
    /// rather than a crash.
    fn flush(&mut self, slides: &mut Vec<ComposedBibleSlide>, passages: &[Passage]) {
        if self.lines.is_empty() {
            self.group = None;
            return;
        }
        let Some(group) = self.group.take() else {
            self.lines.clear();
            self.numbers.clear();
            return;
        };
        if self.numbers.is_empty() {
            self.lines.clear();
            return;
        }
        let main = self.lines.join("\n");
        let reference = Self::group_reference(group, passages);
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

/// The book name for the SECONDARY translation's reference label: the name the
/// secondary translation itself uses ("1 John" in eng-kjv, while the main
/// slk-seb says "1 Ján", #824). `None` when the secondary translation has none
/// of the verses: there is no translation text then, so no label either.
fn secondary_book_name<'a>(
    main_passages: &[BiblePassage],
    secondary_lookup: &'a HashMap<u16, BiblePassage>,
) -> Option<&'a str> {
    main_passages
        .iter()
        .find_map(|p| secondary_lookup.get(&p.reference.verse_start))
        .map(|p| p.reference.book.as_str())
}

/// The slide being built: its main and translation lines and verse spans.
#[derive(Default)]
struct SlideDraft {
    main: String,
    translation: String,
    verses: Vec<(u16, u16)>,
}

impl SlideDraft {
    fn is_empty(&self) -> bool {
        self.main.is_empty()
    }

    /// Whether adding this verse would push the main OR the translation text
    /// over `limit` characters (#828: whichever translation is longer decides).
    fn would_exceed(&self, main_line: &str, translation_line: Option<&str>, limit: usize) -> bool {
        joined_chars(&self.main, main_line) > limit
            || translation_line.is_some_and(|line| joined_chars(&self.translation, line) > limit)
    }

    fn push(&mut self, main_line: &str, translation_line: Option<&str>, verse: (u16, u16)) {
        push_line(&mut self.main, main_line);
        if let Some(line) = translation_line {
            push_line(&mut self.translation, line);
        }
        self.verses.push(verse);
    }
}

/// Characters (never bytes: a Slovak diacritic is one) of `text` once `line`
/// is appended on a new line.
fn joined_chars(text: &str, line: &str) -> usize {
    text.chars().count() + usize::from(!text.is_empty()) + line.chars().count()
}

fn push_line(text: &mut String, line: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(line);
}

/// Everything the live slides of one passage share besides their text.
struct BibleSlideFrame<'a> {
    main_translation: &'a BibleTranslation,
    secondary_translation: Option<&'a BibleTranslation>,
    book: String,
    book_code: Option<String>,
    book_number: Option<u16>,
    chapter: u16,
    reference_label: String,
    translation_reference_label: Option<String>,
}

impl BibleSlideFrame<'_> {
    /// Append the slide of `draft` to `slides` (a blank draft adds nothing).
    fn push_slide(&self, slides: &mut Vec<Slide>, draft: SlideDraft) -> anyhow::Result<()> {
        if draft.main.trim().is_empty() {
            return Ok(());
        }
        let content = SlideContent::new(
            SlideText::new(&draft.main)?,
            SlideText::new(&draft.translation)?,
            SlideText::new(&self.reference_label)?,
            Some(SlideGroup::new(&self.reference_label)),
        );
        let metadata = SlideMetadata::new().with_bible(BibleSlideMetadata {
            translation_code: self.main_translation.code.clone(),
            secondary_translation_code: self.secondary_translation.map(|t| t.code.clone()),
            book: self.book.clone(),
            book_code: self.book_code.clone(),
            book_number: self.book_number,
            chapter: self.chapter,
            verses: draft
                .verses
                .iter()
                .map(|(s, e)| BibleSlideVerseRef::new(*s, *e))
                .collect(),
            main_reference_label: Some(self.reference_label.clone()),
            translation_reference_label: self.translation_reference_label.clone(),
        });
        slides.push(Slide::new(slides.len() as u32, content).with_metadata(Some(metadata)));
        Ok(())
    }
}

/// The labels every slide of one composed passage carries (#851 P3): the live
/// path builds them from the verse range, a sermon item passes its computed
/// `16a`-style label and the secondary book's plain-number label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlideLabels {
    pub(crate) main: String,
    pub(crate) translation: Option<String>,
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
    let Some(first) = main_passages.first() else {
        return Ok(Vec::new());
    };
    let chapter = first.reference.chapter;

    // The full reference label appears on all slides (includes translation
    // code). A secondary translation gets its own label.
    let labels = SlideLabels {
        main: build_reference_label(
            &first.reference.book,
            chapter,
            full_verse_start,
            full_verse_end,
            &main_translation.code,
        ),
        translation: secondary_translation.and_then(|t| {
            let secondary_book = secondary_book_name(main_passages, secondary_lookup)?;
            Some(build_reference_label(
                secondary_book,
                chapter,
                full_verse_start,
                full_verse_end,
                &t.code,
            ))
        }),
    };
    compose_bible_slides_labelled(
        main_translation,
        secondary_translation,
        main_passages,
        secondary_lookup,
        character_limit,
        labels,
    )
}

/// The live composer with the caller's labels (#851 P3): one line
/// `"{verse}. {text}"` per passage, a new slide when the LONGER of the main and
/// the translation text would pass `character_limit` characters (#828), a lone
/// oversized verse kept whole (#434).
pub(crate) fn compose_bible_slides_labelled(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[BiblePassage],
    secondary_lookup: &HashMap<u16, BiblePassage>,
    character_limit: u32,
    labels: SlideLabels,
) -> anyhow::Result<Vec<Slide>> {
    let Some(first) = main_passages.first() else {
        return Ok(Vec::new());
    };
    let frame = BibleSlideFrame {
        main_translation,
        secondary_translation,
        book: first.reference.book.clone(),
        book_code: first.reference.book_code.clone(),
        book_number: first.reference.book_number,
        chapter: first.reference.chapter,
        reference_label: labels.main,
        translation_reference_label: labels.translation,
    };

    let limit = character_limit as usize;
    let mut slides = Vec::new();
    let mut draft = SlideDraft::default();
    for passage in main_passages {
        let verse = passage.reference.verse_start;
        let main_line = format!("{verse}. {}", passage.text);
        let translation_line = secondary_lookup
            .get(&verse)
            .map(|sec| format!("{}. {}", sec.reference.verse_start, sec.text));
        // A lone verse longer than the limit stays one whole slide (#434).
        if !draft.is_empty() && draft.would_exceed(&main_line, translation_line.as_deref(), limit) {
            frame.push_slide(&mut slides, std::mem::take(&mut draft))?;
        }
        draft.push(
            &main_line,
            translation_line.as_deref(),
            (verse, passage.reference.verse_end),
        );
    }
    frame.push_slide(&mut slides, draft)?;

    Ok(slides)
}
