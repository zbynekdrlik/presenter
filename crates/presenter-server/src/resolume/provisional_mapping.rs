//! #808 regression (2026-10-05): a cached mapping is only as good as the
//! moment it was fetched. Two ways it went wrong with no 404 to notice it:
//!
//! - **Arena was still loading.** After an Arena restart on PP, the recovery
//!   fetch got 27 clips with no recognized tag. The driver cached that, every
//!   lyric push was skipped (`no clips configured for lane`), and nothing
//!   refetched until the operator pressed "Refresh mapping".
//! - **The operator switched decks.** `/composition` lists only the selected
//!   deck's clips, so after a switch every cached id points at a deck that is
//!   no longer on the wall.
//!
//! What the host worker does about it, without ever going back to a timer that
//! reads the composition (#808):
//!
//! - Every fetch remembers the selected deck (`decks[].selected.value`) and is
//!   compared with that deck's last good destination kinds. A mapping with no
//!   recognized destination, or one lacking a kind the deck's last good
//!   mapping had, is *suspect*: up to five follow-up fetches run
//!   [`FOLLOW_UP_DELAYS`] apart and stop the moment it is complete again.
//! - Before every stage/Bible push, and on the 10 s tick after the `/product`
//!   probe, a deck check `GET /composition/decks/by-id/{id}` (~360 B).
//!   `selected:false` or 404 drops the mapping: the push refetches it inline
//!   (never rate-limited) and lands on the new deck. A failed check never
//!   blocks the push; it goes out on the cached mapping.
//! - A push that needs a lane the cached mapping lacks, while the deck's last
//!   good mapping had it (or the mapping has no destination at all),
//!   refetches once per deck before it is applied. A lane still empty after
//!   that is what the deck really holds: one WARN, and nothing more until the
//!   deck changes, a complete fetch, or an operator refresh.
//! - A host or deck that never had a kind (Bridge PP has no `#main`, SNV no
//!   `#translate`) never refetches for it.

use super::clip_map::{BIBLE_CLEAR_KIND, BIBLE_LANE_KINDS, MAIN_KINDS, TRANSLATION_KINDS};
use super::driver::{duration_ms, FetchReason, HostDriver, ACTION_TIMEOUT};
use super::mapping_refresh::Push;
use super::ResolumeConnectionSnapshot;
use anyhow::{anyhow, Context};
use reqwest::StatusCode;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::RwLock, time::Instant};
use tracing::{debug, info, warn};

type Status = Arc<RwLock<ResolumeConnectionSnapshot>>;

/// Delays between the follow-up fetches of a suspect mapping: five fetches
/// at most, about 112 s in total. Arena loading a big composition is exactly
/// this window. After the last one the push path takes over.
pub(super) const FOLLOW_UP_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];
/// Minimum spacing of the "deck check failed" WARN for one host.
const DECK_CHECK_WARN_INTERVAL: Duration = Duration::from_secs(300);

/// The selected deck of a composition; `None` when it lists no decks.
pub(super) type DeckKey = Option<i64>;

/// The next follow-up fetch of a suspect mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FollowUp {
    /// Index into [`FOLLOW_UP_DELAYS`] of the step that runs at `due`.
    pub(super) step: usize,
    pub(super) due: Instant,
}

/// What a fetch changed about the follow-up schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScheduleChange {
    Started,
    Advanced,
    /// A lane refetch: it neither starts nor advances the schedule.
    Unchanged,
    /// The last step ran and the mapping is still suspect.
    Exhausted,
}

/// How one fetched mapping compares with its deck's last good mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FetchVerdict {
    /// Every kind the deck's last good mapping had is there.
    Complete { follow_ups_stopped: bool },
    /// An operator refresh: its kinds are the deck's reference from now on.
    Adopted,
    /// Arena may still be loading. `lacking` lists the kinds the deck's last
    /// good mapping had; `no_destinations` means no recognized clip at all.
    Suspect {
        lacking: Vec<&'static str>,
        no_destinations: bool,
        schedule: ScheduleChange,
    },
}

/// Per-host, in-memory state of how far the cached mapping can be trusted.
#[derive(Debug, Default)]
pub(super) struct ProvisionalMapping {
    /// The selected deck of the cached mapping.
    pub(super) selected_deck: DeckKey,
    /// Destination kinds of each deck's last complete mapping.
    pub(super) last_good: HashMap<DeckKey, Vec<&'static str>>,
    /// The next follow-up fetch while the cached mapping is suspect.
    pub(super) follow_up: Option<FollowUp>,
    /// The current deck's one push-triggered lane refetch is used up.
    pub(super) lane_refetch_spent: bool,
    /// When the "deck check failed" WARN was last logged.
    last_deck_check_warn: Option<Instant>,
}

impl ProvisionalMapping {
    /// Record a fetched mapping of `deck` with these destination `kinds`.
    pub(super) fn note_fetch(
        &mut self,
        deck: DeckKey,
        kinds: Vec<&'static str>,
        reason: FetchReason,
        now: Instant,
    ) -> FetchVerdict {
        if deck != self.selected_deck {
            self.lane_refetch_spent = false;
        }
        self.selected_deck = deck;
        if matches!(reason, FetchReason::Manual) && !kinds.is_empty() {
            // Refresh mapping is how an intentional clip edit reaches
            // Presenter, so the operator's result becomes the reference.
            self.last_good.insert(deck, kinds);
            self.follow_up = None;
            self.lane_refetch_spent = false;
            return FetchVerdict::Adopted;
        }
        let lacking: Vec<&'static str> = self
            .last_good
            .get(&deck)
            .map(|good| {
                good.iter()
                    .copied()
                    .filter(|kind| !kinds.contains(kind))
                    .collect()
            })
            .unwrap_or_default();
        let no_destinations = kinds.is_empty();
        if !no_destinations && lacking.is_empty() {
            self.last_good.insert(deck, kinds);
            self.lane_refetch_spent = false;
            let follow_ups_stopped = self.follow_up.take().is_some();
            return FetchVerdict::Complete { follow_ups_stopped };
        }
        let schedule = match reason {
            FetchReason::FollowUp => self.advance_follow_up(now),
            FetchReason::LaneMissing => ScheduleChange::Unchanged,
            FetchReason::Missing
            | FetchReason::ErrorInvalidated
            | FetchReason::StaleId
            | FetchReason::Manual
            | FetchReason::DeckChanged => {
                self.follow_up = Some(FollowUp {
                    step: 0,
                    due: now + FOLLOW_UP_DELAYS[0],
                });
                ScheduleChange::Started
            }
        };
        FetchVerdict::Suspect {
            lacking,
            no_destinations,
            schedule,
        }
    }

    /// Move past the current follow-up step: schedule the next one, or end
    /// the schedule after the last.
    pub(super) fn advance_follow_up(&mut self, now: Instant) -> ScheduleChange {
        let next_step = self
            .follow_up
            .map_or(FOLLOW_UP_DELAYS.len(), |follow_up| follow_up.step + 1);
        match FOLLOW_UP_DELAYS.get(next_step) {
            Some(delay) => {
                self.follow_up = Some(FollowUp {
                    step: next_step,
                    due: now + *delay,
                });
                ScheduleChange::Advanced
            }
            None => {
                self.follow_up = None;
                ScheduleChange::Exhausted
            }
        }
    }

    /// Whether a push that needs `kind`, which the cached mapping lacks,
    /// should refetch: the deck's last good mapping had it, or the cached
    /// mapping has no recognized destination at all (Arena loading).
    fn lane_expected(&self, kind: &str, mapping_has_destinations: bool) -> bool {
        !mapping_has_destinations
            || self
                .last_good
                .get(&self.selected_deck)
                .is_some_and(|good| good.iter().any(|had| *had == kind))
    }

    fn should_warn_deck_check(&mut self, now: Instant) -> bool {
        let due = match self.last_deck_check_warn {
            Some(last) => now.duration_since(last) >= DECK_CHECK_WARN_INTERVAL,
            None => true,
        };
        if due {
            self.last_deck_check_warn = Some(now);
        }
        due
    }
}

/// The id of the deck a `/composition` body marks selected.
pub(super) fn selected_deck_id(composition: &Value) -> DeckKey {
    composition
        .get("decks")?
        .as_array()?
        .iter()
        .find(|deck| deck_selected(deck) == Some(true))?
        .get("id")?
        .as_i64()
}

/// A deck's `selected.value` flag (`/composition` and `/decks/by-id/{id}`).
fn deck_selected(deck: &Value) -> Option<bool> {
    deck.get("selected")?.get("value")?.as_bool()
}

/// The destination kinds a push writes. Timer frames are not listed: they
/// tick every second and never refetch.
pub(super) fn push_required_kinds(push: &Push) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    match push {
        Push::Stage(update) => {
            if update.current_main.is_some() {
                kinds.extend(MAIN_KINDS);
            }
            if update.current_translation.is_some() {
                kinds.extend(TRANSLATION_KINDS);
            }
        }
        Push::Bible(update) => {
            kinds.extend(BIBLE_LANE_KINDS);
            if update.slide_output.is_none() && update.passage.is_none() {
                kinds.push(BIBLE_CLEAR_KIND);
            }
        }
        Push::Timer(_) => {}
    }
    kinds
}

/// The worker's follow-up branch: sleeps until `due`, forever when there is
/// no follow-up scheduled.
pub(super) async fn follow_up_deadline(due: Option<Instant>) {
    match due {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// What the deck check says about the cached mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeckCheck {
    /// The composition lists no decks, or the check failed: keep the mapping.
    Keep,
    /// The cached deck is still selected.
    Selected,
    /// The cached deck is no longer selected, or gone: refetch.
    Switched,
}

/// One deck's answer to `GET /composition/decks/by-id/{id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeckState {
    Selected,
    NotSelected,
    /// 404: the deck was deleted or another composition was loaded.
    Gone,
}

impl HostDriver {
    /// The next follow-up fetch's deadline, for the worker's `select!`.
    pub(super) fn follow_up_due(&self) -> Option<Instant> {
        self.provisional.follow_up.map(|follow_up| follow_up.due)
    }

    /// Called after every successful composition fetch: compare it with the
    /// deck's last good mapping and log what changed.
    pub(super) fn note_fetched_mapping(
        &mut self,
        deck: DeckKey,
        kinds: Vec<&'static str>,
        reason: FetchReason,
    ) {
        let verdict = self
            .provisional
            .note_fetch(deck, kinds, reason, Instant::now());
        let host = &self.config.host;
        match verdict {
            FetchVerdict::Complete {
                follow_ups_stopped: true,
            } => info!(
                %host,
                deck = ?deck,
                reason = reason.as_str(),
                "resolume mapping is complete again; follow-up fetches stopped"
            ),
            FetchVerdict::Complete {
                follow_ups_stopped: false,
            } => {}
            FetchVerdict::Adopted => debug!(
                %host,
                deck = ?deck,
                "resolume operator refresh: its clips are this deck's reference from now on"
            ),
            FetchVerdict::Suspect {
                lacking,
                no_destinations,
                schedule,
            } => self.log_suspect_mapping(deck, reason, &lacking, no_destinations, schedule),
        }
    }

    fn log_suspect_mapping(
        &self,
        deck: DeckKey,
        reason: FetchReason,
        lacking: &[&'static str],
        no_destinations: bool,
        schedule: ScheduleChange,
    ) {
        let host = &self.config.host;
        match (schedule, self.provisional.follow_up) {
            (ScheduleChange::Started | ScheduleChange::Advanced, Some(next)) => info!(
                %host,
                deck = ?deck,
                reason = reason.as_str(),
                no_destinations,
                lacking = ?lacking,
                next_step = next.step + 1,
                of = FOLLOW_UP_DELAYS.len(),
                next_in_secs = next.due.saturating_duration_since(Instant::now()).as_secs(),
                "resolume mapping looks incomplete (Arena may still be loading the composition); follow-up fetch scheduled"
            ),
            (ScheduleChange::Exhausted, _) => warn!(
                %host,
                deck = ?deck,
                no_destinations,
                lacking = ?lacking,
                "resolume mapping still incomplete after every follow-up fetch; follow-ups stopped. A push that needs a missing lane refetches once; Settings > Refresh mapping re-reads it now"
            ),
            _ => debug!(
                %host,
                deck = ?deck,
                reason = reason.as_str(),
                no_destinations,
                lacking = ?lacking,
                "resolume mapping still incomplete"
            ),
        }
    }

    /// One step of the follow-up schedule, when its deadline arrives.
    pub(super) async fn run_follow_up(&mut self, status: &Status) {
        let Some(follow_up) = self.provisional.follow_up else {
            return;
        };
        if !self.config.is_enabled {
            self.provisional.follow_up = None;
            return;
        }
        if self.in_backoff() {
            // A down host: try this step again when its backoff window ends.
            let retry_at = self
                .next_retry_at
                .unwrap_or_else(|| Instant::now() + FOLLOW_UP_DELAYS[0]);
            self.provisional.follow_up = Some(FollowUp {
                due: retry_at,
                ..follow_up
            });
            return;
        }
        info!(
            host = %self.config.host,
            step = follow_up.step + 1,
            of = FOLLOW_UP_DELAYS.len(),
            "resolume follow-up fetch of an incomplete mapping"
        );
        // Keeps the cached mapping until the new one is parsed;
        // `note_fetched_mapping` then advances or stops the schedule.
        match self
            .refresh_mapping_with_reason(FetchReason::FollowUp)
            .await
        {
            Ok(()) => self.mark_connected(status).await,
            Err(err) => {
                // A failed step still counts, so a host that keeps failing
                // ends the schedule.
                let change = self.provisional.advance_follow_up(Instant::now());
                debug!(host = %self.config.host, ?change, "resolume follow-up fetch failed");
                self.record_error(err, status).await;
            }
        }
    }

    /// Before a stage/Bible push (never a timer frame): make sure the cached
    /// mapping belongs to the selected deck, and refetch once for a lane it
    /// lacks but should have.
    pub(super) async fn prepare_mapping_for_push(&mut self, push: &Push) {
        if matches!(push, Push::Timer(_))
            || !self.config.is_enabled
            || self.in_backoff()
            || self.mapping.is_none()
        {
            return;
        }
        if self.check_selected_deck().await == DeckCheck::Switched {
            // The push's own `ensure_mapping` refetches inline, never
            // rate-limited, and the push lands on the new deck. If that fetch
            // fails, the push fails like any composition fetch: it never
            // writes to the ids of a deck that left the wall.
            self.invalidate_mapping(FetchReason::DeckChanged);
            return;
        }
        self.refetch_for_missing_lane(push).await;
    }

    /// The tick's deck check, after a successful `/product` probe: a switched
    /// deck is refetched now, so the next push needs no inline fetch.
    pub(super) async fn follow_deck_switch(&mut self) -> anyhow::Result<()> {
        if self.check_selected_deck().await != DeckCheck::Switched {
            return Ok(());
        }
        self.invalidate_mapping(FetchReason::DeckChanged);
        self.ensure_mapping().await.map(|_| ())
    }

    /// The kinds `push` needs that the cached mapping lacks but should have.
    fn lanes_to_refetch_for(&self, push: &Push) -> Vec<&'static str> {
        let Some(mapping) = &self.mapping else {
            return Vec::new();
        };
        let kinds = mapping.destination_kinds();
        let has_destinations = !kinds.is_empty();
        push_required_kinds(push)
            .into_iter()
            .filter(|kind| {
                !kinds.contains(kind) && self.provisional.lane_expected(kind, has_destinations)
            })
            .collect()
    }

    /// One refetch per deck for a push whose lane the cached mapping lacks.
    /// A failed refetch only logs: the push goes out on the cached mapping.
    async fn refetch_for_missing_lane(&mut self, push: &Push) {
        if self.provisional.lane_refetch_spent {
            return;
        }
        let lacking = self.lanes_to_refetch_for(push);
        if lacking.is_empty() {
            return;
        }
        self.provisional.lane_refetch_spent = true;
        info!(
            host = %self.config.host,
            deck = ?self.provisional.selected_deck,
            lacking = ?lacking,
            "resolume push needs clips the cached mapping lacks (Arena may have been loading); refetching the composition once for this deck"
        );
        if let Err(err) = self
            .refresh_mapping_with_reason(FetchReason::LaneMissing)
            .await
        {
            warn!(
                host = %self.config.host,
                error = %format!("{err:#}"),
                "resolume lane refetch failed; pushing with the cached mapping"
            );
            return;
        }
        let still_missing = self.lanes_to_refetch_for(push);
        if !still_missing.is_empty() {
            warn!(
                host = %self.config.host,
                deck = ?self.provisional.selected_deck,
                still_missing = ?still_missing,
                "resolume deck still has no clips for these lanes after a fresh fetch; such lines are skipped on this deck until the deck changes, the composition is complete again or Settings > Refresh mapping"
            );
        }
    }

    /// `GET /composition/decks/by-id/{selected}`. Any failure keeps the cached
    /// mapping: a deck check must never cost a push.
    async fn check_selected_deck(&mut self) -> DeckCheck {
        let Some(deck_id) = self.provisional.selected_deck else {
            return DeckCheck::Keep;
        };
        let started = Instant::now();
        let answer = self.request_deck_state(deck_id).await;
        debug!(
            target: "presenter::resolume::timing",
            host = %self.config.host,
            deck_id,
            t_deck_check_ms = duration_ms(started.elapsed()),
            answer = ?answer.as_ref().ok(),
            "resolume deck check"
        );
        match answer {
            Ok(DeckState::Selected) => DeckCheck::Selected,
            Ok(state) => {
                info!(
                    host = %self.config.host,
                    deck_id,
                    deck_state = ?state,
                    "resolume deck switched; refetching the composition"
                );
                DeckCheck::Switched
            }
            Err(err) => {
                let error = format!("{err:#}");
                if self.provisional.should_warn_deck_check(Instant::now()) {
                    warn!(host = %self.config.host, deck_id, %error, "resolume deck check failed; pushing with the cached mapping");
                } else {
                    debug!(host = %self.config.host, deck_id, %error, "resolume deck check failed; pushing with the cached mapping");
                }
                DeckCheck::Keep
            }
        }
    }

    async fn request_deck_state(&mut self, deck_id: i64) -> anyhow::Result<DeckState> {
        let endpoint = self.endpoint().await?;
        let url = format!("{}/composition/decks/by-id/{deck_id}", endpoint.base_url);
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .timeout(ACTION_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("deck check GET {url} failed"))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(DeckState::Gone);
        }
        if !status.is_success() {
            return Err(anyhow!("deck check GET {url} failed with status {status}"));
        }
        let body = response
            .json::<Value>()
            .await
            .with_context(|| format!("invalid deck JSON from {url}"))?;
        match deck_selected(&body) {
            Some(true) => Ok(DeckState::Selected),
            Some(false) => Ok(DeckState::NotSelected),
            None => Err(anyhow!("deck check GET {url} has no selected flag")),
        }
    }
}
