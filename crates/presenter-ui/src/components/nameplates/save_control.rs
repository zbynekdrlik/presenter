//! The plate saves of one list, as ONE controller (#871), pure and
//! host-tested. `NameplateStore` (`mod.rs`) runs what it asks for (a PATCH,
//! a retry timer, a show, a list reload) and the rows read what it settles.
//!
//! Per plate it holds the newest texts the operator committed (`desired`),
//! the PATCH on its way (each send has its own token), the texts the server
//! last confirmed, the failure streak and a waiting Zobraziť. A plate with
//! none of that left is idle and has no entry at all.
//!
//! - **One PATCH per plate on its way.** A commit meanwhile only replaces
//!   the desired texts. Once the PATCH is back, the desired texts go if the
//!   server does not have them yet. So the server never applies an older
//!   text after a newer one, and an answer is matched to its save by its
//!   token, never by its texts.
//! - **A page being hidden or closed** may never run again, so an urgent
//!   commit sends the desired texts NOW, beside the PATCH on its way, when
//!   that one carries other texts. The server may apply those two out of
//!   order: accepted for this one case. The first rule repairs what the page
//!   can see: an older answer that comes last leaves the desired texts
//!   unconfirmed, so they go once more.
//! - **A failed save** keeps the desired texts and waits for a retry: a timer
//!   (2 s / 5 s / 15 s for the 1st / 2nd / 3rd failure in a row), the network
//!   or the page back, Zobraziť, any commit. Each timer is armed for ONE
//!   failure and does nothing once a save, a commit or another failure came
//!   since. A commit made while the failed save was on its way is sent at
//!   once. A retry sends the desired texts, never half-typed text (that is a
//!   row's draft, not committed yet). A failure whose text no field holds any
//!   more (its row left the list) gives up.
//! - **A failed save may have been applied** (a lost answer, the 15 s abort
//!   after the server wrote it, an unreadable 2xx body). Unless it carried
//!   the confirmed texts, the confirmed texts are unknown after it, so the
//!   desired texts go again once nothing else is on its way (the PATCH is
//!   idempotent). A failure answered while a PATCH of the plate is still on
//!   its way, or starts now, is not reported: that PATCH's answer decides.
//! - **Zobraziť on a busy plate** waits for a ticket: it goes on air once the
//!   plate is idle with its desired texts confirmed, unless the page showed
//!   or hid a plate, or switched output, since. A failed or deleted end
//!   drops it.

use std::collections::HashMap;

use crate::api::ApiError;

/// The texts of a plate save: (name, role), trimmed.
pub type Texts = (String, String);

/// The toast for typed text whose plate was deleted elsewhere: from the row
/// that leaves the list, and from a save the server answers with 404.
pub const DELETED_UNSAVED: &str = "Menovka bola medzičasom zmazaná — zmena sa neuložila.";

/// The toast for a failed save whose text no field holds any more: its row
/// left the list (an editor output switch).
pub const LOST_UNSAVED: &str = "Zmena menovky sa neuložila — skúste ju zadať znova.";

/// The prefix of a failure toast when a waiting Zobraziť was dropped.
const NOT_SHOWN: &str = "Menovka sa nezobrazila. ";

/// How a plate save ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The server has the texts.
    Saved,
    /// Refused for good: a 4xx for the texts.
    Refused,
    /// The plate is gone (404: deleted elsewhere).
    Gone,
    /// Not saved this time: a network error (a phone's Wi-Fi asleep, the
    /// 15 s timeout), a server fault (5xx, 408, 429) or an expired login
    /// (401, 403: Cloudflare Access). The desired texts are kept and sent
    /// again.
    Failed,
}

/// Where the typed text of a failed save is now, for its toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextLeft {
    /// Still in its field, marked „neuložené", sent again later.
    InField,
    /// Still in its field, and the Zobraziť that waited for the save did not
    /// put the plate on air.
    InFieldNotShown,
    /// In no field: its row left the list.
    Nowhere,
}

impl TextLeft {
    /// From whether a row still holds the text and whether a waiting
    /// Zobraziť was dropped.
    pub fn of(held: bool, not_shown: bool) -> Self {
        match (held, not_shown) {
            (false, _) => Self::Nowhere,
            (true, true) => Self::InFieldNotShown,
            (true, false) => Self::InField,
        }
    }
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
            // An aborted fetch (the 15 s timeout) is a network error.
            ApiError::Network(_) | ApiError::Deserialize(_) => Self::Failed,
            ApiError::Serialize(_) => Self::Refused,
        }
    }

    /// Whether an answer with this outcome is toasted. `held` = a row holds
    /// the plate's text, `on_its_way` = a PATCH of the plate is on its way
    /// after the answer ([`Step::on_its_way`]). A failure answered while one
    /// is has nothing to report yet: that PATCH's answer decides (a save
    /// sent beside it as the page hid, or the desired texts sent again), and
    /// reports a failure of its own.
    pub fn is_reported(self, held: bool, on_its_way: bool) -> bool {
        !(self == Self::Failed && held && on_its_way)
    }

    /// The toast for a save that came back with `error` (`self` is
    /// [`Self::of_error`] of it); `left` = where its typed text is now,
    /// `timer` = a retry timer is armed (from the 4th failure in a row none
    /// is, so the toast does not promise one).
    pub fn message(self, error: &ApiError, left: TextLeft, timer: bool) -> String {
        match (self, left) {
            (Self::Gone, TextLeft::InFieldNotShown) => format!("{NOT_SHOWN}{DELETED_UNSAVED}"),
            (Self::Gone, _) => DELETED_UNSAVED.to_string(),
            (Self::Failed, TextLeft::Nowhere) => format!("{LOST_UNSAVED} ({error})"),
            (Self::Failed, _) => {
                let not_shown = if left == TextLeft::InFieldNotShown {
                    NOT_SHOWN
                } else {
                    ""
                };
                let login = if is_login_error(error) {
                    " Prihlásenie mohlo vypršať — prihláste sa znova v novej karte."
                } else {
                    ""
                };
                let soon = if timer { "sám o chvíľu, " } else { "" };
                format!(
                    "{not_shown}Uloženie zlyhalo: {error}.{login} Text ostal v poli ako \
                     neuložený — uloží sa znova {soon}po obnovení siete alebo návrate na \
                     stránku, po stlačení Zobraziť alebo keď pole opustíte."
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

/// The pause before the automatic retry after the `failures`-th failure in
/// a row: 2 s, 5 s, 15 s, then none. A phone that unlocks fires
/// `visibilitychange` before its Wi-Fi is back, so one retry then is not
/// enough; after three, the next try waits for the network or the page
/// coming back, Zobraziť or a commit.
pub fn retry_delay_ms(failures: u32) -> Option<u32> {
    match failures {
        1 => Some(2_000),
        2 => Some(5_000),
        3 => Some(15_000),
        _ => None,
    }
}

/// A PATCH to start: its token and the texts it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub token: u64,
    pub texts: Texts,
}

/// Texts the rows may stop holding as drafts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settle {
    /// Each field (`Some`) whose text is final: the server confirmed the
    /// desired text and no save on its way carries another. A field whose
    /// draft is this text drops the draft.
    Fields {
        name: Option<String>,
        role: Option<String>,
    },
    /// The plate is gone (deleted elsewhere): every draft goes.
    Gone,
}

/// What a commit did, for the store's log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    /// Its PATCH starts now.
    Sent,
    /// Sent now beside the PATCH on its way (an urgent commit).
    SentBeside,
    /// Waits for the PATCH on its way, then goes.
    Waits,
    /// A PATCH on its way already carries these texts.
    OnItsWay,
    /// The server has these texts: nothing to send.
    Unchanged,
}

/// What the store does after a controller call. `settled` carries a number
/// that changes with every settle, so a row applies each one once.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    /// Start this PATCH.
    pub patch: Option<Patch>,
    /// Arm a retry timer: (the failure it is for, delay in ms).
    pub retry_in: Option<(u64, u32)>,
    pub settled: Option<(u64, Settle)>,
    /// Put the plate on air now (a waiting Zobraziť, still current).
    pub show: bool,
    /// A waiting Zobraziť was dropped (the save failed, or the plate is gone).
    pub not_shown: bool,
    /// An answer left nothing of the plate: the list may be reloaded.
    pub idle: bool,
    /// Set by an answer: a PATCH of the plate is on its way after it (still
    /// out, or started now). Its answer decides, so a failure answered now
    /// is not reported ([`SaveOutcome::is_reported`]).
    pub on_its_way: bool,
}

/// A failure streak: failures in a row, and the number of the newest one
/// (the one a retry timer may act on).
struct Failure {
    count: u32,
    epoch: u64,
}

/// One busy plate.
struct Slot {
    desired: Texts,
    /// The PATCHes on their way: one, or more after urgent commits.
    in_flight: Vec<Patch>,
    /// The texts the server has as far as the page knows: the newest save
    /// answered as saved, else the saved texts when the plate got busy.
    /// `None` after a failed save of other texts: that one may have been
    /// applied, so what the server holds is unknown.
    confirmed: Option<Texts>,
    failure: Option<Failure>,
    /// The ticket of a waiting Zobraziť.
    show: Option<u64>,
}

/// The saves of every plate of one list.
#[derive(Default)]
pub struct SaveControl {
    slots: HashMap<i64, Slot>,
    /// Tokens, failure numbers and settle numbers.
    last: u64,
    /// Bumped by every show or hide this page asks for.
    show_gen: u64,
}

impl SaveControl {
    fn next(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// Whether plate `id` has a save on its way, waiting, failed or a
    /// Zobraziť waiting.
    pub fn is_busy(&self, id: i64) -> bool {
        self.slots.contains_key(&id)
    }

    /// The newest committed texts of a busy plate.
    pub fn desired(&self, id: i64) -> Option<Texts> {
        self.slots.get(&id).map(|slot| slot.desired.clone())
    }

    /// Whether a save of plate `id` failed and its texts are not saved yet.
    pub fn is_unsaved(&self, id: i64) -> bool {
        self.slots
            .get(&id)
            .is_some_and(|slot| slot.failure.is_some())
    }

    /// The plates whose failed save waits for a retry, in id order.
    pub fn failed(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .slots
            .iter()
            .filter(|(_, slot)| slot.failure.is_some() && slot.in_flight.is_empty())
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// The page is being hidden or closed: every plate whose desired texts
    /// wait behind a PATCH on its way, and that no row on the page will
    /// commit urgently (`has_row` false: its row left the list, an editor
    /// output switch), sends them now beside it, as an urgent commit does.
    /// One step per plate sent, in id order.
    pub fn send_waiting(&mut self, has_row: impl Fn(i64) -> bool) -> Vec<(i64, Step)> {
        let mut ids: Vec<i64> = self
            .slots
            .iter()
            .filter(|(id, slot)| {
                !has_row(**id)
                    && !slot.in_flight.is_empty()
                    && slot
                        .in_flight
                        .iter()
                        .all(|patch| patch.texts != slot.desired)
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(|id| (id, Step::with_patch(self.send(id))))
            .collect()
    }

    /// The operator committed `texts` for plate `id`. `saved` = the texts
    /// the page last had from the server (what an idle plate compares
    /// with); `urgent` = the page is being hidden or closed.
    pub fn commit(&mut self, id: i64, texts: Texts, saved: Texts, urgent: bool) -> (Commit, Step) {
        let Some(slot) = self.slots.get_mut(&id) else {
            self.slots.insert(id, Slot::new(texts, saved));
            return Self::sent_or_unchanged(self.advance(id));
        };
        let on_its_way = slot.in_flight.iter().any(|patch| patch.texts == texts);
        slot.desired = texts;
        if slot.in_flight.is_empty() {
            return Self::sent_or_unchanged(self.advance(id));
        }
        if on_its_way {
            return (Commit::OnItsWay, Step::default());
        }
        if urgent {
            let patch = self.send(id);
            return (Commit::SentBeside, Step::with_patch(patch));
        }
        (Commit::Waits, Step::default())
    }

    /// A commit that found nothing on its way: sent, or already saved (no
    /// answer came, so there is nothing new to reload).
    fn sent_or_unchanged(mut step: Step) -> (Commit, Step) {
        if step.patch.is_some() {
            return (Commit::Sent, step);
        }
        step.idle = false;
        (Commit::Unchanged, step)
    }

    /// The PATCH `token` of plate `id` came back with `outcome`; `held` =
    /// a row still holds the plate's typed text. `None` = an answer for a
    /// save the controller has forgotten (its plate was deleted, or its
    /// failure gave up): nothing to report.
    pub fn finish(
        &mut self,
        id: i64,
        token: u64,
        outcome: SaveOutcome,
        held: bool,
    ) -> Option<Step> {
        let slot = self.slots.get_mut(&id)?;
        let at = slot.in_flight.iter().position(|p| p.token == token)?;
        let sent = slot.in_flight.remove(at);
        match outcome {
            SaveOutcome::Gone => return Some(self.forget(id, true)),
            SaveOutcome::Failed if !held => return Some(self.forget(id, false)),
            SaveOutcome::Refused if sent.texts == slot.desired => {
                return Some(self.refused(id, sent.texts));
            }
            SaveOutcome::Saved => {
                slot.confirmed = Some(sent.texts.clone());
                slot.failure = None;
            }
            SaveOutcome::Refused => slot.failure = None,
            SaveOutcome::Failed => {
                // It may have been applied all the same: unless it carried
                // the confirmed texts, what the server holds is unknown.
                if slot.confirmed.as_ref() != Some(&sent.texts) {
                    slot.confirmed = None;
                }
                let count = slot.failure.as_ref().map_or(0, |f| f.count) + 1;
                let epoch = self.next();
                if let Some(slot) = self.slots.get_mut(&id) {
                    slot.failure = Some(Failure { count, epoch });
                }
            }
        }
        let slot = self.slots.get_mut(&id)?;
        if !slot.in_flight.is_empty() {
            let mut step = Step {
                on_its_way: true,
                ..Step::default()
            };
            if outcome == SaveOutcome::Saved {
                step.settled = self.settled_fields(id);
            }
            return Some(step);
        }
        if outcome == SaveOutcome::Failed && slot.desired == sent.texts {
            return Some(Self::wait_for_retry(slot));
        }
        let mut step = self.advance(id);
        step.on_its_way = step.patch.is_some();
        if outcome == SaveOutcome::Saved && step.settled.is_none() {
            step.settled = self.settled_fields(id);
        }
        Some(step)
    }

    /// Send the failed save of plate `id` again (the network or the page is
    /// back, Zobraziť, its row left the list). Nothing when it did not fail
    /// or is on its way.
    pub fn retry(&mut self, id: i64) -> Step {
        let waiting = self
            .slots
            .get(&id)
            .is_some_and(|slot| slot.failure.is_some() && slot.in_flight.is_empty());
        if waiting {
            self.advance(id)
        } else {
            Step::default()
        }
    }

    /// The retry timer armed for failure `epoch` of plate `id` ran out: a
    /// retry, unless a save, a commit or another failure came since.
    pub fn retry_due(&mut self, id: i64, epoch: u64) -> Step {
        let current = self
            .slots
            .get(&id)
            .and_then(|slot| slot.failure.as_ref())
            .is_some_and(|failure| failure.epoch == epoch);
        if current {
            self.retry(id)
        } else {
            Step::default()
        }
    }

    /// Zobraziť on plate `id`: `true` = show it now (idle). A busy plate
    /// waits with a ticket (and a failed save goes again now).
    pub fn show(&mut self, id: i64) -> (bool, Step) {
        if !self.slots.contains_key(&id) {
            return (true, Step::default());
        }
        let ticket = self.note_show();
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.show = Some(ticket);
        }
        (false, self.retry(id))
    }

    /// Forget plate `id`: its row left a list that no longer has it
    /// (deleted elsewhere). A later answer of its saves is dropped.
    pub fn drop_plate(&mut self, id: i64) {
        self.slots.remove(&id);
    }

    /// The page shows or hides a plate: a waiting Zobraziť is stale now.
    pub fn note_show(&mut self) -> u64 {
        self.show_gen += 1;
        self.show_gen
    }

    /// Nothing of plate `id` is on its way: send its desired texts, or, when
    /// the server is known to have them, the plate is idle (its entry goes,
    /// its fields are settled, a current waiting Zobraziť goes on air). A
    /// plate with no entry has nothing to do.
    fn advance(&mut self, id: i64) -> Step {
        let Some(slot) = self.slots.get(&id) else {
            return Step::default();
        };
        if slot.confirmed.as_ref() != Some(&slot.desired) {
            return Step::with_patch(self.send(id));
        }
        let Some(slot) = self.slots.remove(&id) else {
            return Step::default();
        };
        Step {
            settled: Some((self.next(), Settle::both(slot.desired))),
            show: slot.show == Some(self.show_gen),
            idle: true,
            ..Step::default()
        }
    }

    /// Start a PATCH of plate `id`'s desired texts; `None` for a plate with
    /// no entry (never a PATCH with empty texts).
    fn send(&mut self, id: i64) -> Option<Patch> {
        if !self.slots.contains_key(&id) {
            return None;
        }
        let token = self.next();
        let slot = self.slots.get_mut(&id)?;
        let patch = Patch {
            token,
            texts: slot.desired.clone(),
        };
        slot.in_flight.push(patch.clone());
        Some(patch)
    }

    /// The failed save waits for a retry: a timer when the streak allows
    /// one; a waiting Zobraziť is dropped.
    fn wait_for_retry(slot: &mut Slot) -> Step {
        let retry_in = slot
            .failure
            .as_ref()
            .and_then(|f| retry_delay_ms(f.count).map(|delay| (f.epoch, delay)));
        Step {
            retry_in,
            not_shown: slot.show.take().is_some(),
            ..Step::default()
        }
    }

    /// Drop plate `id`: deleted elsewhere (`gone`: every draft goes), or a
    /// failure whose text no field holds.
    fn forget(&mut self, id: i64, gone: bool) -> Step {
        let not_shown = self
            .slots
            .remove(&id)
            .is_some_and(|slot| slot.show.is_some());
        Step {
            settled: gone.then(|| (self.next(), Settle::Gone)),
            not_shown,
            idle: true,
            ..Step::default()
        }
    }

    /// The desired texts were refused for good: the plate keeps the texts
    /// the server has, and the fields show them again. A waiting Zobraziť
    /// goes on air with them.
    fn refused(&mut self, id: i64, texts: Texts) -> Step {
        let show = self
            .slots
            .remove(&id)
            .is_some_and(|slot| slot.show == Some(self.show_gen));
        Step {
            settled: Some((self.next(), Settle::both(texts))),
            show,
            idle: true,
            ..Step::default()
        }
    }

    /// The fields of busy plate `id` whose text is final: confirmed as the
    /// desired text, and no save on its way carries another. None while the
    /// server's texts are unknown.
    fn settled_fields(&mut self, id: i64) -> Option<(u64, Settle)> {
        let slot = self.slots.get(&id)?;
        let confirmed = slot.confirmed.as_ref()?;
        let name = (confirmed.0 == slot.desired.0
            && slot.in_flight.iter().all(|p| p.texts.0 == confirmed.0))
        .then(|| confirmed.0.clone());
        let role = (confirmed.1 == slot.desired.1
            && slot.in_flight.iter().all(|p| p.texts.1 == confirmed.1))
        .then(|| confirmed.1.clone());
        if name.is_none() && role.is_none() {
            return None;
        }
        Some((self.next(), Settle::Fields { name, role }))
    }
}

impl Settle {
    /// Both texts are final.
    fn both((name, role): Texts) -> Self {
        Self::Fields {
            name: Some(name),
            role: Some(role),
        }
    }
}

impl Slot {
    fn new(desired: Texts, saved: Texts) -> Self {
        Self {
            desired,
            in_flight: Vec::new(),
            confirmed: Some(saved),
            failure: None,
            show: None,
        }
    }
}

impl Step {
    fn with_patch(patch: Option<Patch>) -> Self {
        Self {
            patch,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests;
