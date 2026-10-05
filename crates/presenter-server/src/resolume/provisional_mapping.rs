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
//!   compared with that deck's last good destination kinds (a body without
//!   decks, Arena mid-load, with the deck selected before). A mapping lacking a
//!   kind the deck had, or with no recognized destination for a deck without
//!   history, is *suspect*: up to five follow-up fetches run
//!   [`FOLLOW_UP_DELAYS`] apart and stop the moment it is complete again. If
//!   the last two follow-ups saw the same composition (kinds and clip count),
//!   it is what the deck holds and becomes its reference.
//! - Before every stage/Bible push ([`HostDriver::ensure_mapping_for_push`]),
//!   and on the 10 s tick after the `/product` probe, a deck check
//!   `GET /composition/decks/by-id/{id}` (~360 B). `selected:false` or 404
//!   drops the mapping: the push refetches it inline (never rate-limited) and
//!   lands on the new deck. A failed check never blocks the push.
//! - A push that needs a lane the cached mapping lacks but should have (the
//!   mapping is provisional: Arena loading, or a deck that never produced a
//!   complete mapping) refetches before it is applied. Every follow-up step
//!   (failed or not) re-arms it; after a refetch that left the lane empty, or
//!   a failed one, the next waits only [`LANE_REFETCH_RETRY`] (3 s): no text
//!   is skipped for longer. A deck accepted as lacking the lane never
//!   refetches for it.
//! - A host or deck that never had a kind (Bridge PP has no `#main`, SNV no
//!   `#translate`) never refetches for it.

use super::clip_map::{BIBLE_CLEAR_KIND, BIBLE_LANE_KINDS, MAIN_KINDS, TRANSLATION_KINDS};
use super::driver::{duration_ms, FetchReason, HostDriver, MappingFetchOutcome, ACTION_TIMEOUT};
use super::error_kind::{classify_error, ResolumeErrorKind};
use super::{BibleUpdate, ResolumeConnectionSnapshot, StageUpdate};
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
/// Owner rule (2026-10-05): no text may be skipped. While the mapping is
/// provisional (Arena loading, or a deck that never produced a complete
/// mapping in this process), a push that needs a missing lane refetches when
/// the previous lane refetch, empty or failed, finished at least this long
/// ago (a follow-up step re-arms it sooner). A line is skipped for at most
/// this long. Only a deck accepted as lacking a kind stops refetching for it
/// (its last-good reference has no such kind, so it never refetches).
pub(super) const LANE_REFETCH_RETRY: Duration = Duration::from_secs(3);
/// After a lane refetch that TIMED OUT (`COMPOSITION_TIMEOUT`, 15 s): a
/// hanging `/composition` is host trouble, the exception the owner rule
/// allows. Retrying every 3 s would block the serial host worker 15 s of
/// every 18 s, and every queued line (placeable lanes, Bible, timer frames)
/// would wait behind it until the 16-slot command channel drops updates.
pub(super) const LANE_REFETCH_AFTER_TIMEOUT: Duration = Duration::from_secs(30);

/// How long after a failed lane refetch the next one may run: 30 s after a
/// timeout, [`LANE_REFETCH_RETRY`] after any fast failure (refused, reset, a
/// 5xx).
pub(super) fn lane_refetch_retry_after(err: &anyhow::Error) -> Duration {
    match classify_error(err) {
        ResolumeErrorKind::Timeout => LANE_REFETCH_AFTER_TIMEOUT,
        _ => LANE_REFETCH_RETRY,
    }
}

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

/// Whether a push that needs a missing lane may refetch the composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LaneRefetch {
    /// The next such push refetches. Re-armed by every follow-up step (even
    /// a failed one), a deck change, a complete fetch and an operator refresh.
    #[default]
    Armed,
    /// Used: allowed again from this instant, [`LANE_REFETCH_RETRY`] after
    /// the last lane refetch (a follow-up step due sooner re-arms it then).
    RetryAt(Instant),
}

impl LaneRefetch {
    fn allowed(self, now: Instant) -> bool {
        match self {
            Self::Armed => true,
            Self::RetryAt(at) => now >= at,
        }
    }
}

/// How one fetched mapping compares with its deck's last good mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FetchVerdict {
    /// Every kind the deck's last good mapping had is there.
    Complete { follow_ups_stopped: bool },
    /// An operator refresh: its kinds are the deck's reference from now on.
    Adopted,
    /// The follow-ups ran out and the last two saw the same composition
    /// (kinds and clip count): it is what the deck holds now (e.g. clips
    /// removed on purpose).
    Settled { lacking: Vec<&'static str> },
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
    /// The selected deck the cached composition lists; `None` when it lists
    /// no decks (nothing to check then).
    pub(super) selected_deck: DeckKey,
    /// The deck the cached mapping is judged against. A body without decks
    /// (Arena mid-load) keeps the deck selected before.
    pub(super) reference_deck: DeckKey,
    /// Destination kinds of each deck's last complete (or settled) mapping.
    pub(super) last_good: HashMap<DeckKey, Vec<&'static str>>,
    /// Kinds and clip count of the previous follow-up of the running
    /// schedule: the same composition at the last two follow-ups (about a
    /// minute apart) is settled, a changing one is still loading.
    last_follow_up: Option<(Vec<&'static str>, usize)>,
    /// The next follow-up fetch while the cached mapping is suspect.
    pub(super) follow_up: Option<FollowUp>,
    pub(super) lane_refetch: LaneRefetch,
    /// The last deck check answered 404 for this deck id.
    deck_gone: Option<i64>,
    /// A deck the check calls gone (404) although a fresh composition still
    /// selects it: its 404s no longer count as a switch, so a broken deck
    /// answer cannot turn every push and tick into a composition fetch.
    pub(super) contradicted_deck: Option<i64>,
}

impl ProvisionalMapping {
    /// Record a fetched mapping of `deck` with these destination `kinds`
    /// and `clip_count` clips in all.
    pub(super) fn note_fetch(
        &mut self,
        deck: DeckKey,
        kinds: Vec<&'static str>,
        clip_count: usize,
        reason: FetchReason,
        now: Instant,
    ) -> FetchVerdict {
        // A body without decks right after bodies with decks is Arena
        // mid-load: suspect even with every kind, so the follow-ups bring the
        // deck list (and with it the deck check) back.
        let deckless_after_decks = deck.is_none() && self.reference_deck.is_some();
        let reference = deck.or(self.reference_deck);
        if reference != self.reference_deck {
            self.lane_refetch = LaneRefetch::Armed;
            self.last_follow_up = None;
        }
        self.track_deck_contradiction(deck, reason);
        self.selected_deck = deck;
        self.reference_deck = reference;
        let previous_follow_up = if matches!(reason, FetchReason::FollowUp) {
            self.last_follow_up.replace((kinds.clone(), clip_count))
        } else {
            None
        };
        if matches!(reason, FetchReason::Manual) && !kinds.is_empty() {
            // Refresh mapping is how an intentional clip edit reaches
            // Presenter, so the operator's result becomes the reference.
            self.last_good.insert(reference, kinds);
            self.follow_up = None;
            self.last_follow_up = None;
            self.lane_refetch = LaneRefetch::Armed;
            self.contradicted_deck = None;
            return FetchVerdict::Adopted;
        }
        let (lacking, has_history) = self.lacking_vs_history(reference, &kinds);
        let no_destinations = kinds.is_empty();
        let suspect = deckless_after_decks
            || if has_history {
                !lacking.is_empty()
            } else {
                no_destinations
            };
        if !suspect {
            self.last_good.insert(reference, kinds);
            self.lane_refetch = LaneRefetch::Armed;
            self.last_follow_up = None;
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
            | FetchReason::DeckChanged => self.start_follow_ups(now),
        };
        let stable = previous_follow_up
            .as_ref()
            .is_some_and(|(previous, count)| *previous == kinds && *count == clip_count);
        if schedule == ScheduleChange::Exhausted && settles(deck, has_history, &kinds, stable) {
            self.last_good.insert(reference, kinds);
            return FetchVerdict::Settled { lacking };
        }
        FetchVerdict::Suspect {
            lacking,
            no_destinations,
            schedule,
        }
    }

    /// The kinds the `reference` deck's last good mapping had that `kinds`
    /// lacks, and whether that deck has a history at all.
    fn lacking_vs_history(
        &self,
        reference: DeckKey,
        kinds: &[&'static str],
    ) -> (Vec<&'static str>, bool) {
        match self.last_good.get(&reference) {
            Some(good) => (
                good.iter()
                    .copied()
                    .filter(|kind| !kinds.contains(kind))
                    .collect(),
                true,
            ),
            None => (Vec::new(), false),
        }
    }

    /// A suspect fetch (other than a follow-up or a lane refetch) starts the
    /// schedule over.
    fn start_follow_ups(&mut self, now: Instant) -> ScheduleChange {
        self.follow_up = Some(FollowUp {
            step: 0,
            due: now + FOLLOW_UP_DELAYS[0],
        });
        self.last_follow_up = None;
        ScheduleChange::Started
    }

    /// A deck-changed refetch that still selects the deck the check called
    /// gone marks it contradicted; a fetch selecting another deck clears it.
    fn track_deck_contradiction(&mut self, deck: DeckKey, reason: FetchReason) {
        let gone = self.deck_gone.take();
        if matches!(reason, FetchReason::DeckChanged) && gone.is_some() && deck == gone {
            self.contradicted_deck = gone;
        } else if deck.is_some() && deck != self.contradicted_deck {
            self.contradicted_deck = None;
        }
    }

    /// Move past the current follow-up step: schedule the next one, or end
    /// the schedule after the last. Every step, failed or not, re-arms the
    /// lane refetch for the next push.
    pub(super) fn advance_follow_up(&mut self, now: Instant) -> ScheduleChange {
        self.lane_refetch = LaneRefetch::Armed;
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
    /// should refetch: the deck's last good mapping had it, or the deck has
    /// no history and the cached mapping no recognized destination at all
    /// (Arena loading at a cold start).
    fn lane_expected(&self, kind: &str, mapping_has_destinations: bool) -> bool {
        match self.last_good.get(&self.reference_deck) {
            Some(good) => good.contains(&kind),
            None => !mapping_has_destinations,
        }
    }

    /// A lane refetch left a lane this push should have missing (or failed):
    /// the next one waits [`LANE_REFETCH_RETRY`]. A follow-up step due sooner
    /// re-arms it at that step (`advance_follow_up`).
    fn spend_lane_refetch(&mut self, now: Instant) {
        self.lane_refetch = LaneRefetch::RetryAt(now + LANE_REFETCH_RETRY);
    }
}

/// After the last follow-up: the composition is what the deck holds when the
/// last two follow-ups saw the same one (`stable`), and it has recognized
/// clips or is a listed deck with no history (a deck without presenter clips,
/// such as a video deck). A deck that had clips and still shows none keeps
/// being re-checked on pushes: that is Arena still loading.
fn settles(deck: DeckKey, has_history: bool, kinds: &[&'static str], stable: bool) -> bool {
    stable && (!kinds.is_empty() || (deck.is_some() && !has_history))
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

/// The lanes a stage push writes.
pub(super) fn stage_required_kinds(update: &StageUpdate) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if update.current_main.is_some() {
        kinds.extend(MAIN_KINDS);
    }
    if update.current_translation.is_some() {
        kinds.extend(TRANSLATION_KINDS);
    }
    kinds
}

/// The lanes a Bible push writes; a clear also triggers `#bible-clear`.
pub(super) fn bible_required_kinds(update: &BibleUpdate) -> Vec<&'static str> {
    let mut kinds = BIBLE_LANE_KINDS.to_vec();
    if update.slide_output.is_none() && update.passage.is_none() {
        kinds.push(BIBLE_CLEAR_KIND);
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
        clip_count: usize,
        reason: FetchReason,
    ) {
        let contradicted_before = self.provisional.contradicted_deck;
        let verdict = self
            .provisional
            .note_fetch(deck, kinds, clip_count, reason, Instant::now());
        let host = &self.config.host;
        if let Some(deck_id) = self.provisional.contradicted_deck {
            if contradicted_before != Some(deck_id) {
                warn!(
                    %host,
                    deck_id,
                    "resolume deck check answered 404 for a deck the fresh composition still selects; its 404s no longer count as a deck switch"
                );
            }
        }
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
            FetchVerdict::Settled { lacking } => warn!(
                %host,
                deck = ?deck,
                lacking = ?lacking,
                "resolume composition did not change over the follow-up fetches; accepting it as this deck's content (lines for the lacking clips are skipped)"
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
                no_deck_list = deck.is_none(),
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
                retry_secs = LANE_REFETCH_RETRY.as_secs(),
                "resolume mapping still incomplete after every follow-up fetch; follow-ups stopped. A push that needs a missing lane refetches, at most once per retry_secs; Settings > Refresh mapping re-reads it now"
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
            // A down host: try this step again when its backoff window ends
            // (`in_backoff` means `next_retry_at` is set and in the future).
            if let Some(retry_at) = self.next_retry_at {
                self.provisional.follow_up = Some(FollowUp {
                    due: retry_at,
                    ..follow_up
                });
            }
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

    /// The push path's `ensure_mapping` (stage and Bible; timer frames use the
    /// plain one): follow a deck switch, fetch a missing mapping, then refetch
    /// if the mapping lacks a lane in `required` that it should have. The
    /// outcome's `refetched` covers every fetch, for the push audit.
    pub(super) async fn ensure_mapping_for_push(
        &mut self,
        required: &[&'static str],
    ) -> anyhow::Result<MappingFetchOutcome> {
        if self.mapping.is_some() && self.check_selected_deck().await == DeckCheck::Switched {
            // `ensure_mapping` refetches inline, never rate-limited, and the
            // push lands on the new deck. If that fetch fails, the push fails
            // like any composition fetch: it never writes to the ids of a
            // deck that left the wall.
            self.invalidate_mapping(FetchReason::DeckChanged);
        }
        let outcome = self.ensure_mapping().await?;
        if outcome.refetched {
            // This push fetched the mapping itself: nothing fresher to get.
            return Ok(outcome);
        }
        let refetched = self.refetch_for_missing_lane(required).await;
        Ok(MappingFetchOutcome { refetched })
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

    /// The kinds in `required` that the cached mapping lacks but should have.
    fn lanes_to_refetch_for(&self, required: &[&'static str]) -> Vec<&'static str> {
        let Some(mapping) = &self.mapping else {
            return Vec::new();
        };
        let kinds = mapping.destination_kinds();
        let has_destinations = !kinds.is_empty();
        required
            .iter()
            .copied()
            .filter(|kind| {
                !kinds.contains(kind) && self.provisional.lane_expected(kind, has_destinations)
            })
            .collect()
    }

    /// Refetch for a push whose lane the cached mapping lacks but should
    /// have, when [`LaneRefetch`] allows it. Returns whether it fetched. A
    /// failed refetch only logs: the push goes out on the cached mapping.
    async fn refetch_for_missing_lane(&mut self, required: &[&'static str]) -> bool {
        if !self.provisional.lane_refetch.allowed(Instant::now()) {
            return false;
        }
        let lacking = self.lanes_to_refetch_for(required);
        if lacking.is_empty() {
            return false;
        }
        info!(
            host = %self.config.host,
            deck = ?self.provisional.selected_deck,
            lacking = ?lacking,
            "resolume push needs clips the cached mapping lacks (Arena may have been loading); refetching the composition"
        );
        if let Err(err) = self
            .refresh_mapping_with_reason(FetchReason::LaneMissing)
            .await
        {
            let retry = lane_refetch_retry_after(&err);
            self.provisional.lane_refetch = LaneRefetch::RetryAt(Instant::now() + retry);
            self.log_lane_refetch_failed(&err, retry);
            return false;
        }
        let still_missing = self.lanes_to_refetch_for(required);
        if !still_missing.is_empty() {
            self.provisional.spend_lane_refetch(Instant::now());
            self.log_lane_still_missing(&still_missing);
        }
        true
    }

    /// One WARN per 300 s per host (#563h limiter), DEBUG otherwise: with a
    /// refetch every 3 s during a long load the WARN would flood the log.
    fn log_lane_still_missing(&mut self, still_missing: &[&'static str]) {
        let warn_now = self.should_warn_missing_clip("lane-refetch");
        let host = &self.config.host;
        let deck = self.provisional.selected_deck;
        let retry_secs = LANE_REFETCH_RETRY.as_secs();
        let message = "resolume composition still lacks these clips after a fresh fetch (Arena may still be loading); such lines are skipped until the next refetch (at most retry_secs, or the next follow-up step)";
        if warn_now {
            warn!(%host, deck = ?deck, still_missing = ?still_missing, retry_secs, "{message}");
        } else {
            debug!(%host, deck = ?deck, still_missing = ?still_missing, retry_secs, "{message}");
        }
    }

    /// A failed lane refetch: the push goes out on the cached mapping. WARN at
    /// most once per 300 s per host (#563h limiter), DEBUG otherwise.
    fn log_lane_refetch_failed(&mut self, err: &anyhow::Error, retry: Duration) {
        let warn_now = self.should_warn_missing_clip("lane-refetch-failed");
        let host = &self.config.host;
        let error = format!("{err:#}");
        let retry_secs = retry.as_secs();
        let message = "resolume lane refetch failed; pushing with the cached mapping";
        if warn_now {
            warn!(%host, %error, retry_secs, "{message}");
        } else {
            debug!(%host, %error, retry_secs, "{message}");
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
            Ok(DeckState::Gone) if self.provisional.contradicted_deck == Some(deck_id) => {
                debug!(host = %self.config.host, deck_id, "resolume deck check 404 for a contradicted deck; keeping the mapping");
                DeckCheck::Keep
            }
            Ok(state) => {
                if state == DeckState::Gone {
                    self.provisional.deck_gone = Some(deck_id);
                }
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
                // #563h's per-key limiter: one WARN per 300 s per host.
                if self.should_warn_missing_clip("deck-check") {
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
