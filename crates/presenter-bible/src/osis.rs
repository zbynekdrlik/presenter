//! OSIS book ids of the 66 books, in canonical order (#851 D3b). An exact table:
//! an id outside it fails the import, nothing is matched fuzzily at ingest.

const BOOKS: [&str; 66] = [
    "Gen", "Exod", "Lev", "Num", "Deut", "Josh", "Judg", "Ruth", "1Sam", "2Sam", "1Kgs", "2Kgs",
    "1Chr", "2Chr", "Ezra", "Neh", "Esth", "Job", "Ps", "Prov", "Eccl", "Song", "Isa", "Jer",
    "Lam", "Ezek", "Dan", "Hos", "Joel", "Amos", "Obad", "Jonah", "Mic", "Nah", "Hab", "Zeph",
    "Hag", "Zech", "Mal", "Matt", "Mark", "Luke", "John", "Acts", "Rom", "1Cor", "2Cor", "Gal",
    "Eph", "Phil", "Col", "1Thess", "2Thess", "1Tim", "2Tim", "Titus", "Phlm", "Heb", "Jas",
    "1Pet", "2Pet", "1John", "2John", "3John", "Jude", "Rev",
];

/// The canonical book number, 1 to 66.
pub(crate) fn book_number(osis: &str) -> Option<i32> {
    let index = BOOKS.iter().position(|book| *book == osis)?;
    i32::try_from(index + 1).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::bible::canonical_book_by_number;

    /// The id's letters appear in the book's name, in order: "Phlm" fits
    /// Philemon and not Philippians, so a swap anywhere in the table fails.
    #[test]
    fn every_book_sits_at_its_canonical_number() {
        for (index, osis) in BOOKS.iter().enumerate() {
            let number = book_number(osis).expect(osis);
            assert_eq!(number as usize, index + 1);
            let book = canonical_book_by_number(number as u16).expect("canonical book");
            let name = book.english_name.to_lowercase();
            let mut letters = name.chars();
            let fits = osis
                .to_lowercase()
                .chars()
                .all(|wanted| letters.any(|letter| letter == wanted));
            assert!(fits, "{osis} at {number}: {}", book.english_name);
        }
    }

    #[test]
    fn an_id_outside_the_table_is_unknown() {
        for id in ["gen", "Psalm", "Tob", ""] {
            assert_eq!(book_number(id), None, "{id}");
        }
    }
}
