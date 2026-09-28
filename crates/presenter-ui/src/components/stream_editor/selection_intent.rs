//! Monotonic selection-intent counter for the stream editor (#787 reopen).
//!
//! Every local selection change (pick / deselect / scene open / panel close /
//! the start of an element create) bumps the counter. An async action that
//! wants to select something when its response lands (e.g. `add_element`
//! selecting the element it created) captures a ticket BEFORE awaiting and
//! applies the selection only if the ticket is still current — so a late
//! response can never override a newer choice the operator made meanwhile.
//! Pure + host-tested; the ctx holds it in a `StoredValue`.

/// The counter. `Default` starts at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionIntent {
    seq: u64,
}

impl SelectionIntent {
    /// Record a new local selection intent; returns its ticket.
    pub fn bump(&mut self) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    /// True when no newer intent was recorded since `ticket` was issued.
    pub fn is_current(&self, ticket: u64) -> bool {
        self.seq == ticket
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_ticket_is_current() {
        let mut s = SelectionIntent::default();
        let t = s.bump();
        assert!(s.is_current(t));
    }

    #[test]
    fn a_newer_intent_invalidates_an_older_ticket() {
        let mut s = SelectionIntent::default();
        let add = s.bump();
        let pick = s.bump();
        assert!(!s.is_current(add), "late add response must not select");
        assert!(s.is_current(pick));
    }

    #[test]
    fn tickets_are_strictly_increasing() {
        let mut s = SelectionIntent::default();
        let a = s.bump();
        let b = s.bump();
        assert!(b > a);
    }

    #[test]
    fn default_state_is_not_current_for_a_future_ticket() {
        let s = SelectionIntent::default();
        assert!(!s.is_current(1));
        assert!(s.is_current(0));
    }
}
