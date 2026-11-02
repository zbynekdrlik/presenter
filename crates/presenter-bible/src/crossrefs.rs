//! OpenBible.info cross-references (#851 D3b, spec §5.5): `From Verse`, `To
//! Verse`, `Votes`, tab separated, OSIS ids. Every row is parsed before the
//! votes filter, so an id the table does not know fails the import.

use anyhow::{bail, ensure, Context, Result};
use presenter_core::bible::CrossReference;

use crate::lexicon::read_zip_member;
use crate::osis::book_number;

const MEMBER: &str = "cross_references.txt";
const HEADER: &str = "From Verse";
/// Rows below this are left out: 2 keeps 29,234 verses, 5 would only 19,892.
const MIN_VOTES: i32 = 2;

fn place(id: &str) -> Result<(i32, i32, i32)> {
    let mut parts = id.split('.');
    let (Some(book), Some(chapter), Some(verse), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        bail!("malformed OSIS id {id:?}");
    };
    let book = book_number(book).with_context(|| format!("unknown OSIS book in {id:?}"))?;
    let chapter = chapter
        .parse()
        .with_context(|| format!("chapter of {id:?}"))?;
    let verse = verse.parse().with_context(|| format!("verse of {id:?}"))?;
    Ok((book, chapter, verse))
}

fn parse_line(line: &str) -> Result<CrossReference> {
    let mut columns = line.split('\t');
    let (Some(from), Some(to), Some(votes), None) = (
        columns.next(),
        columns.next(),
        columns.next(),
        columns.next(),
    ) else {
        bail!("expected three columns");
    };
    let (from_book, from_chapter, from_verse) = place(from)?;
    let (start, end) = to.split_once('-').unwrap_or((to, to));
    let (to_book, to_chapter, to_verse) = place(start)?;
    let (to_book_end, to_chapter_end, to_verse_end) = place(end)?;
    ensure!(
        (to_book, to_chapter, to_verse) <= (to_book_end, to_chapter_end, to_verse_end),
        "range {to:?} runs backwards"
    );
    Ok(CrossReference {
        from_book,
        from_chapter,
        from_verse,
        to_book,
        to_chapter,
        to_verse,
        to_book_end,
        to_chapter_end,
        to_verse_end,
        votes: votes.trim().parse().context("votes")?,
    })
}

/// The rows worth showing, in primary-key order (the table's own, so the
/// inserts append).
pub fn parse_cross_references(zip_bytes: &[u8]) -> Result<Vec<CrossReference>> {
    let text = String::from_utf8(read_zip_member(zip_bytes, MEMBER)?)
        .context("cross-references are not UTF-8")?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with(HEADER) {
            continue;
        }
        let row = parse_line(line).with_context(|| format!("{MEMBER} line {}", index + 1))?;
        if row.votes >= MIN_VOTES {
            rows.push(row);
        }
    }
    rows.sort_unstable_by_key(|row| {
        (
            row.from_book,
            row.from_chapter,
            row.from_verse,
            row.to_book,
            row.to_chapter,
            row.to_verse,
        )
    });
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::test_zip;

    const COMMITTED: &[u8] = include_bytes!("../../../data/study/openbible-cross-references.zip");

    fn parsed(body: &str) -> Result<Vec<CrossReference>> {
        let text = format!("From Verse\tTo Verse\tVotes\t#header\n{body}");
        parse_cross_references(&test_zip(&[(MEMBER, text.as_bytes())]))
    }

    #[test]
    fn a_verse_a_chapter_range_and_a_book_range() {
        let rows = parsed(
            "Gen.1.1\tJer.32.17\t91\nGen.1.2\tGen.1.1-Gen.2.3\t5\n1John.3.18\t2John.1.1-3John.1.15\t11\n",
        )
        .unwrap();
        assert_eq!(
            (rows[0].to_book, rows[0].to_chapter, rows[0].to_verse),
            (24, 32, 17)
        );
        assert_eq!(rows[0].to_book_end, 24);
        assert_eq!((rows[0].to_chapter_end, rows[0].to_verse_end), (32, 17));
        assert_eq!((rows[0].votes, rows[0].from_book), (91, 1));
        assert_eq!((rows[1].to_chapter_end, rows[1].to_verse_end), (2, 3));
        assert_eq!((rows[2].to_book, rows[2].to_book_end), (63, 64));
        assert_eq!((rows[2].to_chapter_end, rows[2].to_verse_end), (1, 15));
    }

    #[test]
    fn rows_under_two_votes_stay_out() {
        let rows =
            parsed("Gen.1.1\tJer.1.1\t2\nGen.1.1\tJer.1.2\t1\nGen.1.1\tJer.1.3\t-3\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].to_verse, 1);
    }

    #[test]
    fn an_unknown_id_fails_even_on_a_row_the_votes_would_drop() {
        let err = parsed("Gen.1.1\tTob.1.1\t0\n").unwrap_err();
        assert!(format!("{err:#}").contains("unknown OSIS book"), "{err:#}");
        assert!(format!("{err:#}").contains("line 2"), "{err:#}");
    }

    #[test]
    fn malformed_rows_fail() {
        for row in [
            "Gen.1.1\tJer.1.1\n",
            "Gen.1\tJer.1.1\t3\n",
            "Gen.1.1\tJer.1.1\tmany\n",
            "Gen.1.1\tJer.2.1-Jer.1.1\t3\n",
        ] {
            assert!(parsed(row).is_err(), "{row:?}");
        }
    }

    #[test]
    fn the_committed_file_has_every_row_it_should() {
        let rows = parse_cross_references(COMMITTED).unwrap();
        assert_eq!(rows.len(), 328_139);
        let mut verses: Vec<_> = rows
            .iter()
            .map(|row| (row.from_book, row.from_chapter, row.from_verse))
            .collect();
        verses.dedup();
        assert_eq!(verses.len(), 29_234);
        assert_eq!(
            rows.iter()
                .filter(|row| row.to_book != row.to_book_end)
                .count(),
            16
        );
    }
}
