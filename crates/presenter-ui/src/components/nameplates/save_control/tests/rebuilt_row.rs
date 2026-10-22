//! The controller side of a row rebuilt while a save of its plate waits
//! (#871 review): what the rebuilt row relies on. While the plate is busy,
//! `desired` is the newest committed texts (the row reads them through its
//! `PlateSave` and stands on them, `row_save::base_text`, tested there);
//! once idle it is `None`, so the row stands on the list again. And a
//! failed save of such a row, which has no drafts to commit, is sent by a
//! retry.

use super::*;

#[test]
fn desired_is_the_newest_committed_texts_while_busy_and_none_once_idle() {
    // Controller contract, not the row: save 1 (a new name) on its way, the
    // editor switched output and back, so the plate's row was rebuilt while
    // the list still had the server's OLDER texts. While busy, `desired`
    // holds the newest committed texts (what the rebuilt row stands on);
    // a commit built on them keeps the newer name; idle, it is `None`.
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("Nová", "starý"), old(), false).1);
    // The old row leaves with the same texts (the editor output switch).
    assert_eq!(
        control.commit(1, t("Nová", "starý"), old(), false).0,
        Commit::OnItsWay
    );
    // Busy: the newest committed texts, not the listed `old()`.
    let base = control.desired(1);
    assert_eq!(base, Some(t("Nová", "starý")), "not the listed {:?}", old());
    // A commit built on them (only the role typed): it keeps the newer name.
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

#[test]
fn a_rebuilt_rows_failed_save_with_nothing_on_its_way_is_sent_by_a_retry_once() {
    // Review finding: save 1 (a new name) on its way, the editor switched
    // output and back, and save 1 failed after the switch back. The plate is
    // listed again, so it waits for a retry. Its rebuilt row has no drafts,
    // so no commit of it reaches the controller, and the hidden-page send
    // skips it (no save is on its way to send it beside). `retry` is what
    // sends it; the row calls it (`NameplateStore::retry_of`) when it has
    // nothing to commit.
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("Nová", "starý"), old(), false).1);
    // The old row leaves with the same texts (the editor output switch).
    assert_eq!(
        control.commit(1, t("Nová", "starý"), old(), false).0,
        Commit::OnItsWay
    );
    let step = control
        .finish(1, first.token, Failed, true)
        .unwrap_or_default();
    assert!(step.patch.is_none() && step.retry_in.is_some(), "{step:?}");
    assert!(control.is_unsaved(1));
    assert!(control.send_waiting(|other| other != 1).is_empty());

    let retry = patch(&control.retry(1));
    assert_eq!(retry.texts, t("Nová", "starý"), "the desired texts");
    // Asked again (hidden, then `pagehide`): it is on its way, so nothing.
    assert_eq!(control.retry(1), Step::default());
    assert!(control.send_waiting(|other| other != 1).is_empty());

    let step = control
        .finish(1, retry.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none(), "{step:?}");
    assert!(!control.is_busy(1));
}
