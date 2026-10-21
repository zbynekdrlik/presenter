//! The store's side of the plate saves (#871): it hands every commit,
//! answer, retry and Zobraziť to the ONE controller ([`SaveControl`],
//! pure) and carries out what it asks for: a keepalive PATCH with a 15 s
//! timeout, a retry timer, a toast, a show, a list reload, and the
//! [`PlateSave`] each row reads (its „neuložené" marker and the texts it
//! may stop holding as drafts).

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::Nameplate;

use super::save_control::{Commit, Patch, SaveControl, SaveOutcome, Settle, Step, TextLeft, Texts};
use super::{nameplate_path, NameplateStore, NameplateTextReq};
use crate::api::ApiError;

/// A plate PATCH that has not answered after this is aborted (a failure),
/// so a save that never answers never holds its plate (#871 review).
const SAVE_TIMEOUT_MS: u32 = 15_000;

/// What a row reads about its plate's saves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlateSave {
    /// A save failed and the newest texts are not saved yet: „neuložené".
    pub unsaved: bool,
    /// The newest settle, numbered so a row applies each one once.
    pub settled: Option<(u64, Settle)>,
}

impl NameplateStore {
    /// What row `id` reads about its plate's saves (tracked).
    pub fn save_state(self, id: i64) -> PlateSave {
        self.save_views
            .try_with(|views| views.get(&id).cloned())
            .flatten()
            .unwrap_or_default()
    }

    /// Whether plate `id` has a save on its way, waiting or failed, or a
    /// Zobraziť waiting.
    pub fn is_saving(self, id: i64) -> bool {
        self.saves
            .try_with_value(|control| control.is_busy(id))
            .unwrap_or(false)
    }

    /// The newest texts committed for plate `id`, while it is busy.
    pub fn desired(self, id: i64) -> Option<Texts> {
        self.saves
            .try_with_value(|control| control.desired(id))
            .flatten()
    }

    /// A row commits `texts` (trimmed, the name not empty); `saved` = the
    /// texts it shows as saved, `urgent` = the page is being hidden or
    /// closed, `reason` = why (for the log). The controller decides: send
    /// now, wait for the save on its way, or nothing.
    pub fn save(self, id: i64, texts: Texts, saved: Texts, urgent: bool, reason: &'static str) {
        let Some((commit, step)) = self
            .saves
            .try_update_value(|control| control.commit(id, texts, saved, urgent))
        else {
            return;
        };
        self.run(id, step);
        match commit {
            Commit::Sent => leptos::logging::log!("nameplates: plate {id} save sent"),
            Commit::SentBeside => leptos::logging::log!(
                "nameplates: plate {id} save sent now beside the one on its way ({reason})"
            ),
            Commit::Waits => {
                leptos::logging::log!("nameplates: plate {id} save waits for the one on its way")
            }
            Commit::OnItsWay => leptos::logging::log!(
                "nameplates: plate {id} save of these texts is on its way ({reason})"
            ),
            Commit::Unchanged => {
                leptos::logging::log!("nameplates: plate {id} nothing to save ({reason})")
            }
        }
    }

    /// Send every failed save again (`trigger`: the network or the page is
    /// back).
    pub fn retry_failed(self, trigger: &'static str) {
        let ids = self
            .saves
            .try_with_value(SaveControl::failed)
            .unwrap_or_default();
        for id in ids {
            self.retry_with(id, trigger, |control| control.retry(id));
        }
    }

    /// Run a retry of plate `id` the controller allows (`retry`), and log it.
    fn retry_with(
        self,
        id: i64,
        trigger: &'static str,
        retry: impl FnOnce(&mut SaveControl) -> Step,
    ) {
        let Some(step) = self.saves.try_update_value(retry) else {
            return;
        };
        let sent = step.patch.is_some();
        self.run(id, step);
        if sent {
            leptos::logging::log!("nameplates: plate {id} failed save sent again ({trigger})");
        }
    }

    /// Zobraziť on plate `id` (its row committed its drafts first): on air
    /// now when nothing of the plate is busy, else once its newest texts
    /// are saved (a failed save goes again now).
    pub fn show_saved(self, id: i64) {
        let Some((now, step)) = self.saves.try_update_value(|control| control.show(id)) else {
            return;
        };
        if now {
            self.show_person(id);
            return;
        }
        leptos::logging::log!("nameplates: plate {id} goes on air once its save is through");
        let sent = step.patch.is_some();
        self.run(id, step);
        if sent {
            leptos::logging::log!(
                "nameplates: plate {id} failed save sent again (Zobraziť was pressed)"
            );
        }
    }

    /// Forget plate `id`'s saves: its row left a list that no longer has
    /// it (deleted elsewhere). A later answer is dropped silently.
    pub fn drop_saves(self, id: i64) {
        self.saves.update_value(|control| control.drop_plate(id));
        self.update_view(id, None);
    }

    /// Carry out a controller step for plate `id`.
    fn run(self, id: i64, step: Step) {
        let Step {
            patch,
            retry_in,
            settled,
            show,
            idle,
            ..
        } = step;
        if let Some(patch) = patch {
            self.send_patch(id, patch);
        }
        if let Some((failure, delay)) = retry_in {
            leptos::logging::log!("nameplates: plate {id} failed save retried in {delay} ms");
            leptos::task::spawn_local(async move {
                TimeoutFuture::new(delay).await;
                self.retry_with(id, "the retry pause ran out", |control| {
                    control.retry_due(id, failure)
                });
            });
        }
        self.update_view(id, settled);
        if show {
            leptos::logging::log!("nameplates: plate {id} saved; shown now");
            self.show_person(id);
        }
        if idle {
            self.reload_list();
        }
    }

    /// Bring row `id`'s [`PlateSave`] up to date (notifies only on a change).
    fn update_view(self, id: i64, settled: Option<(u64, Settle)>) {
        let unsaved = self
            .saves
            .try_with_value(|control| control.is_unsaved(id))
            .unwrap_or(false);
        self.save_views.maybe_update(|views| {
            let view = views.entry(id).or_default();
            let before = view.clone();
            view.unsaved = unsaved;
            if settled.is_some() {
                view.settled = settled;
            }
            *view != before
        });
    }

    /// Start a plate PATCH: a keepalive fetch, handed to the browser before
    /// this returns (a page closed right after still sends it), aborted
    /// after [`SAVE_TIMEOUT_MS`].
    fn send_patch(self, id: i64, Patch { token, texts }: Patch) {
        let (primary_text, secondary_text) = texts;
        let req = NameplateTextReq {
            primary_text,
            secondary_text,
        };
        let fetch = crate::api::start_patch_keepalive(&nameplate_path(id), &req, SAVE_TIMEOUT_MS);
        leptos::task::spawn_local(async move {
            let result = crate::api::finish_json::<Nameplate>(fetch).await;
            self.finish_save(id, token, result);
        });
    }

    /// The PATCH `token` of plate `id` answered. A saved plate goes into
    /// the local list at once (before the refetch); a save that did not go
    /// through toasts why, with where its typed text is now.
    fn finish_save(self, id: i64, token: u64, result: Result<Nameplate, ApiError>) {
        let error = match result {
            Ok(saved) => {
                self.next_gen();
                self.plates.update(|list| {
                    if let Some(plate) = list.iter_mut().find(|p| p.id == saved.id) {
                        *plate = saved;
                    }
                });
                None
            }
            Err(e) => Some(e),
        };
        let outcome = error
            .as_ref()
            .map_or(SaveOutcome::Saved, SaveOutcome::of_error);
        // A row holds the typed text while the plate is in the list it shows.
        let held = self
            .plates
            .try_with_untracked(|list| list.iter().any(|p| p.id == id))
            .unwrap_or(false);
        let Some(step) = self
            .saves
            .try_update_value(|control| control.finish(id, token, outcome, held))
            .flatten()
        else {
            leptos::logging::log!(
                "nameplates: plate {id} answer of a forgotten save ({outcome:?})"
            );
            return;
        };
        if let Some(e) = &error {
            leptos::logging::log!("nameplates: plate {id} save not done ({outcome:?}): {e}");
            if outcome.is_reported(held, step.idle) {
                let left = TextLeft::of(held, step.not_shown);
                self.notify(outcome.message(e, left), "error");
            }
        }
        let all_saved = outcome == SaveOutcome::Saved && step.idle;
        self.run(id, step);
        if all_saved {
            leptos::logging::log!("nameplates: plate {id} saved; nothing left to send");
        }
    }
}
