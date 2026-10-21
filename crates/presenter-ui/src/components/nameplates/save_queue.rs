//! One save in flight per plate, and how a save ends (#871 review), pure and
//! host-tested.
//!
//! A row saves on blur, and two saves of one plate can follow each other
//! within one round trip (Tab from the name to the role, then leave the
//! role). Sent at once, a server that applies them out of order keeps the
//! EARLIER text. So, like the reorder PUTs (`OrderSync` in `mod.rs`), a
//! plate's saves go one at a time: while one is on its way, the newest save
//! waits and goes once that one is done. A save carries both fields as they
//! are when it is made, so a newer waiting save replaces an older one. When
//! the page is being hidden or closed, a waiting save leaves the queue and
//! goes at once ([`SaveQueue::take_waiting`]).
//!
//! [`SaveOutcome`] is what a row learns about its save: only a network error
//! or a server fault keeps the typed text for another try.

use std::collections::hash_map::{Entry, HashMap};

use crate::api::ApiError;

/// The toast for typed text whose plate was deleted elsewhere: from the row
/// that leaves the list, and from a save the server answers with 404.
pub const DELETED_UNSAVED: &str = "Menovka bola medzičasom zmazaná — zmena sa neuložila.";

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
    /// lock) or a server fault (5xx, 408, 429). The row KEEPS its drafts and
    /// sends them again on its next save (leaving the field, Enter, the page
    /// back in view).
    Failed,
}

impl SaveOutcome {
    /// The outcome of a save that came back with `error`.
    pub fn of_error(error: &ApiError) -> Self {
        match error {
            ApiError::Status(404, _) | ApiError::Refused(404, _) | ApiError::NotFound(_) => {
                Self::Gone
            }
            ApiError::Status(code, _) | ApiError::Refused(code, _) => {
                if *code >= 500 || *code == 408 || *code == 429 {
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
    /// [`Self::of_error`] of it).
    pub fn message(self, error: &ApiError) -> String {
        match self {
            Self::Gone => DELETED_UNSAVED.to_string(),
            Self::Failed => format!(
                "Uloženie zlyhalo: {error}. Text ostal v poli — uloží sa znova, keď pole opustíte."
            ),
            Self::Saved | Self::Refused => format!("Uloženie zlyhalo: {error}"),
        }
    }
}

/// The saves waiting behind the one on its way, per plate.
pub struct SaveQueue<J> {
    /// Plate id → its waiting save. A plate is a key exactly while a save of
    /// it is on its way.
    waiting: HashMap<i64, Option<J>>,
}

impl<J> Default for SaveQueue<J> {
    fn default() -> Self {
        Self {
            waiting: HashMap::new(),
        }
    }
}

impl<J> SaveQueue<J> {
    /// Offer a save of plate `id`: `Some(job)` = send it now (nothing of that
    /// plate is on its way), `None` = it waits for the one on its way. An
    /// older waiting save is dropped (the newer one has the same or newer
    /// texts, and settles the drafts when it is done).
    pub fn offer(&mut self, id: i64, job: J) -> Option<J> {
        match self.waiting.entry(id) {
            Entry::Occupied(mut slot) => {
                *slot.get_mut() = Some(job);
                None
            }
            Entry::Vacant(slot) => {
                slot.insert(None);
                Some(job)
            }
        }
    }

    /// The save of plate `id` on its way is done: the waiting save to send
    /// next (the plate stays busy), or `None` (the plate is idle again).
    pub fn finish(&mut self, id: i64) -> Option<J> {
        let next = self.waiting.get_mut(&id)?.take();
        if next.is_none() {
            self.waiting.remove(&id);
        }
        next
    }

    /// The page is being hidden or closed: the waiting save of plate `id`,
    /// to send NOW beside the one on its way (a hidden page may be frozen or
    /// closed before that one is done, and the waiting save would never go
    /// out). The plate stays busy until the one on its way is done, which
    /// then finds nothing waiting.
    pub fn take_waiting(&mut self, id: i64) -> Option<J> {
        self.waiting.get_mut(&id)?.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_save_of_a_plate_goes_at_once_and_other_plates_never_wait() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "a1"), Some("a1"));
        assert_eq!(queue.offer(2, "b1"), Some("b1"));
    }

    #[test]
    fn a_save_made_while_one_is_on_its_way_waits_and_goes_next() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "first"), Some("first"));
        assert_eq!(queue.offer(1, "second"), None);
        assert_eq!(queue.finish(1), Some("second"));
        assert_eq!(queue.finish(1), None);
        // Idle again: the next save goes at once.
        assert_eq!(queue.offer(1, "third"), Some("third"));
    }

    #[test]
    fn only_the_newest_waiting_save_is_sent() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "first"), Some("first"));
        assert_eq!(queue.offer(1, "second"), None);
        assert_eq!(queue.offer(1, "third"), None);
        assert_eq!(queue.finish(1), Some("third"));
        assert_eq!(queue.finish(1), None);
    }

    #[test]
    fn a_plate_waits_only_for_its_own_save() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "a1"), Some("a1"));
        assert_eq!(queue.offer(1, "a2"), None);
        assert_eq!(queue.offer(2, "b1"), Some("b1"));
        assert_eq!(queue.finish(2), None);
        assert_eq!(queue.finish(1), Some("a2"));
    }

    #[test]
    fn finishing_a_plate_with_nothing_on_its_way_sends_nothing() {
        let mut queue: SaveQueue<&str> = SaveQueue::default();
        assert_eq!(queue.finish(7), None);
        assert_eq!(queue.offer(7, "a"), Some("a"));
    }

    #[test]
    fn a_hidden_page_takes_the_waiting_save_out_to_send_it_at_once() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "first"), Some("first"));
        assert_eq!(queue.offer(1, "second"), None);
        assert_eq!(queue.take_waiting(1), Some("second"));
        // Taken once: the save on its way then finds nothing waiting.
        assert_eq!(queue.take_waiting(1), None);
        assert_eq!(queue.finish(1), None);
        // Idle again: the next save goes at once.
        assert_eq!(queue.offer(1, "third"), Some("third"));
    }

    #[test]
    fn the_plate_stays_busy_after_its_waiting_save_was_taken_out() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(1, "first"), Some("first"));
        assert_eq!(queue.offer(1, "second"), None);
        assert_eq!(queue.take_waiting(1), Some("second"));
        // The first is still on its way: a newer save waits for it.
        assert_eq!(queue.offer(1, "third"), None);
        assert_eq!(queue.finish(1), Some("third"));
    }

    #[test]
    fn nothing_is_taken_out_of_an_idle_plate_or_one_with_nothing_waiting() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.take_waiting(3), None);
        assert_eq!(queue.offer(3, "a"), Some("a"));
        assert_eq!(queue.take_waiting(3), None);
        assert_eq!(queue.finish(3), None);
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
            SaveOutcome::Gone.message(&status(404)),
            "Menovka bola medzičasom zmazaná — zmena sa neuložila."
        );
    }

    #[test]
    fn a_failed_save_says_the_text_stayed_and_a_refused_one_says_why() {
        let failed = SaveOutcome::Failed.message(&status(503));
        assert!(
            failed.starts_with("Uloženie zlyhalo: HTTP 503: x"),
            "{failed}"
        );
        assert!(failed.contains("Text ostal v poli"), "{failed}");
        assert_eq!(
            SaveOutcome::Refused.message(&status(422)),
            "Uloženie zlyhalo: HTTP 422: x"
        );
    }
}
