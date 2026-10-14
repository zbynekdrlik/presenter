//! Slovak hints and labels (#850 spec §5.6).

use crate::bible::resolver::{chapters_hint, counted, segment_label, verses_hint, Segment};

#[test]
fn slovak_counted_forms() {
    assert_eq!(counted(1, "kapitolu", "kapitoly", "kapitol"), "1 kapitolu");
    assert_eq!(counted(3, "kapitolu", "kapitoly", "kapitol"), "3 kapitoly");
    assert_eq!(counted(5, "kapitolu", "kapitoly", "kapitol"), "5 kapitol");
    assert_eq!(
        counted(150, "kapitolu", "kapitoly", "kapitol"),
        "150 kapitol"
    );
    assert_eq!(chapters_hint(13), "Kniha má len 13 kapitol");
    assert_eq!(chapters_hint(1), "Kniha má len 1 kapitolu");
    assert_eq!(verses_hint(40), "Kapitola má len 40 veršov");
    assert_eq!(verses_hint(4), "Kapitola má len 4 verše");
}

#[test]
fn segment_labels() {
    assert_eq!(segment_label("Žalmy", &Segment::whole(23)), "Žalmy 23");
    assert_eq!(
        segment_label("Hebrejom", &Segment::verses(11, 1, 1)),
        "Hebrejom 11:1"
    );
    assert_eq!(
        segment_label("Ján", &Segment::verses(3, 16, 18)),
        "Ján 3:16-18"
    );
}
