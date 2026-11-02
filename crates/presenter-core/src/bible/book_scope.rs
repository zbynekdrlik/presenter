//! Which books a study read covers (#851 D4, spec §6.4, §11): `all`, `ot`, `nt`
//! or a book code. Book numbers are canonical, so a scope works in every
//! translation.

use super::canonical_book_by_code;

const LAST_OLD_TESTAMENT_BOOK: u16 = 39;
const LAST_BOOK: u16 = 66;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookScope {
    #[default]
    All,
    OldTestament,
    NewTestament,
    Book(u16),
}

impl BookScope {
    /// `None` for anything that is not a scope. Empty means the whole Bible.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() || input.eq_ignore_ascii_case("all") {
            return Some(Self::All);
        }
        if input.eq_ignore_ascii_case("ot") {
            return Some(Self::OldTestament);
        }
        if input.eq_ignore_ascii_case("nt") {
            return Some(Self::NewTestament);
        }
        canonical_book_by_code(input).map(|book| Self::Book(book.number))
    }

    /// The first and the last book number, both included.
    pub fn book_numbers(self) -> (u16, u16) {
        match self {
            Self::All => (1, LAST_BOOK),
            Self::OldTestament => (1, LAST_OLD_TESTAMENT_BOOK),
            Self::NewTestament => (LAST_OLD_TESTAMENT_BOOK + 1, LAST_BOOK),
            Self::Book(number) => (number, number),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_is_a_word_or_a_book_code() {
        assert_eq!(BookScope::parse(""), Some(BookScope::All));
        assert_eq!(BookScope::parse(" all "), Some(BookScope::All));
        assert_eq!(BookScope::parse("OT"), Some(BookScope::OldTestament));
        assert_eq!(BookScope::parse("nt"), Some(BookScope::NewTestament));
        assert_eq!(BookScope::parse("heb"), Some(BookScope::Book(58)));
        assert_eq!(BookScope::parse("Hebrews"), None);
    }

    #[test]
    fn the_testaments_meet_between_malachi_and_matthew() {
        assert_eq!(
            canonical_book_by_code("MAL").map(|book| book.number),
            Some(BookScope::OldTestament.book_numbers().1)
        );
        assert_eq!(
            canonical_book_by_code("MAT").map(|book| book.number),
            Some(BookScope::NewTestament.book_numbers().0)
        );
        assert_eq!(BookScope::Book(58).book_numbers(), (58, 58));
    }
}
