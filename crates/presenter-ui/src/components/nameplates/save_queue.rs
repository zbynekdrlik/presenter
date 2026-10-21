//! One save in flight per plate (#871 review), pure and host-tested.
//!
//! A row saves on blur, and two saves of one plate can follow each other
//! within one round trip (Tab from the name to the role, then leave the
//! role). Sent at once, a server that applies them out of order keeps the
//! EARLIER text. So, like the reorder PUTs (`OrderSync` in `mod.rs`), a
//! plate's saves go one at a time: while one is on its way, the newest save
//! waits and goes once that one is done. A save carries both fields as they
//! are when it is made, so a newer waiting save replaces an older one.

use std::collections::hash_map::{Entry, HashMap};

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
}
