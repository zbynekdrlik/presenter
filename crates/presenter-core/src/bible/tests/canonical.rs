use super::super::canonical::canonical_book_by_name;

#[test]
fn canonical_lookup_matches_slovak_aliases() {
    let judges = canonical_book_by_name("Sudcovia").expect("Judges alias present");
    assert_eq!(judges.code, "JDG");
    assert_eq!(judges.number, 7);

    let romans = canonical_book_by_name("List apoštola Pavla Rímskym").expect("Romans alias");
    assert_eq!(romans.code, "ROM");
    assert_eq!(romans.number, 45);

    let revelation = canonical_book_by_name("Revelation of John").expect("Revelation alias");
    assert_eq!(revelation.code, "REV");
    assert_eq!(revelation.number, 66);
}

#[test]
fn canonical_lookup_matches_slovak_abbreviations() {
    let cases = [
        ("Sk", "ACT"),
        ("Mar", "MRK"),
        ("Iz", "ISA"),
        ("Luk", "LUK"),
        ("Mat", "MAT"),
        ("Ján", "JHN"),
        ("Žid", "HEB"),
        ("Rim", "ROM"),
        ("Gal", "GAL"),
        ("Ef", "EPH"),
        ("Fil", "PHP"),
        ("Kol", "COL"),
        ("Tít", "TIT"),
        ("Flm", "PHM"),
        ("Žalm", "PSA"),
        ("Prísl", "PRO"),
        ("Jer", "JER"),
        ("Ez", "EZK"),
        ("Dan", "DAN"),
        ("1Sa", "1SA"),
        ("1Kor", "1CO"),
        ("2Kor", "2CO"),
        ("1Sol", "1TH"),
        ("1Tim", "1TI"),
        ("2Tim", "2TI"),
        ("Zj", "REV"),
    ];

    for (abbrev, expected_code) in cases {
        let book = canonical_book_by_name(abbrev)
            .unwrap_or_else(|| panic!("abbreviation '{abbrev}' should resolve"));
        assert_eq!(
            book.code, expected_code,
            "'{abbrev}' should map to {expected_code}"
        );
    }
}
