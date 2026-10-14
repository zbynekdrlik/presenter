//! Resolver data types (#850 spec §5.1).

/// The source family a book form comes from (spec §5.5); lower wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Slovak: stored names, the four biblia.sk systems, the canonical Slovak lists.
    T1 = 1,
    /// BPRP Slovak, English names, OSIS ids.
    T2 = 2,
    /// Czech and BPRP English short forms.
    T3 = 3,
}

/// How a typed key met a book form; lower wins (spec §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    Exact,
    /// `kr` = the `1 Kr` family without its number.
    OrdinalStripped,
    Prefix,
    /// Damerau-Levenshtein within 1 (key length 4–6) or 2 (7+).
    Fuzzy,
}

/// One book a typed name may mean, best first in [`ParsedQuery::candidates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookCandidate {
    pub code: String,
    pub number: u16,
    pub tier: Tier,
    pub kind: MatchKind,
    /// 1 when a numbered book matched without a typed ordinal; fuzzy adds the distance.
    pub penalty: u8,
    /// The form that matched, as its source writes it (`Žid`).
    pub matched_form: String,
}

/// The chapter/verse part as typed, before a book's structure is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSpec {
    /// `11`; on a single-chapter book a lone `n > 1` is verse `n`.
    Chapter(u16),
    /// `23-24`.
    ChapterRange { start: u16, end: u16 },
    /// `11:1`, `11:1-3`, `5:3.5.7`, `3:16,18`; each range has `end >= start`.
    Verses {
        chapter: u16,
        ranges: Vec<(u16, u16)>,
    },
    /// `1:1-2:3`.
    CrossChapter {
        chapter: u16,
        verse_start: u16,
        end_chapter: u16,
        verse_end: u16,
    },
}

impl RawSpec {
    /// The segments as typed, without a structure (for `Resolution::NoBook`).
    pub fn unbound_segments(&self) -> Vec<Segment> {
        match self {
            Self::Chapter(chapter) => vec![Segment::whole(*chapter)],
            Self::ChapterRange { start, end } => vec![Segment::whole(*start), Segment::whole(*end)],
            Self::Verses { chapter, ranges } => ranges
                .iter()
                .map(|&(start, end)| Segment::verses(*chapter, start, end))
                .collect(),
            Self::CrossChapter {
                chapter,
                verse_start,
                end_chapter,
                verse_end,
            } => vec![
                Segment {
                    chapter: *chapter,
                    verse_start: Some(*verse_start),
                    verse_end: None,
                },
                Segment::verses(*end_chapter, 1, *verse_end),
            ],
        }
    }
}

/// Pure text analysis of one input: book candidates, the chapter/verse part
/// and the words the grammar did not use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub input: String,
    /// The book part as typed (`zid`, `1 kor`, `I.`).
    pub book_text: String,
    pub ordinal: Option<u8>,
    pub key: String,
    /// A bare ordinal with no book (`1`, `2.`).
    pub bare_ordinal: Option<u8>,
    pub candidates: Vec<BookCandidate>,
    pub spec: Option<RawSpec>,
    /// Text after the last chapter/verse token the grammar used.
    pub leftover: String,
}

/// One contiguous piece of a passage; both verses `None` = the whole chapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub chapter: u16,
    pub verse_start: Option<u16>,
    pub verse_end: Option<u16>,
}

impl Segment {
    pub const fn whole(chapter: u16) -> Self {
        Self {
            chapter,
            verse_start: None,
            verse_end: None,
        }
    }

    pub const fn verses(chapter: u16, start: u16, end: u16) -> Self {
        Self {
            chapter,
            verse_start: Some(start),
            verse_end: Some(end),
        }
    }
}

/// A book as the MAIN translation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRef {
    pub code: String,
    pub number: u16,
    /// The stored name in the main translation (`Hebrejom` on slk-seb).
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub book: BookRef,
    pub matched_form: String,
    pub segments: Vec<Segment>,
}

/// What an input means against the main translation (spec §5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// `alternatives` = other books that fit, best first.
    Passage {
        passage: Passage,
        alternatives: Vec<Passage>,
    },
    /// The default ties on (tier, kind) with another fitting book.
    Ambiguous {
        default: Passage,
        alternatives: Vec<Passage>,
    },
    OutOfRange {
        book: BookRef,
        requested: Segment,
        hint: String,
        fix: Option<Passage>,
    },
    /// A book with no numbers typed.
    BookOnly {
        book: BookRef,
        alternatives: Vec<BookRef>,
    },
    /// Only an ordinal typed (`1`, `I.`, `prvý`).
    Incomplete { ordinal: u8, books: Vec<BookRef> },
    /// `11:1` → hint "Napíš aj knihu"; words only → text search (no hint).
    NoBook {
        segments: Vec<Segment>,
        hint: Option<String>,
    },
}
