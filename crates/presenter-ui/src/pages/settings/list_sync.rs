//! #819: keep a settings list card from applying list responses out of order.
//!
//! A card fetches its list from the 5 s poll AND after every save / delete / test.
//! A poll sent just before a save can arrive after the save's own reload: applied,
//! it would show the old values again for up to 5 s and could close the editor the
//! operator just opened on a row the stale list does not know yet. Every request is
//! numbered; a response older than the last one applied is dropped.

use leptos::prelude::*;

/// A card's list-request counter. `Copy` (stored values only).
#[derive(Clone, Copy)]
pub(super) struct ResponseOrder {
    issued: StoredValue<u64>,
    applied: StoredValue<u64>,
}

impl Default for ResponseOrder {
    fn default() -> Self {
        Self {
            issued: StoredValue::new(0),
            applied: StoredValue::new(0),
        }
    }
}

impl ResponseOrder {
    /// Number a request as it goes out.
    pub(super) fn begin(self) -> u64 {
        self.issued.update_value(|n| *n += 1);
        self.issued.get_value()
    }

    /// May the response to request `seq` be applied? Records it when it may.
    pub(super) fn accept(self, seq: u64) -> bool {
        let newer = is_newer(seq, self.applied.get_value());
        if newer {
            self.applied.set_value(seq);
        }
        newer
    }
}

/// A response is applied only when no later-issued request was applied first.
fn is_newer(seq: u64, applied: u64) -> bool {
    seq > applied
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn only_a_response_newer_than_the_last_applied_one_is_applied() {
        assert!(is_newer(1, 0));
        assert!(is_newer(5, 3));
        // The poll sent before the save (seq 3) lands after the save's reload (seq 4).
        assert!(!is_newer(3, 4));
        // The same response twice.
        assert!(!is_newer(4, 4));
    }
}
