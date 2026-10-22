//! A row rebuilt while a save of its plate waits (#871 review): the row
//! stands on the controller's newest committed texts (`desired`, carried to
//! it in its `PlateSave`), never on the plate list's possibly older ones.

use super::*;

#[test]
fn a_rebuilt_row_stands_on_the_waiting_texts_so_a_role_typed_there_keeps_the_newer_name() {
    // Review finding: save 1 (a new name) on its way, the editor switched
    // output and back, so the plate's row was rebuilt while the list still
    // had the server's OLDER name. Typing only the role there committed
    // that older name with it, and it replaced the newer one waiting here.
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("Nová", "starý"), old(), false).1);
    // The old row leaves with the same texts (the editor output switch).
    assert_eq!(
        control.commit(1, t("Nová", "starý"), old(), false).0,
        Commit::OnItsWay
    );
    // The rebuilt row: the list still says `old()`, but the plate is busy,
    // so the row stands on its newest committed texts.
    let base = control.desired(1);
    assert_eq!(base, Some(t("Nová", "starý")), "not the listed {:?}", old());
    // Only the role is typed and left: the commit carries the base name.
    let (name, _) = base.unwrap_or_else(old);
    let (commit, _) = control.commit(1, (name, "kazateľ".to_string()), old(), false);
    assert_eq!(commit, Commit::Waits);
    let step = control
        .finish(1, first.token, Saved, true)
        .unwrap_or_default();
    let second = patch(&step);
    assert_eq!(second.texts, t("Nová", "kazateľ"), "both newest texts");
    let step = control
        .finish(1, second.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none(), "{step:?}");
    // Idle: no desired texts, so the row stands on the list again (which
    // the saved answer has just brought up to date).
    assert_eq!(control.desired(1), None);
}
