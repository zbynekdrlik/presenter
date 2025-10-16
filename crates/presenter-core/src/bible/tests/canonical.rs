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
