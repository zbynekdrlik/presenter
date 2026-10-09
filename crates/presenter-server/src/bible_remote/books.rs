//! USFM book code → the book abbreviation `api.nlt.to` accepts in `ref=`
//! (#826). Every entry was checked live on 2026-10-09 (batched
//! `;`-separated refs): the OSIS abbreviations work except `1Thess`/`2Thess`
//! and `1John`/`2John`/`3John`, which the API rejects — it wants
//! `1Thes`/`2Thes` and `1Jn`/`2Jn`/`3Jn`. An unknown abbreviation is not an
//! HTTP error: the API answers 200 with an empty page.

/// `(USFM code, NLT API abbreviation)` for the 66 books, in canon order.
const NLT_BOOK_ABBREVIATIONS: [(&str, &str); 66] = [
    ("GEN", "Gen"),
    ("EXO", "Exod"),
    ("LEV", "Lev"),
    ("NUM", "Num"),
    ("DEU", "Deut"),
    ("JOS", "Josh"),
    ("JDG", "Judg"),
    ("RUT", "Ruth"),
    ("1SA", "1Sam"),
    ("2SA", "2Sam"),
    ("1KI", "1Kgs"),
    ("2KI", "2Kgs"),
    ("1CH", "1Chr"),
    ("2CH", "2Chr"),
    ("EZR", "Ezra"),
    ("NEH", "Neh"),
    ("EST", "Esth"),
    ("JOB", "Job"),
    ("PSA", "Ps"),
    ("PRO", "Prov"),
    ("ECC", "Eccl"),
    ("SNG", "Song"),
    ("ISA", "Isa"),
    ("JER", "Jer"),
    ("LAM", "Lam"),
    ("EZK", "Ezek"),
    ("DAN", "Dan"),
    ("HOS", "Hos"),
    ("JOL", "Joel"),
    ("AMO", "Amos"),
    ("OBA", "Obad"),
    ("JON", "Jonah"),
    ("MIC", "Mic"),
    ("NAM", "Nah"),
    ("HAB", "Hab"),
    ("ZEP", "Zeph"),
    ("HAG", "Hag"),
    ("ZEC", "Zech"),
    ("MAL", "Mal"),
    ("MAT", "Matt"),
    ("MRK", "Mark"),
    ("LUK", "Luke"),
    ("JHN", "John"),
    ("ACT", "Acts"),
    ("ROM", "Rom"),
    ("1CO", "1Cor"),
    ("2CO", "2Cor"),
    ("GAL", "Gal"),
    ("EPH", "Eph"),
    ("PHP", "Phil"),
    ("COL", "Col"),
    ("1TH", "1Thes"),
    ("2TH", "2Thes"),
    ("1TI", "1Tim"),
    ("2TI", "2Tim"),
    ("TIT", "Titus"),
    ("PHM", "Phlm"),
    ("HEB", "Heb"),
    ("JAS", "Jas"),
    ("1PE", "1Pet"),
    ("2PE", "2Pet"),
    ("1JN", "1Jn"),
    ("2JN", "2Jn"),
    ("3JN", "3Jn"),
    ("JUD", "Jude"),
    ("REV", "Rev"),
];

/// The `api.nlt.to` abbreviation of a USFM book code (`"1JN"` → `"1Jn"`),
/// or `None` for a code outside the 66-book canon.
pub(super) fn nlt_book_abbreviation(book_code: &str) -> Option<&'static str> {
    let book_code = book_code.trim();
    NLT_BOOK_ABBREVIATIONS
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(book_code))
        .map(|(_, abbreviation)| *abbreviation)
}
