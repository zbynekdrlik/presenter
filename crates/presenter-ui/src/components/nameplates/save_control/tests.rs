//! The plate save controller's transitions (#871): serialized saves, the
//! urgent send of a hidden page, failure streaks and their timers, a gone
//! plate, a refused text, the waiting Zobraziť, and what the rows settle.

use super::SaveOutcome::{Failed, Gone, Refused, Saved};
use super::*;

fn t(name: &str, role: &str) -> Texts {
    (name.to_string(), role.to_string())
}

/// The texts the page had from the server before the edits.
fn old() -> Texts {
    t("Starý", "starý")
}

/// The PATCH a step starts (a test failure when it starts none).
fn patch(step: &Step) -> Patch {
    step.patch
        .clone()
        .unwrap_or_else(|| panic!("expected a PATCH, got {step:?}"))
}

/// The fields a step settles.
fn settled(step: &Step) -> Option<Settle> {
    step.settled.as_ref().map(|(_, settle)| settle.clone())
}

fn both(name: &str, role: &str) -> Option<Settle> {
    Some(Settle::Fields {
        name: Some(name.to_string()),
        role: Some(role.to_string()),
    })
}

#[test]
fn the_first_commit_of_a_plate_is_sent_at_once_and_other_plates_never_wait() {
    let mut control = SaveControl::default();
    let (commit, step) = control.commit(1, t("Anna", "pastorka"), old(), false);
    assert_eq!(commit, Commit::Sent);
    assert_eq!(patch(&step).texts, t("Anna", "pastorka"));
    let (commit, step) = control.commit(2, t("Ben", "hosť"), old(), false);
    assert_eq!(commit, Commit::Sent);
    assert_eq!(patch(&step).texts, t("Ben", "hosť"));
    assert!(control.is_busy(1) && control.is_busy(2));
}

#[test]
fn a_commit_while_a_save_is_on_its_way_is_sent_after_it_once_with_the_newest_texts() {
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A1", "r"), old(), false).1);
    assert_eq!(
        control.commit(1, t("A2", "r"), old(), false),
        (Commit::Waits, Step::default())
    );
    assert_eq!(
        control.commit(1, t("A3", "r"), old(), false),
        (Commit::Waits, Step::default())
    );
    assert_eq!(control.desired(1), Some(t("A3", "r")));

    let step = control
        .finish(1, first.token, Saved, true)
        .unwrap_or_default();
    let second = patch(&step);
    assert_eq!(second.texts, t("A3", "r"));
    // The role is final already (confirmed, desired, and on its way).
    assert_eq!(
        settled(&step),
        Some(Settle::Fields {
            name: None,
            role: Some("r".to_string())
        })
    );

    let step = control
        .finish(1, second.token, Saved, true)
        .unwrap_or_default();
    assert_eq!(step.patch, None, "exactly one save after the first");
    assert!(step.idle);
    assert_eq!(settled(&step), both("A3", "r"));
    assert!(!control.is_busy(1));
}

#[test]
fn a_commit_of_the_texts_already_on_their_way_sends_nothing_more() {
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    // The window blur saved it; the page hides right after (urgent).
    assert_eq!(
        control.commit(1, t("A", "r"), old(), true),
        (Commit::OnItsWay, Step::default())
    );
    let step = control
        .finish(1, first.token, Saved, true)
        .unwrap_or_default();
    assert_eq!(step.patch, None);
    assert!(step.idle);
}

#[test]
fn typed_and_typed_back_within_one_round_trip_is_saved_once_and_never_stays_unsaved() {
    // #871 review: saves were told apart by their texts; A → B → A left the
    // row marked „neuložené" after the text was saved.
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.commit(1, t("B", "r"), old(), false);
    control.commit(1, t("A", "r"), old(), false);
    let step = control
        .finish(1, first.token, Saved, true)
        .unwrap_or_default();
    assert_eq!(step.patch, None, "the server has A, the desired text");
    assert!(step.idle);

    // The same, with the first save failing: A waits for a retry, goes once,
    // and the plate is saved and unmarked.
    let first = patch(&control.commit(2, t("A", "r"), old(), false).1);
    control.commit(2, t("B", "r"), old(), false);
    control.commit(2, t("A", "r"), old(), false);
    let step = control
        .finish(2, first.token, Failed, true)
        .unwrap_or_default();
    assert_eq!(step.patch, None, "A failed: it waits for a retry");
    assert!(control.is_unsaved(2));
    let retry = patch(&control.retry(2));
    assert_eq!(retry.texts, t("A", "r"));
    let step = control
        .finish(2, retry.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle);
    assert!(!control.is_unsaved(2));
    assert!(!control.is_busy(2));
}

#[test]
fn a_failed_save_keeps_the_desired_texts_and_retries_after_2_5_and_15_seconds() {
    let mut control = SaveControl::default();
    let mut sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let mut delays = Vec::new();
    for _ in 0..4 {
        let step = control
            .finish(1, sent.token, Failed, true)
            .unwrap_or_default();
        assert_eq!(step.patch, None);
        assert!(control.is_unsaved(1));
        assert_eq!(control.desired(1), Some(t("A", "r")));
        delays.push(step.retry_in.map(|(_, delay)| delay));
        sent = patch(&control.retry(1));
        assert_eq!(sent.texts, t("A", "r"));
    }
    assert_eq!(delays, vec![Some(2_000), Some(5_000), Some(15_000), None]);
    let step = control
        .finish(1, sent.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle);
    assert!(!control.is_unsaved(1));
}

#[test]
fn a_retry_timer_acts_only_on_the_failure_it_was_armed_for() {
    // #871 review: an old timer fired early for the next failure.
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let step = control
        .finish(1, sent.token, Failed, true)
        .unwrap_or_default();
    let (first_failure, _) = step.retry_in.unwrap_or_default();

    // The operator commits again before that timer: sent at once, fails.
    let sent = patch(&control.commit(1, t("B", "r"), old(), false).1);
    let step = control
        .finish(1, sent.token, Failed, true)
        .unwrap_or_default();
    let (second_failure, delay) = step.retry_in.unwrap_or_default();
    assert_eq!(delay, 5_000, "the second failure in a row");

    // The first timer runs out: nothing; the second one retries.
    assert_eq!(control.retry_due(1, first_failure), Step::default());
    let retry = patch(&control.retry_due(1, second_failure));
    assert_eq!(retry.texts, t("B", "r"));
    // While that retry is on its way, the same timer does nothing more.
    assert_eq!(control.retry_due(1, second_failure), Step::default());
}

#[test]
fn a_saved_answer_ends_the_failure_streak() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let failure = control
        .finish(1, sent.token, Failed, true)
        .unwrap_or_default()
        .retry_in;
    let (epoch, _) = failure.unwrap_or_default();
    let retry = patch(&control.retry(1));
    control.commit(1, t("B", "r"), old(), false);
    let step = control
        .finish(1, retry.token, Saved, true)
        .unwrap_or_default();
    assert!(!control.is_unsaved(1), "the network works again");
    let next = patch(&step);
    assert_eq!(next.texts, t("B", "r"));
    assert_eq!(control.retry_due(1, epoch), Step::default());
    // A new failure starts a new streak: 2 s again.
    let step = control
        .finish(1, next.token, Failed, true)
        .unwrap_or_default();
    assert_eq!(step.retry_in.map(|(_, delay)| delay), Some(2_000));
}

#[test]
fn a_commit_made_while_the_failed_save_was_on_its_way_goes_at_once() {
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.commit(1, t("B", "r"), old(), false);
    let step = control
        .finish(1, first.token, Failed, true)
        .unwrap_or_default();
    assert_eq!(patch(&step).texts, t("B", "r"));
    assert!(control.is_unsaved(1), "marked until B is saved");
}

#[test]
fn any_commit_sends_a_failed_save_again() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.finish(1, sent.token, Failed, true);
    // Leaving the field again (same texts) is a retry.
    let (commit, step) = control.commit(1, t("A", "r"), old(), false);
    assert_eq!(commit, Commit::Sent);
    assert_eq!(patch(&step).texts, t("A", "r"));
}

#[test]
fn typed_back_to_the_saved_texts_after_a_failure_is_sent_once_more() {
    // Review finding: a failed save may have been applied all the same (a
    // lost answer, the 15 s abort after the server wrote it, an unreadable
    // 2xx body), so after B failed the server may hold B. Typed back to A,
    // A is sent once more (the PATCH is idempotent), never left unsent.
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.finish(1, first.token, Saved, true);
    // Idle; the page's list now has A.
    assert!(!control.is_busy(1));
    let second = patch(&control.commit(1, t("B", "r"), t("A", "r"), false).1);
    control.finish(1, second.token, Failed, true);
    assert!(control.is_unsaved(1));
    let (commit, step) = control.commit(1, t("A", "r"), t("A", "r"), false);
    assert_eq!(commit, Commit::Sent);
    let again = patch(&step);
    assert_eq!(again.texts, t("A", "r"));
    let step = control
        .finish(1, again.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none(), "sent once");
    assert_eq!(settled(&step), both("A", "r"));
    assert!(!control.is_unsaved(1));
    assert!(!control.is_busy(1));
}

#[test]
fn a_save_that_never_answers_is_aborted_as_a_failure_so_the_plate_is_free_again() {
    // #871 review: the 15 s timeout aborts the fetch, a network error.
    let aborted = crate::api::ApiError::Network(gloo_net::Error::GlooError(
        "signal is aborted without reason".to_string(),
    ));
    assert_eq!(SaveOutcome::of_error(&aborted), Failed);
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.commit(1, t("B", "r"), old(), false);
    let step = control
        .finish(1, sent.token, SaveOutcome::of_error(&aborted), true)
        .unwrap_or_default();
    assert_eq!(patch(&step).texts, t("B", "r"), "nothing blocks the plate");
}

#[test]
fn a_commit_of_the_saved_texts_on_an_idle_plate_sends_nothing_and_settles_the_drafts() {
    let mut control = SaveControl::default();
    let (commit, step) = control.commit(1, t("A", "r"), t("A", "r"), false);
    assert_eq!(commit, Commit::Unchanged);
    assert_eq!(step.patch, None);
    assert_eq!(settled(&step), both("A", "r"));
    assert!(!control.is_busy(1));
}

#[test]
fn a_failure_whose_text_no_row_holds_gives_up() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let step = control
        .finish(1, sent.token, Failed, false)
        .unwrap_or_default();
    assert!(step.idle);
    assert_eq!(step.retry_in, None);
    assert!(!control.is_busy(1));
}

#[test]
fn a_gone_plate_drops_its_desired_texts_its_waiting_zobrazit_and_later_answers() {
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.commit(1, t("B", "r"), old(), true); // urgent: a second one on its way
    let (now, _) = control.show(1);
    assert!(!now);
    let step = control
        .finish(1, first.token, Gone, true)
        .unwrap_or_default();
    assert_eq!(settled(&step), Some(Settle::Gone));
    assert!(step.not_shown && step.idle);
    assert_eq!(step.patch, None);
    assert!(!control.is_busy(1));
    // The save sent beside it is answered later: forgotten, nothing to say.
    assert_eq!(control.finish(1, first.token + 1, Gone, true), None);
}

#[test]
fn refused_desired_texts_are_dropped_and_the_fields_show_the_saved_ones_again() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let step = control
        .finish(1, sent.token, Refused, true)
        .unwrap_or_default();
    assert!(step.idle);
    assert_eq!(settled(&step), both("A", "r"));
    assert!(!control.is_busy(1));

    // Refused texts that are no longer the desired ones: the desired go.
    let sent = patch(&control.commit(2, t("A", "r"), old(), false).1);
    control.commit(2, t("B", "r"), old(), false);
    let step = control
        .finish(2, sent.token, Refused, true)
        .unwrap_or_default();
    assert_eq!(patch(&step).texts, t("B", "r"));
}

#[test]
fn an_urgent_commit_sends_beside_the_save_on_its_way_and_an_older_answer_last_is_repaired() {
    let mut control = SaveControl::default();
    let older = patch(&control.commit(1, t("Anna", "kazateľka"), old(), false).1);
    control.commit(1, t("Anna", "pastorka"), old(), false);
    // The page hides: the desired texts go NOW beside the older save.
    let (commit, step) = control.commit(1, t("Anna", "pastorka"), old(), true);
    assert_eq!(commit, Commit::SentBeside);
    let newer = patch(&step);
    assert_eq!(newer.texts, t("Anna", "pastorka"));
    // The same urgent commit again sends nothing more.
    assert_eq!(
        control.commit(1, t("Anna", "pastorka"), old(), true).0,
        Commit::OnItsWay
    );

    // The newer one is answered first: the name is final, the role is not
    // (the older save on its way carries another role).
    let step = control
        .finish(1, newer.token, Saved, true)
        .unwrap_or_default();
    assert_eq!(step.patch, None);
    assert_eq!(
        settled(&step),
        Some(Settle::Fields {
            name: Some("Anna".to_string()),
            role: None
        })
    );
    // The older one last: the server may hold the older role, so the desired
    // texts go once more, and then the plate is idle.
    let step = control
        .finish(1, older.token, Saved, true)
        .unwrap_or_default();
    let again = patch(&step);
    assert_eq!(again.texts, t("Anna", "pastorka"));
    let step = control
        .finish(1, again.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle);
    assert_eq!(settled(&step), both("Anna", "pastorka"));
}

#[test]
fn zobrazit_on_an_idle_plate_shows_at_once_and_on_a_busy_one_waits_for_the_newest_texts() {
    let mut control = SaveControl::default();
    assert_eq!(control.show(1), (true, Step::default()));

    let first = patch(&control.commit(1, t("A1", "r"), old(), false).1);
    control.commit(1, t("A2", "r"), old(), false);
    let (now, step) = control.show(1);
    assert!(!now);
    assert_eq!(step, Step::default(), "nothing failed, nothing to retry");
    let step = control
        .finish(1, first.token, Saved, true)
        .unwrap_or_default();
    assert!(!step.show, "A1 is not the newest text");
    let second = patch(&step);
    let step = control
        .finish(1, second.token, Saved, true)
        .unwrap_or_default();
    assert!(step.show && step.idle, "A2 is saved: on air now");
}

#[test]
fn a_waiting_zobrazit_is_stale_after_another_show_or_hide_of_the_page() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.show(1);
    control.note_show(); // Skryť, or another Zobraziť
    let step = control
        .finish(1, sent.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && !step.show);
}

#[test]
fn a_failed_save_drops_the_waiting_zobrazit_and_zobrazit_sends_a_failed_save_again() {
    let mut control = SaveControl::default();
    let sent = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.show(1);
    let step = control
        .finish(1, sent.token, Failed, true)
        .unwrap_or_default();
    assert!(step.not_shown);

    // Zobraziť on the failed plate: the save goes again now, and the plate
    // goes on air once it is saved.
    let (now, step) = control.show(1);
    assert!(!now);
    let retry = patch(&step);
    let step = control
        .finish(1, retry.token, Saved, true)
        .unwrap_or_default();
    assert!(step.show && step.idle);
}

#[test]
fn only_failed_plates_with_nothing_on_their_way_wait_for_a_retry() {
    let mut control = SaveControl::default();
    let a = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let b = patch(&control.commit(2, t("B", "r"), old(), false).1);
    control.commit(3, t("C", "r"), old(), false);
    control.finish(2, b.token, Failed, true);
    control.finish(1, a.token, Failed, true);
    assert_eq!(control.failed(), vec![1, 2]);
    control.retry(1);
    assert_eq!(control.failed(), vec![2]);
}

#[test]
fn an_answer_for_an_unknown_save_changes_nothing() {
    let mut control = SaveControl::default();
    assert_eq!(control.finish(7, 1, Saved, true), None);
    let sent = patch(&control.commit(7, t("A", "r"), old(), false).1);
    assert_eq!(control.finish(7, sent.token + 5, Saved, true), None);
    assert!(control.is_busy(7));
}

#[test]
fn an_older_save_failing_after_the_newer_one_saved_sends_the_newest_once_more_then_shows() {
    // Review findings: the page hid, the newest texts went beside the older
    // save and were saved first; the older one then failed. A failed PATCH
    // may have been applied all the same (a lost answer, the 15 s abort
    // after the server wrote it), so the server may hold the OLDER texts:
    // the newest go once more, and a waiting Zobraziť goes on air only once
    // they are saved again. The failure itself is not toasted (the newest
    // texts are on their way; their answer reports).
    let mut control = SaveControl::default();
    let older = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let newer = patch(&control.commit(1, t("B", "r"), old(), true).1);
    let (now, _) = control.show(1);
    assert!(!now);
    let step = control
        .finish(1, newer.token, Saved, true)
        .unwrap_or_default();
    assert!(!step.show);
    let step = control
        .finish(1, older.token, Failed, true)
        .unwrap_or_default();
    let again = patch(&step);
    assert_eq!(again.texts, t("B", "r"));
    assert!(!step.show && !step.not_shown && !step.idle);
    assert!(step.retry_in.is_none());
    assert!(!Failed.is_reported(step.on_its_way));
    // A failure with nothing on its way after it (it waits for a retry, or
    // its field is gone) is reported; so are a gone plate and a refused
    // text.
    assert!(Failed.is_reported(false));
    assert!(Gone.is_reported(true));
    assert!(Refused.is_reported(true));
    let step = control
        .finish(1, again.token, Saved, true)
        .unwrap_or_default();
    assert!(
        step.show && step.idle,
        "on air once the newest texts are saved"
    );
    assert_eq!(settled(&step), both("B", "r"));
    assert!(!control.is_unsaved(1));
    assert!(!control.is_busy(1));
}

#[test]
fn typed_back_while_its_save_was_on_its_way_and_that_save_failing_sends_the_texts_once_more() {
    // Review finding: B on its way, typed back to A (the confirmed texts),
    // then B fails. A failed PATCH may have been applied all the same (a
    // lost answer, the 15 s abort after the server wrote it), so the server
    // may hold B: A goes once more. The failure is not toasted: A's own
    // answer reports.
    let mut control = SaveControl::default();
    let a = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.finish(1, a.token, Saved, true);
    let b = patch(&control.commit(1, t("B", "r"), t("A", "r"), false).1);
    assert_eq!(
        control.commit(1, t("A", "r"), t("A", "r"), false).0,
        Commit::Waits
    );
    let step = control.finish(1, b.token, Failed, true).unwrap_or_default();
    let again = patch(&step);
    assert_eq!(again.texts, t("A", "r"));
    assert!(step.on_its_way);
    assert!(!Failed.is_reported(step.on_its_way));
    let step = control
        .finish(1, again.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none(), "sent once");
    assert_eq!(settled(&step), both("A", "r"));
    assert!(!control.is_unsaved(1));
}

#[test]
fn an_older_save_failing_while_the_save_beside_it_is_still_on_its_way_reports_nothing() {
    // The answer of the save still on its way decides the plate (and
    // reports a failure of its own), so this failure is not toasted.
    let mut control = SaveControl::default();
    let older = patch(&control.commit(1, t("A", "r"), old(), false).1);
    let newer = patch(&control.commit(1, t("B", "r"), old(), true).1);
    let step = control
        .finish(1, older.token, Failed, true)
        .unwrap_or_default();
    assert!(step.on_its_way);
    assert!(step.patch.is_none() && step.retry_in.is_none() && !step.idle);
    assert!(!Failed.is_reported(step.on_its_way));
    let step = control
        .finish(1, newer.token, Saved, true)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none());
    assert!(!control.is_unsaved(1));
    assert!(!control.is_busy(1));
}

#[test]
fn a_plate_with_no_entry_starts_no_patch() {
    // Review finding: `send` / `advance` on a plate with no entry built a
    // PATCH with an empty name and role.
    let mut control = SaveControl::default();
    assert_eq!(control.send(9), None);
    assert_eq!(control.advance(9), Step::default());
    assert!(!control.is_busy(9));
}

#[test]
fn a_save_whose_row_left_goes_at_once_beside_the_one_on_its_way_when_the_page_hides() {
    // Review finding: an editor output switch takes the row away while a
    // PATCH is on its way; the row's commit waits behind it, and with no row
    // left nothing sent it when the page hid (a closed tab lost it).
    let mut control = SaveControl::default();
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    assert_eq!(
        control.commit(1, t("A", "pastor"), old(), false).0,
        Commit::Waits
    );
    // Plate 2: its newest texts are on their way already, nothing waits.
    control.commit(2, t("B", "r"), old(), false);
    // A plate with a row on the page: that row commits urgently itself.
    assert!(control.send_waiting(|_| true).is_empty());
    let sent = control.send_waiting(|id| id != 1);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let (id, step) = &sent[0];
    assert_eq!(*id, 1);
    let beside = patch(step);
    assert_eq!(beside.texts, t("A", "pastor"));
    // Sent once: the page hiding again sends nothing more.
    assert!(control.send_waiting(|_| false).is_empty());
    // Both answers back in order: the plate is idle on the newest texts.
    control.finish(1, first.token, Saved, false);
    let step = control
        .finish(1, beside.token, Saved, false)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none());
    assert!(!control.is_busy(1));
}

#[test]
fn a_rebuilt_row_sends_only_its_own_plates_waiting_save_and_only_once_when_the_page_hides() {
    // Review finding: the editor switched A → B → A while save 1 was on its
    // way. The plate is listed again, but its new row has no drafts while
    // the old row's texts still wait here: the list's `send_waiting` skips
    // a listed plate and the row had nothing to commit. The row now asks
    // for its own plate only (`has_row` = every other plate).
    let mut control = SaveControl::default();
    control.commit(1, t("A", "r"), old(), false);
    control.commit(1, t("A", "pastor"), old(), false);
    // Plate 2 waits too: its own row (or the list) sends it, never plate 1's.
    control.commit(2, t("B", "r"), old(), false);
    control.commit(2, t("B", "hosť"), old(), false);
    let sent = control.send_waiting(|other| other != 1);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, 1);
    assert_eq!(patch(&sent[0].1).texts, t("A", "pastor"));
    // Hidden, then `pagehide`: the row asks again, the list skips its listed
    // plates. Plate 1 went once.
    assert!(control.send_waiting(|other| other != 1).is_empty());
    assert!(control.send_waiting(|_| true).is_empty());
    assert_eq!(
        control.desired(2),
        Some(t("B", "hosť")),
        "plate 2 still waits"
    );
}

/// Plate 1's row has left the list (an editor output switch) while save 1
/// was on its way; its commit waits, and the page hiding sends it beside
/// save 1 (save 2). Returns (save 1, save 2).
fn rowless_save_sent_beside(control: &mut SaveControl) -> (Patch, Patch) {
    let first = patch(&control.commit(1, t("A", "r"), old(), false).1);
    control.commit(1, t("A", "pastor"), old(), false);
    let sent = control.send_waiting(|_| false);
    assert_eq!(sent.len(), 1, "{sent:?}");
    (first, patch(&sent[0].1))
}

#[test]
fn a_failure_whose_row_left_waits_for_the_save_sent_beside_it_and_a_saved_answer_ends_it() {
    // Review finding: save 1 failing with no row gave up at once. It toasted
    // „skúste ju zadať znova" and dropped save 2, which the server may well
    // have saved: the operator was told to retype text the server has.
    let mut control = SaveControl::default();
    let (first, beside) = rowless_save_sent_beside(&mut control);
    let step = control
        .finish(1, first.token, Failed, false)
        .unwrap_or_default();
    assert!(step.on_its_way, "save 2 decides: {step:?}");
    assert!(!Failed.is_reported(step.on_its_way));
    assert!(!step.idle && step.patch.is_none() && step.retry_in.is_none());
    assert!(control.is_busy(1));
    let step = control
        .finish(1, beside.token, Saved, false)
        .unwrap_or_default();
    assert!(step.idle && step.patch.is_none(), "{step:?}");
    assert!(!step.not_shown);
    assert!(!control.is_busy(1));
}

#[test]
fn a_failure_whose_row_left_gives_up_once_the_save_sent_beside_it_fails_too() {
    let mut control = SaveControl::default();
    let (first, beside) = rowless_save_sent_beside(&mut control);
    control.finish(1, first.token, Failed, false);
    let step = control
        .finish(1, beside.token, Failed, false)
        .unwrap_or_default();
    // Nothing is on its way and no field holds the text: the give-up toast.
    assert!(
        step.idle && !step.on_its_way && step.retry_in.is_none(),
        "{step:?}"
    );
    assert!(Failed.is_reported(step.on_its_way));
    assert_eq!(TextLeft::of(false, step.not_shown), TextLeft::Nowhere);
    assert!(!control.is_busy(1));
}

// ---- Outcomes and toasts --------------------------------------------------

fn status(code: u16) -> ApiError {
    ApiError::Status(code, "x".to_string())
}

#[test]
fn only_a_network_error_or_a_server_fault_keeps_the_text_for_another_try() {
    let lost = ApiError::Network(gloo_net::Error::GlooError("Failed to fetch".to_string()));
    assert_eq!(SaveOutcome::of_error(&lost), Failed);
    for code in [500, 502, 503, 504, 408, 429] {
        assert_eq!(SaveOutcome::of_error(&status(code)), Failed, "{code}");
    }
    for code in [400, 409, 422] {
        assert_eq!(SaveOutcome::of_error(&status(code)), Refused, "{code}");
    }
    assert_eq!(
        SaveOutcome::of_error(&ApiError::Refused(422, "Meno".to_string())),
        Refused
    );
}

#[test]
fn an_expired_login_is_not_the_texts_fault_so_the_text_is_kept() {
    // Behind Cloudflare Access an expired session answers 401 / 403: after a
    // new login the same text saves fine.
    for code in [401, 403] {
        assert_eq!(SaveOutcome::of_error(&status(code)), Failed, "{code}");
    }
}

#[test]
fn a_404_means_the_plate_was_deleted_elsewhere() {
    assert_eq!(SaveOutcome::of_error(&status(404)), Gone);
    assert_eq!(
        SaveOutcome::of_error(&ApiError::Refused(404, "x".to_string())),
        Gone
    );
    assert_eq!(
        Gone.message(&status(404), TextLeft::InField, true),
        "Menovka bola medzičasom zmazaná — zmena sa neuložila."
    );
    assert_eq!(
        Gone.message(&status(404), TextLeft::InFieldNotShown, true),
        "Menovka sa nezobrazila. Menovka bola medzičasom zmazaná — zmena sa neuložila."
    );
}

#[test]
fn a_failed_save_says_the_text_stayed_and_a_refused_one_says_why() {
    let failed = Failed.message(&status(503), TextLeft::InField, true);
    assert!(
        failed.starts_with("Uloženie zlyhalo: HTTP 503: x"),
        "{failed}"
    );
    assert!(failed.contains("Text ostal v poli"), "{failed}");
    // It goes again by itself, not only when the field is left.
    assert!(failed.contains("uloží sa znova sám"), "{failed}");
    assert!(!failed.contains("Prihlásenie"), "{failed}");
    assert_eq!(
        Refused.message(&status(422), TextLeft::InField, true),
        "Uloženie zlyhalo: HTTP 422: x"
    );
}

#[test]
fn a_failed_save_with_no_retry_timer_left_does_not_promise_one() {
    // Review finding: from the 4th failure in a row no timer is armed, yet
    // the toast said „uloží sa znova sám o chvíľu".
    let timed = Failed.message(&status(503), TextLeft::InField, true);
    assert!(timed.contains("uloží sa znova sám o chvíľu"), "{timed}");
    let untimed = Failed.message(&status(503), TextLeft::InField, false);
    assert!(untimed.contains("Text ostal v poli"), "{untimed}");
    assert!(!untimed.contains("sám"), "{untimed}");
    // The other ways it goes again are named either way.
    for when in ["siete", "návrate na stránku", "Zobraziť", "pole opustíte"] {
        assert!(timed.contains(when), "{when}: {timed}");
        assert!(untimed.contains(when), "{when}: {untimed}");
    }
    let not_shown = Failed.message(&status(503), TextLeft::InFieldNotShown, false);
    assert!(
        not_shown.starts_with("Menovka sa nezobrazila. "),
        "{not_shown}"
    );
    assert!(!not_shown.contains("sám"), "{not_shown}");
}

#[test]
fn an_expired_login_says_to_log_in_again_without_losing_the_text() {
    let message = Failed.message(&status(401), TextLeft::InField, true);
    assert!(message.contains("Prihlásenie mohlo vypršať"), "{message}");
    assert!(message.contains("Text ostal v poli"), "{message}");
}

#[test]
fn a_failed_save_whose_field_is_gone_says_to_type_the_change_again() {
    let message = Failed.message(&status(503), TextLeft::of(false, true), true);
    assert!(message.starts_with(LOST_UNSAVED), "{message}");
    assert!(!message.contains("Text ostal v poli"), "{message}");
}

#[test]
fn a_failed_save_a_zobrazit_waited_for_says_the_plate_was_not_shown() {
    let message = Failed.message(&status(503), TextLeft::of(true, true), true);
    assert!(
        message.starts_with("Menovka sa nezobrazila. Uloženie zlyhalo: HTTP 503: x"),
        "{message}"
    );
    assert!(message.contains("Text ostal v poli"), "{message}");
    assert_eq!(TextLeft::of(true, false), TextLeft::InField);
}
