//! A plate's saves in order, and how a save ends (#871 review), pure and
//! host-tested.
//!
//! A row saves on blur, and two saves of one plate can follow each other
//! within one round trip (Tab from the name to the role, then leave the
//! role). Sent at once, a server that applies them out of order keeps the
//! EARLIER text. So, like the reorder PUTs (`OrderSync` in `mod.rs`), a
//! plate's saves go one at a time: while one is on its way, the newest save
//! waits and goes once nothing of that plate is on its way any more. A save
//! carries both fields as they are when it is made, so a newer waiting save
//! replaces an older one.
//!
//! When the page is being hidden or closed, a waiting save leaves the queue
//! and goes at once ([`SaveQueue::take_waiting`]), beside the one on its way.
//! The plate then has TWO saves on their way, and stays busy until both are
//! back. "Sent last" is not "applied last": when the older one is answered
//! after the newer one, the server may hold the older texts, so the newest
//! texts are sent once more ([`Next::Send`] with no `done`).
//!
//! [`SaveOutcome`] is what a row learns about its save: only a network error,
//! a server fault or an expired login keeps the typed text for another try.

use std::collections::HashMap;

use crate::api::ApiError;

/// The toast for typed text whose plate was deleted elsewhere: from the row
/// that leaves the list, and from a save the server answers with 404.
pub const DELETED_UNSAVED: &str = "Menovka bola medzičasom zmazaná — zmena sa neuložila.";

/// The toast for a failed save whose text no field holds any more: its row
/// left the list (an output switch, the page closed) or it was a re-send.
pub const LOST_UNSAVED: &str = "Zmena menovky sa neuložila — skúste ju zadať znova.";

/// How a plate save ended, for the row that made it (#871 review).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The server has the texts.
    Saved,
    /// Refused for good: an empty name (checked before sending) or a 4xx for
    /// the texts. The row drops its drafts; the field shows the saved text.
    Refused,
    /// The plate is gone (404: deleted elsewhere). The store has said so
    /// ([`DELETED_UNSAVED`]); the row drops its drafts.
    Gone,
    /// Not saved this time: a network error (a phone's Wi-Fi asleep after a
    /// lock), a server fault (5xx, 408, 429) or an expired login (401, 403:
    /// Cloudflare Access). The row KEEPS its drafts, marks itself
    /// „neuložené" and sends them again.
    Failed,
}

/// Where the typed text of a failed save is now (#871 review), for its
/// toast. The row that made the save answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextLeft {
    /// Still in its field, marked „neuložené", sent again later.
    InField,
    /// Still in its field, and the Zobraziť that waited for the save did not
    /// put the plate on air.
    InFieldNotShown,
    /// In no field: its row left the list, or the save was a re-send.
    Nowhere,
}

impl SaveOutcome {
    /// The outcome of a save that came back with `error`.
    pub fn of_error(error: &ApiError) -> Self {
        match error {
            ApiError::Status(404, _) | ApiError::Refused(404, _) | ApiError::NotFound(_) => {
                Self::Gone
            }
            ApiError::Status(code, _) | ApiError::Refused(code, _) => {
                if *code >= 500 || matches!(*code, 401 | 403 | 408 | 429) {
                    Self::Failed
                } else {
                    Self::Refused
                }
            }
            // A 2xx whose body did not parse: the text may well be saved, and
            // sending it again is harmless, so it is kept like a lost answer.
            ApiError::Network(_) | ApiError::Deserialize(_) => Self::Failed,
            ApiError::Serialize(_) => Self::Refused,
        }
    }

    /// The toast for a save that came back with `error` (`self` is
    /// [`Self::of_error`] of it); `left` = where its typed text is now.
    pub fn message(self, error: &ApiError, left: TextLeft) -> String {
        match (self, left) {
            (Self::Gone, _) => DELETED_UNSAVED.to_string(),
            (Self::Failed, TextLeft::Nowhere) => format!("{LOST_UNSAVED} ({error})"),
            (Self::Failed, _) => {
                let not_shown = if left == TextLeft::InFieldNotShown {
                    "Menovka sa nezobrazila. "
                } else {
                    ""
                };
                let login = if is_login_error(error) {
                    " Prihlásenie mohlo vypršať — prihláste sa znova v novej karte."
                } else {
                    ""
                };
                format!(
                    "{not_shown}Uloženie zlyhalo: {error}.{login} Text ostal v poli ako \
                     neuložený — uloží sa znova sám o chvíľu, po návrate na stránku alebo \
                     keď pole opustíte."
                )
            }
            (Self::Saved | Self::Refused, _) => format!("Uloženie zlyhalo: {error}"),
        }
    }
}

/// An expired login (Cloudflare Access answers 401 / 403).
fn is_login_error(error: &ApiError) -> bool {
    matches!(
        error,
        ApiError::Status(401 | 403, _) | ApiError::Refused(401 | 403, _)
    )
}

/// The pause before the automatic retry of a failed save, after `failures`
/// failures in a row (#871 review): 2 s, 5 s, 15 s, then none. A phone that
/// unlocks fires `visibilitychange` before its Wi-Fi is back, so one retry
/// then is not enough; after three, the next try waits for the network or the
/// page coming back, or the field being left.
pub fn retry_delay_ms(failures: u32) -> Option<u32> {
    match failures {
        1 => Some(2_000),
        2 => Some(5_000),
        3 => Some(15_000),
        _ => None,
    }
}

/// A save to send now: its number in the queue, its texts, and what to run
/// once it is done (`None` for a re-send of the newest texts).
#[derive(Debug, PartialEq)]
pub struct Job<T, D> {
    pub seq: u64,
    pub texts: T,
    pub done: Option<D>,
}

/// What [`SaveQueue::finish`] asks for next.
#[derive(Debug, PartialEq)]
pub enum Next<T, D> {
    /// Send this save: the one waiting, or the newest texts once more.
    Send(Job<T, D>),
    /// Another save of the plate is still on its way.
    Busy,
    /// Nothing of the plate is left to send.
    Idle,
}

/// One plate's saves.
struct Plate<T, D> {
    /// Saves on their way: one, or more after a hidden page sent its waiting
    /// save beside them.
    in_flight: u32,
    /// The newest save made meanwhile.
    waiting: Option<(T, D)>,
    /// The number and texts of the newest save sent.
    newest: u64,
    newest_texts: T,
    /// The newest save came back saved.
    newest_saved: bool,
    /// An older save came back saved after it: the server may hold the older
    /// texts.
    overtaken: bool,
}

/// The saves of every plate on its way, and the ones waiting for them.
pub struct SaveQueue<T, D> {
    plates: HashMap<i64, Plate<T, D>>,
    last_seq: u64,
}

impl<T, D> Default for SaveQueue<T, D> {
    fn default() -> Self {
        Self {
            plates: HashMap::new(),
            last_seq: 0,
        }
    }
}

impl<T: Clone, D> SaveQueue<T, D> {
    fn next_seq(&mut self) -> u64 {
        self.last_seq += 1;
        self.last_seq
    }

    /// Offer a save of plate `id`: `Some(job)` = send it now (nothing of that
    /// plate is on its way), `None` = it waits. An older waiting save is
    /// dropped WITHOUT its `done` (the newer one has the same or newer texts,
    /// and settles the drafts when it is done).
    pub fn offer(&mut self, id: i64, texts: T, done: D) -> Option<Job<T, D>> {
        if let Some(plate) = self.plates.get_mut(&id) {
            plate.waiting = Some((texts, done));
            return None;
        }
        let seq = self.next_seq();
        self.plates.insert(
            id,
            Plate {
                in_flight: 1,
                waiting: None,
                newest: seq,
                newest_texts: texts.clone(),
                newest_saved: false,
                overtaken: false,
            },
        );
        Some(Job {
            seq,
            texts,
            done: Some(done),
        })
    }

    /// The page is being hidden or closed: the waiting save of plate `id`,
    /// to send NOW beside the one on its way (a hidden page may be frozen or
    /// closed before that one is done, and the waiting save would never go
    /// out). The plate stays busy until both are back.
    pub fn take_waiting(&mut self, id: i64) -> Option<Job<T, D>> {
        let (texts, done) = self.plates.get_mut(&id)?.waiting.take()?;
        let seq = self.next_seq();
        let plate = self.plates.get_mut(&id)?;
        plate.in_flight += 1;
        Some(plate.sent(seq, texts, Some(done)))
    }

    /// Save `seq` of plate `id` is back with `outcome`: what to send next.
    ///
    /// A 404 drops the waiting save (it could only be refused again). Once
    /// nothing of the plate is on its way, the waiting save goes; else, when
    /// an older save was answered saved after the newest one, the newest
    /// texts go once more; else the plate is idle.
    pub fn finish(&mut self, id: i64, seq: u64, outcome: SaveOutcome) -> Next<T, D> {
        let next = {
            let Some(plate) = self.plates.get_mut(&id) else {
                return Next::Idle;
            };
            plate.in_flight = plate.in_flight.saturating_sub(1);
            match outcome {
                SaveOutcome::Saved if seq == plate.newest => plate.newest_saved = true,
                SaveOutcome::Saved if plate.newest_saved => plate.overtaken = true,
                SaveOutcome::Gone => plate.waiting = None,
                _ => {}
            }
            if plate.in_flight > 0 {
                return Next::Busy;
            }
            match plate.waiting.take() {
                Some((texts, done)) => Some((texts, Some(done))),
                None if plate.overtaken => Some((plate.newest_texts.clone(), None)),
                None => None,
            }
        };
        let Some((texts, done)) = next else {
            self.plates.remove(&id);
            return Next::Idle;
        };
        let seq = self.next_seq();
        let Some(plate) = self.plates.get_mut(&id) else {
            return Next::Idle;
        };
        plate.in_flight = 1;
        Next::Send(plate.sent(seq, texts, done))
    }
}

impl<T: Clone, D> Plate<T, D> {
    /// Save `seq` with `texts` goes now, as the plate's newest.
    fn sent(&mut self, seq: u64, texts: T, done: Option<D>) -> Job<T, D> {
        self.newest = seq;
        self.newest_texts = texts.clone();
        self.newest_saved = false;
        self.overtaken = false;
        Job { seq, texts, done }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::SaveOutcome::{Failed, Gone, Saved};

    /// The job a test expects: number, texts, and whether it has a `done`
    /// (the `done` stands for the row that made the save).
    fn job(seq: u64, texts: &'static str, done: Option<char>) -> Job<&'static str, char> {
        Job { seq, texts, done }
    }

    #[test]
    fn the_first_save_of_a_plate_goes_at_once_and_other_plates_never_wait() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "a1", 'a'), Some(job(1, "a1", Some('a'))));
        assert_eq!(queue.offer(2, "b1", 'b'), Some(job(2, "b1", Some('b'))));
    }

    #[test]
    fn a_save_made_while_one_is_on_its_way_waits_and_goes_next() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert_eq!(
            queue.finish(1, 1, Saved),
            Next::Send(job(2, "second", Some('s')))
        );
        assert_eq!(queue.finish(1, 2, Saved), Next::Idle);
        // Idle again: the next save goes at once.
        assert_eq!(
            queue.offer(1, "third", 't'),
            Some(job(3, "third", Some('t')))
        );
    }

    #[test]
    fn only_the_newest_waiting_save_is_sent() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert_eq!(queue.offer(1, "third", 't'), None);
        assert_eq!(
            queue.finish(1, 1, Saved),
            Next::Send(job(2, "third", Some('t')))
        );
        assert_eq!(queue.finish(1, 2, Saved), Next::Idle);
    }

    #[test]
    fn a_plate_waits_only_for_its_own_save() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "a1", 'a').is_some());
        assert_eq!(queue.offer(1, "a2", 'a'), None);
        assert_eq!(queue.offer(2, "b1", 'b'), Some(job(2, "b1", Some('b'))));
        assert_eq!(queue.finish(2, 2, Saved), Next::Idle);
        assert_eq!(
            queue.finish(1, 1, Saved),
            Next::Send(job(3, "a2", Some('a')))
        );
    }

    #[test]
    fn a_failed_or_refused_save_still_lets_the_waiting_one_go() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert_eq!(
            queue.finish(1, 1, Failed),
            Next::Send(job(2, "second", Some('s')))
        );
        assert_eq!(queue.offer(1, "third", 't'), None);
        assert_eq!(
            queue.finish(1, 2, SaveOutcome::Refused),
            Next::Send(job(3, "third", Some('t')))
        );
    }

    #[test]
    fn finishing_a_plate_with_nothing_on_its_way_sends_nothing() {
        let mut queue: SaveQueue<&str, char> = SaveQueue::default();
        assert_eq!(queue.finish(7, 1, Saved), Next::Idle);
        assert!(queue.offer(7, "a", 'a').is_some());
    }

    #[test]
    fn a_hidden_page_takes_the_waiting_save_out_to_send_it_at_once() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert_eq!(queue.take_waiting(1), Some(job(2, "second", Some('s'))));
        // Taken once: nothing is left to take or to send after them.
        assert_eq!(queue.take_waiting(1), None);
        assert_eq!(queue.finish(1, 1, Saved), Next::Busy);
        assert_eq!(queue.finish(1, 2, Saved), Next::Idle);
        // Idle again: the next save goes at once.
        assert!(queue.offer(1, "third", 't').is_some());
    }

    #[test]
    fn the_plate_stays_busy_until_both_saves_are_back_after_one_was_taken_out() {
        // #871 review: the first answer used to mark the plate idle while the
        // save taken out was still on its way, and the next save went beside
        // it.
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert!(queue.take_waiting(1).is_some());
        assert_eq!(queue.finish(1, 1, Saved), Next::Busy);
        // The second is still on its way: a newer save waits for it.
        assert_eq!(queue.offer(1, "third", 't'), None);
        assert_eq!(
            queue.finish(1, 2, Saved),
            Next::Send(job(3, "third", Some('t')))
        );
        assert_eq!(queue.finish(1, 3, Saved), Next::Idle);
    }

    #[test]
    fn an_older_save_answered_after_the_newest_one_sends_the_newest_texts_once_more() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "older", 'o').is_some());
        assert_eq!(queue.offer(1, "newer", 'n'), None);
        assert!(queue.take_waiting(1).is_some());
        // The newer one is answered first, the older one after it: the server
        // may have applied the older texts last.
        assert_eq!(queue.finish(1, 2, Saved), Next::Busy);
        assert_eq!(queue.finish(1, 1, Saved), Next::Send(job(3, "newer", None)));
        // Once: the re-send itself is the newest and goes alone.
        assert_eq!(queue.finish(1, 3, Saved), Next::Idle);
    }

    #[test]
    fn no_re_send_when_the_newest_save_was_answered_last_or_was_not_saved() {
        // In order: the newest is the last applied.
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "older", 'o').is_some());
        assert_eq!(queue.offer(1, "newer", 'n'), None);
        assert!(queue.take_waiting(1).is_some());
        assert_eq!(queue.finish(1, 1, Saved), Next::Busy);
        assert_eq!(queue.finish(1, 2, Saved), Next::Idle);

        // The older one failed: the server never had its texts.
        assert!(queue.offer(2, "older", 'o').is_some());
        assert_eq!(queue.offer(2, "newer", 'n'), None);
        assert!(queue.take_waiting(2).is_some());
        assert_eq!(queue.finish(2, 4, Saved), Next::Busy);
        assert_eq!(queue.finish(2, 3, Failed), Next::Idle);

        // The newest failed: its row keeps the text and sends it again.
        assert!(queue.offer(3, "older", 'o').is_some());
        assert_eq!(queue.offer(3, "newer", 'n'), None);
        assert!(queue.take_waiting(3).is_some());
        assert_eq!(queue.finish(3, 6, Failed), Next::Busy);
        assert_eq!(queue.finish(3, 5, Saved), Next::Idle);
    }

    #[test]
    fn a_waiting_save_goes_instead_of_a_re_send_it_makes_needless() {
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "older", 'o').is_some());
        assert_eq!(queue.offer(1, "newer", 'n'), None);
        assert!(queue.take_waiting(1).is_some());
        assert_eq!(queue.offer(1, "newest", 'w'), None);
        assert_eq!(queue.finish(1, 2, Saved), Next::Busy);
        assert_eq!(
            queue.finish(1, 1, Saved),
            Next::Send(job(3, "newest", Some('w')))
        );
        assert_eq!(queue.finish(1, 3, Saved), Next::Idle);
    }

    #[test]
    fn a_404_drops_the_save_waiting_behind_it() {
        // #871 review: it could only be refused again (a second 404 + toast).
        let mut queue = SaveQueue::default();
        assert!(queue.offer(1, "first", 'f').is_some());
        assert_eq!(queue.offer(1, "second", 's'), None);
        assert_eq!(queue.finish(1, 1, Gone), Next::Idle);
        assert!(queue.offer(1, "third", 't').is_some());
    }

    #[test]
    fn nothing_is_taken_out_of_an_idle_plate_or_one_with_nothing_waiting() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.take_waiting(3), None);
        assert!(queue.offer(3, "a", 'a').is_some());
        assert_eq!(queue.take_waiting(3), None);
        assert_eq!(queue.finish(3, 1, Saved), Next::Idle);
    }

    #[test]
    fn a_failed_save_is_retried_after_2_5_and_15_seconds_then_waits_for_the_operator() {
        assert_eq!(retry_delay_ms(1), Some(2_000));
        assert_eq!(retry_delay_ms(2), Some(5_000));
        assert_eq!(retry_delay_ms(3), Some(15_000));
        assert_eq!(retry_delay_ms(4), None);
        assert_eq!(retry_delay_ms(0), None);
    }

    fn status(code: u16) -> ApiError {
        ApiError::Status(code, "x".to_string())
    }

    #[test]
    fn only_a_network_error_or_a_server_fault_keeps_the_text_for_another_try() {
        let lost = ApiError::Network(gloo_net::Error::GlooError("Failed to fetch".to_string()));
        assert_eq!(SaveOutcome::of_error(&lost), SaveOutcome::Failed);
        for code in [500, 502, 503, 504, 408, 429] {
            assert_eq!(
                SaveOutcome::of_error(&status(code)),
                SaveOutcome::Failed,
                "{code}"
            );
        }
        for code in [400, 409, 422] {
            assert_eq!(
                SaveOutcome::of_error(&status(code)),
                SaveOutcome::Refused,
                "{code}"
            );
        }
        assert_eq!(
            SaveOutcome::of_error(&ApiError::Refused(422, "Meno".to_string())),
            SaveOutcome::Refused
        );
    }

    #[test]
    fn an_expired_login_is_not_the_texts_fault_so_the_text_is_kept() {
        // Behind Cloudflare Access an expired session answers 401 / 403:
        // after a new login the same text saves fine.
        for code in [401, 403] {
            assert_eq!(
                SaveOutcome::of_error(&status(code)),
                SaveOutcome::Failed,
                "{code}"
            );
        }
    }

    #[test]
    fn a_404_means_the_plate_was_deleted_elsewhere() {
        assert_eq!(SaveOutcome::of_error(&status(404)), SaveOutcome::Gone);
        assert_eq!(
            SaveOutcome::of_error(&ApiError::Refused(404, "x".to_string())),
            SaveOutcome::Gone
        );
        assert_eq!(
            SaveOutcome::Gone.message(&status(404), TextLeft::InField),
            "Menovka bola medzičasom zmazaná — zmena sa neuložila."
        );
    }

    #[test]
    fn a_failed_save_says_the_text_stayed_and_a_refused_one_says_why() {
        let failed = SaveOutcome::Failed.message(&status(503), TextLeft::InField);
        assert!(
            failed.starts_with("Uloženie zlyhalo: HTTP 503: x"),
            "{failed}"
        );
        assert!(failed.contains("Text ostal v poli"), "{failed}");
        // It goes again by itself, not only when the field is left.
        assert!(failed.contains("uloží sa znova sám"), "{failed}");
        assert!(!failed.contains("Prihlásenie"), "{failed}");
        assert_eq!(
            SaveOutcome::Refused.message(&status(422), TextLeft::InField),
            "Uloženie zlyhalo: HTTP 422: x"
        );
    }

    #[test]
    fn an_expired_login_says_to_log_in_again_without_losing_the_text() {
        let message = SaveOutcome::Failed.message(&status(401), TextLeft::InField);
        assert!(message.contains("Prihlásenie mohlo vypršať"), "{message}");
        assert!(message.contains("Text ostal v poli"), "{message}");
    }

    #[test]
    fn a_failed_save_whose_field_is_gone_says_to_type_the_change_again() {
        let message = SaveOutcome::Failed.message(&status(503), TextLeft::Nowhere);
        assert!(message.starts_with(LOST_UNSAVED), "{message}");
        assert!(!message.contains("Text ostal v poli"), "{message}");
    }

    #[test]
    fn a_failed_save_a_zobrazit_waited_for_says_the_plate_was_not_shown() {
        let message = SaveOutcome::Failed.message(&status(503), TextLeft::InFieldNotShown);
        assert!(
            message.starts_with("Menovka sa nezobrazila. Uloženie zlyhalo: HTTP 503: x"),
            "{message}"
        );
        assert!(message.contains("Text ostal v poli"), "{message}");
    }
}
