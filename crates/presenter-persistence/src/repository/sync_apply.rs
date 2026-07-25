//! #555 LWW reconciliation core: apply one peer song onto the local DB.
//!
//! Invariants that keep the two instances convergent:
//! - a peer row is applied only when STRICTLY newer than what we hold (or unknown);
//! - an applied row stores the PEER's `updated_at`, never `now()` — an applied change
//!   is not a new edit, which is what prevents echo/ping-pong loops;
//! - adopt-by-name updates the EXISTING local row in place (its presentation id — and
//!   with it every playlist reference — survives; only the `sync_id` is adopted).
use super::util::build_slide_active_model;
use super::Repository;
use crate::entities::{
    library, presentation as presentation_entity, slide as slide_entity, slide_stage_layout,
};
use crate::SyncPresentation;
use chrono::{DateTime, Utc};
use presenter_core::search::fold_query;
use presenter_core::{Slide, SlideContent, SlideText};
use sea_orm::{
    sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use tracing::{info, instrument, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncApplyOutcome {
    Created,
    Updated,
    AdoptedByName,
    SkippedNotNewer,
}

impl SyncApplyOutcome {
    /// Did this apply WRITE to the DB? (Drives the no-echo/audit counts.)
    pub fn wrote(self) -> bool {
        !matches!(self, SyncApplyOutcome::SkippedNotNewer)
    }
}

/// Trash is retained this long before the periodic background task
/// (`state/mod.rs`) hard-deletes it, AND — the SAME constant, never a
/// second copy (#558 round-3 T8) — this is the age past which an UNKNOWN
/// tombstone (no matching local row at all) is presumed already-pruned
/// rather than merely never-seen (#558 R2/S7). One shared `pub const` so
/// the prune task and `sync_should_apply` can never drift apart.
pub const PRUNE_HORIZON: chrono::Duration = chrono::Duration::days(30);

/// LWW: apply the peer row iff it is strictly newer than what we hold (or unknown).
/// `local` is `None` when we have no matching song at all. `peer_deleted` is
/// whether the PEER's row is currently a tombstone.
///
/// #558 S7 + R2: when `local` is `None` AND the peer entry is a tombstone,
/// this distinguishes "we've never held this row at all" (a
/// fresh/re-provisioned peer's first sync — the tombstone should be
/// created locally so trash contents converge) from "we pruned it
/// ourselves already" (S7's actual concern) by the tombstone's AGE against
/// the 30-day prune horizon: a genuinely pruned row's tombstone predates
/// it (real pruning never touches a fresher row), so only a tombstone
/// OLDER than the horizon is skipped. The OLD rule (`!peer_deleted`,
/// unconditional) treated every unknown tombstone the same, so a fresh
/// peer's first sync permanently skipped anything already trashed on the
/// other side and the two instances' trash contents diverged forever. A
/// peer delete of a row we DO still hold (`local: Some(_)`) is unaffected
/// — that's normal trash propagation, gated on `peer > local` like any
/// other edit.
pub fn sync_should_apply(
    peer: DateTime<Utc>,
    peer_deleted: bool,
    local: Option<DateTime<Utc>>,
) -> bool {
    match local {
        None => !peer_deleted || Utc::now() - peer < PRUNE_HORIZON,
        Some(local) => peer > local,
    }
}

impl Repository {
    /// `peer_sync_ids` is the FULL set of sync_ids the peer's manifest
    /// currently reports (#558 round-4 U2) — pass an empty set when no
    /// such context exists (a standalone apply outside a real sync cycle).
    ///
    /// #558 X5: returns the LOCAL presentation id this apply actually
    /// WROTE — `Some(id)` whenever `outcome.wrote()` is true (updated,
    /// adopted-by-name, or newly created), `None` for a skipped
    /// (never-write) apply. The caller (`state/sync.rs`) uses this to evict
    /// the RIGHT cache entry: `resolve_sync_apply_target` picks a lock
    /// target BEFORE this transaction starts, and a narrow probe→
    /// transaction gap can in theory let this apply resolve a DIFFERENT
    /// row than the one that was locked (e.g. an adopt-by-name candidate
    /// that only became eligible/ineligible in between). The DB write
    /// itself is already transaction-serialized and LWW-safe regardless;
    /// this return value just lets the caller ALSO evict the id it truly
    /// wrote, in addition to the pre-resolved lock target, so a divergence
    /// never leaves a stale cached snapshot un-evicted.
    #[instrument(skip_all, fields(sync_id = %incoming.sync_id, name = %incoming.name))]
    pub async fn apply_sync_presentation(
        &self,
        incoming: &SyncPresentation,
        peer_sync_ids: &HashSet<String>,
    ) -> anyhow::Result<(SyncApplyOutcome, Option<presenter_core::PresentationId>)> {
        let txn = self.db.begin().await?;

        // 1. Match by sync_id — independent of the library (whether the
        // peer's library exists locally yet, or has been renamed, is
        // resolved by `ensure_library` ONLY once we know we're writing —
        // #558 round-4 U4).
        let by_sync = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::SyncId.eq(incoming.sync_id.clone()))
            .one(&txn)
            .await?;

        if let Some(existing) = by_sync {
            let local_updated: DateTime<Utc> = existing.updated_at.into();
            if !sync_should_apply(
                incoming.updated_at,
                incoming.deleted_at.is_some(),
                Some(local_updated),
            ) {
                txn.commit().await?;
                info!("sync skip (not newer)");
                return Ok((SyncApplyOutcome::SkippedNotNewer, None));
            }
            // #578: a TOMBSTONE keeps the presentation under its EXISTING
            // library — never ensure/create a live library for a row being
            // trashed. When the peer deleted the whole library, the local
            // library is being tombstoned in the SAME cycle (library recon
            // runs first), so `ensure_library`'s live-only lookup would miss
            // it and create a fresh EMPTY LIVE library shell to hold the
            // tombstone. Only a LIVE apply (re)attaches to the peer's library.
            let library_id = if incoming.deleted_at.is_some() {
                existing.library_id.clone()
            } else {
                Self::ensure_library(&txn, &incoming.library_name, incoming.updated_at).await?
            };
            Self::write_synced_row(&txn, &existing.id, &library_id, incoming).await?;
            txn.commit().await?;
            info!("sync updated");
            let id =
                presenter_core::PresentationId::from_uuid(uuid::Uuid::parse_str(&existing.id)?);
            return Ok((SyncApplyOutcome::Updated, Some(id)));
        }

        // 2. Adopt-by-name: same name in the same-named library, unknown
        // sync_id — ONLY for a LIVE peer entry (#558 round-3 Decision B,
        // fixes T1). A peer TOMBSTONE with an unknown sync_id must NEVER
        // adopt a live local song by name: two different sites can
        // independently hold DIFFERENT songs that happen to share a name
        // (different sync_ids), so trashing one of them locally must never
        // reach across and trash the OTHER site's unrelated same-named
        // song. A tombstone therefore skips this step entirely and falls
        // through to step 3, which creates its own separate trashed row
        // instead — never touching any existing local row.
        //
        // #558 round-4 U4: this step only LOOKS UP the library (never
        // creates it) — a library that doesn't exist locally yet cannot
        // hold any adoption candidate, so a missing library is simply "no
        // candidates", not a reason to create one speculatively.
        if incoming.deleted_at.is_none() {
            if let Some(library_id) = Self::find_library_id(&txn, &incoming.library_name).await? {
                if let Some(result) =
                    Self::try_adopt_by_name(&txn, &library_id, incoming, peer_sync_ids).await?
                {
                    txn.commit().await?;
                    return Ok(result);
                }
            }
        }

        // 3. Unknown sync_id → the tombstone-aware create-or-skip helper.
        // Ensures/creates the library only once the horizon gate inside it
        // decides to WRITE (#558 round-4 U4) — a skipped (never-write)
        // apply must never leave behind a phantom, permanently-empty
        // library.
        let result = Self::apply_unknown_sync_id(&txn, incoming).await?;
        txn.commit().await?;
        Ok(result)
    }

    /// Look up an existing LIVE library by name — NEVER creates one (#558
    /// round-4 U4). Used wherever "no library" can be treated as "nothing
    /// to do" without paying for a write. Generic over the connection
    /// (#558 W5/W8/W9) so the SAME implementation serves both the real
    /// apply (inside its transaction) and `resolve_sync_apply_target` (a
    /// plain pre-transaction probe, no transaction involved).
    ///
    /// #580: a same-named TOMBSTONED library is deliberately invisible here
    /// — `ensure_library` (the only caller that may WRITE) decides that case
    /// itself via LWW against the incoming presentation's `updated_at`,
    /// rather than treating "no live match" as "nothing exists".
    async fn find_library_id<C: sea_orm::ConnectionTrait>(
        conn: &C,
        library_name: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(library::Entity::find()
            .filter(library::Column::Name.eq(library_name.to_string()))
            .filter(library::Column::DeletedAt.is_null())
            .one(conn)
            .await?
            .map(|l| l.id))
    }

    /// The single LIVE (non-trashed) local presentation row matching
    /// `library_id` + `name`, if unambiguous — the adopt-by-name candidate
    /// rule (#558 S4: `.one()` with no ORDER BY picks an arbitrary row among
    /// duplicates, so the full candidate set is fetched, filtered to live
    /// rows, and ordered deterministically; 0 or 2+ candidates means "no
    /// unambiguous match", never a guess).
    ///
    /// Generic over the connection (#558 W5/W8/W9) so this ONE
    /// implementation serves BOTH `try_adopt_by_name` (inside the real
    /// apply transaction) and `resolve_sync_apply_target` (a plain
    /// pre-transaction probe used only to pick a lock target) — the two can
    /// no longer independently drift apart. That drift is exactly what let
    /// the OLD, separately-implemented probe (`repository/sync.rs`'s
    /// `find_live_presentation_id_by_name`, since deleted) lock the WRONG
    /// target: it never gained the `peer_sync_ids` single-shot exclusion
    /// `try_adopt_by_name` picked up in round-4 U2, so it could report a
    /// candidate the real apply would actually skip (creating a new row
    /// instead) — locking a row nothing was about to touch, while the row
    /// ACTUALLY written held no lock at all.
    async fn find_live_by_name_candidate<C: sea_orm::ConnectionTrait>(
        conn: &C,
        library_id: &str,
        name: &str,
    ) -> anyhow::Result<Option<presentation_entity::Model>> {
        let mut candidates = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.eq(library_id.to_string()))
            .filter(presentation_entity::Column::Name.eq(name.to_string()))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .order_by_asc(presentation_entity::Column::CreatedAt)
            .order_by_asc(presentation_entity::Column::Id)
            .all(conn)
            .await?;
        if candidates.len() != 1 {
            return Ok(None);
        }
        Ok(Some(candidates.remove(0)))
    }

    /// Resolve the LOCAL presentation id a sync apply of `incoming` will
    /// touch, WITHOUT starting a transaction or writing anything (#558
    /// W5/W8/W9) — matches `apply_sync_presentation`'s own step 1 (by
    /// `sync_id`) then step 2 (adopt-by-name, respecting the SAME
    /// `peer_sync_ids` single-shot exclusion) via the IDENTICAL query
    /// helpers, so it can never drift from what the real apply decides.
    ///
    /// Used ONLY to pick a lock target BEFORE the real apply transaction
    /// runs: the caller (`state/sync.rs`) fetches the peer's content FIRST,
    /// holding NO lock — the ~15s network round-trip must never happen
    /// while blocking a concurrent edit op on this presentation (#558 W5) —
    /// then calls this on a plain (non-transactional) connection to learn
    /// the target, acquires the per-presentation lock for it, and THEN
    /// calls `apply_sync_presentation`, which re-resolves the identical
    /// target under that lock. `None` means either a genuinely new identity
    /// (nothing exists yet to lock) or an ambiguous/excluded adopt-by-name
    /// candidate (the real apply will CREATE instead) — neither case has an
    /// existing row a concurrent edit op could be touching.
    #[instrument(skip_all, fields(sync_id = %incoming.sync_id, name = %incoming.name))]
    pub async fn resolve_sync_apply_target(
        &self,
        incoming: &SyncPresentation,
        peer_sync_ids: &HashSet<String>,
    ) -> anyhow::Result<Option<presenter_core::PresentationId>> {
        if let Some(existing) = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::SyncId.eq(incoming.sync_id.clone()))
            .one(&self.db)
            .await?
        {
            return Ok(Some(presenter_core::PresentationId::from_uuid(
                uuid::Uuid::parse_str(&existing.id)?,
            )));
        }
        if incoming.deleted_at.is_some() {
            return Ok(None);
        }
        let Some(library_id) = Self::find_library_id(&self.db, &incoming.library_name).await?
        else {
            return Ok(None);
        };
        let Some(candidate) =
            Self::find_live_by_name_candidate(&self.db, &library_id, &incoming.name).await?
        else {
            return Ok(None);
        };
        if peer_sync_ids.contains(&candidate.sync_id) {
            return Ok(None);
        }
        Ok(Some(presenter_core::PresentationId::from_uuid(
            uuid::Uuid::parse_str(&candidate.id)?,
        )))
    }

    /// Look up an existing LIVE library by name, or REVIVE-or-attach-to a
    /// same-named TOMBSTONED one, or create a brand-new live library if
    /// neither exists (#558 round-4 U4; #580). Call this ONLY once the
    /// caller is committed to writing a presentation row into it — never
    /// speculatively, or a skipped/never-write apply leaves behind a
    /// phantom, permanently-empty library row.
    ///
    /// #580: previously, the moment no LIVE same-named library existed, this
    /// ALWAYS minted a fresh one — even when a TOMBSTONED library of that
    /// name already existed. That let a peer's concurrent
    /// library-delete-vs-presentation-create race diverge the two instances
    /// forever (one side ends up with a resurrected library holding the new
    /// presentation, the other still shows the library hidden with the
    /// presentation orphaned underneath it). Fix (decision on #580, option
    /// b): decide live-vs-tombstoned from the TOMBSTONED library's own
    /// `updated_at` against the incoming presentation's `updated_at` — the
    /// SAME LWW mechanism `apply_sync_library` already uses for library
    /// manifest sync (#578's `updated_at`/`sync_id`/`deleted_at` columns).
    /// `presentation_updated_at > library_updated` (strictly, mirroring
    /// `sync_should_apply`'s tie-break) revives the library in place —
    /// clearing `deleted_at` and bumping `updated_at` to the presentation's
    /// own timestamp so the peer's next pull sees a strictly newer live
    /// library row and revives its own copy too. Otherwise the library
    /// stays tombstoned and the presentation simply attaches to it (hidden,
    /// matching what the peer that did the deleting already shows). Either
    /// way the EXISTING library row is reused — its identity (`sync_id`)
    /// never changes, and reviving it never revives any presentation
    /// tombstoned under it (those carry their own tombstones with their own
    /// `updated_at` and settle by their own LWW).
    async fn ensure_library(
        txn: &sea_orm::DatabaseTransaction,
        library_name: &str,
        presentation_updated_at: DateTime<Utc>,
    ) -> anyhow::Result<String> {
        if let Some(id) = Self::find_library_id(txn, library_name).await? {
            return Ok(id);
        }
        // Order by UpdatedAt DESC, SyncId DESC: repeated delete/recreate
        // cycles of the same-named library leave MULTIPLE tombstoned rows
        // behind (the partial unique index only guards LIVE rows — see
        // idx_libraries_name_live_unique), and `.one()` with no explicit
        // order picks an arbitrary one. Always deciding against the MOST
        // RECENT tombstone is the one that's actually relevant to this
        // race; an older, already-superseded tombstone would compare the
        // presentation against the wrong timestamp entirely. `SyncId` is
        // the tie-breaker for an EXACT `updated_at` collision (two rapid
        // delete/recreate cycles can land on the same wall-clock
        // millisecond) — it MUST be a peer-stable key, never a local-only
        // one like the row `Id` (`find_live_by_name_candidate`'s own
        // secondary sort), or two peers holding the same two tombstoned
        // rows could break the tie in DIFFERENT directions and pick a
        // DIFFERENT "most recent" tombstone each — reintroducing the exact
        // divergence class this fix closes. `sync_id` converges identically
        // on both sides once synced, so ordering by it is deterministic
        // peer-to-peer.
        if let Some(tombstoned) = library::Entity::find()
            .filter(library::Column::Name.eq(library_name.to_string()))
            .filter(library::Column::DeletedAt.is_not_null())
            .order_by_desc(library::Column::UpdatedAt)
            .order_by_desc(library::Column::SyncId)
            .one(txn)
            .await?
        {
            let library_updated: DateTime<Utc> = tombstoned.updated_at.into();
            if presentation_updated_at > library_updated {
                library::Entity::update_many()
                    .col_expr(
                        library::Column::DeletedAt,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(
                        library::Column::UpdatedAt,
                        Expr::value(presentation_updated_at.to_rfc3339()),
                    )
                    .filter(library::Column::Id.eq(tombstoned.id.clone()))
                    .filter(library::Column::DeletedAt.is_not_null())
                    .exec(txn)
                    .await?;
            }
            return Ok(tombstoned.id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        library::Entity::insert(library::ActiveModel {
            id: Set(id.clone()),
            name: Set(library_name.to_string()),
            search_name: Set(fold_query(library_name)),
            created_at: Set(Utc::now().into()),
            // #578: a library implicitly created to hold a synced presentation
            // is live with a fresh sync identity; the library manifest sync
            // converges that identity with the peer via the name-match adopt.
            updated_at: Set(Utc::now().into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(None),
        })
        .exec(txn)
        .await?;
        Ok(id)
    }

    /// #578: resolve a library id for a brand-new TOMBSTONED presentation
    /// WITHOUT creating an empty LIVE library shell. Attach to any existing
    /// same-named library (live preferred, then tombstoned); if none exists,
    /// create a TOMBSTONED library (its identity converges separately via the
    /// peer's library manifest). Used only by `apply_unknown_sync_id` for a
    /// tombstone — a live entry keeps using `ensure_library`.
    async fn ensure_library_for_tombstone(
        txn: &sea_orm::DatabaseTransaction,
        library_name: &str,
    ) -> anyhow::Result<String> {
        // order_by DeletedAt ASC → live rows (NULL sorts first in SQLite) win
        // over a tombstoned same-named row when both exist.
        if let Some(row) = library::Entity::find()
            .filter(library::Column::Name.eq(library_name.to_string()))
            .order_by_asc(library::Column::DeletedAt)
            .one(txn)
            .await?
        {
            return Ok(row.id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        library::Entity::insert(library::ActiveModel {
            id: Set(id.clone()),
            name: Set(library_name.to_string()),
            search_name: Set(fold_query(library_name)),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(Some(now.into())),
        })
        .exec(txn)
        .await?;
        Ok(id)
    }

    /// Step 2 of `apply_sync_presentation`: adopt-by-name among LIVE
    /// (non-trashed) candidates only (#558 S4). A trashed local row must
    /// never be silently resurrected via adoption, and an AMBIGUOUS match
    /// (2+ live candidates sharing the same name) must never guess which
    /// one to adopt — both fall through to `None` (step 3 creates
    /// instead). `.one()` with no ORDER BY picks an arbitrary row, so the
    /// candidate set is fetched in full, filtered to live rows, ordered
    /// deterministically, and adoption proceeds ONLY when exactly one live
    /// candidate remains.
    ///
    /// Returns `Some((outcome, id))` when a final decision was reached
    /// (adopted, or skipped because local is newer — `id` is `Some` only
    /// for the former, per #558 X5); `None` when there is nothing to adopt
    /// onto, so the caller falls through to step 3. (Extracted per the #558
    /// round-3/round-4 function-length gate.)
    ///
    /// #558 W8/W9: the candidate query itself now lives in
    /// `find_live_by_name_candidate` — the SAME implementation
    /// `resolve_sync_apply_target` uses for its pre-transaction lock-target
    /// probe, so the two can never independently drift apart again.
    async fn try_adopt_by_name(
        txn: &sea_orm::DatabaseTransaction,
        library_id: &str,
        incoming: &SyncPresentation,
        peer_sync_ids: &HashSet<String>,
    ) -> anyhow::Result<Option<(SyncApplyOutcome, Option<presenter_core::PresentationId>)>> {
        let Some(existing) =
            Self::find_live_by_name_candidate(txn, library_id, &incoming.name).await?
        else {
            return Ok(None);
        };

        // #558 round-4 U2: adopt-by-name is single-shot per name. Two
        // independently-created LIVE same-name songs on the PEER
        // (different sync_ids, both listed in its manifest) used to
        // serially adopt onto this ONE local candidate within a single
        // cycle — the second peer entry processed found the first
        // entry's just-written row as its sole live candidate and
        // overwrote it, silently discarding the first twin. The orphaned
        // identity then refetched + re-adopted every following cycle
        // forever. If this candidate's OWN sync_id is itself known to the
        // peer, the peer is ALREADY tracking it as a separate identity —
        // adopting it here would orphan that peer entry instead. Fall
        // through to step 3 (returns `None`), which creates a brand-new
        // row for `incoming` instead; both twins then converge in the
        // same cycle.
        if peer_sync_ids.contains(&existing.sync_id) {
            info!(
                candidate_sync_id = %existing.sync_id,
                "sync skip adopt-by-name (candidate's sync_id is itself peer-known); \
                 creating a new row instead"
            );
            return Ok(None);
        }

        let local_updated: DateTime<Utc> = existing.updated_at.into();
        if !sync_should_apply(incoming.updated_at, false, Some(local_updated)) {
            // Local wins; the peer will adopt OUR sync_id when it pulls us.
            info!("sync skip (adopt-by-name, local newer)");
            return Ok(Some((SyncApplyOutcome::SkippedNotNewer, None)));
        }
        Self::write_synced_row(txn, &existing.id, library_id, incoming).await?;
        info!("sync adopted-by-name");
        let id = presenter_core::PresentationId::from_uuid(uuid::Uuid::parse_str(&existing.id)?);
        Ok(Some((SyncApplyOutcome::AdoptedByName, Some(id))))
    }

    /// Step 3 of `apply_sync_presentation`: the peer's `sync_id` matched no
    /// local row (and, for a live entry, no adopt-by-name candidate). For a
    /// TOMBSTONE still within PRUNE_HORIZON — #558 R2/S7 via
    /// `sync_should_apply`'s `None` branch — or any live entry, CREATE a
    /// brand-new row with the peer's identity + timestamps; an
    /// already-pruned tombstone (older than the horizon) is skipped, never
    /// resurrected as a fresh trashed row. Never touches any existing
    /// local row. (Extracted per the #558 round-3 function-length gate.)
    ///
    /// #558 round-4 U4: the horizon gate runs BEFORE the library is
    /// ensured/created — a skip must never leave behind a phantom,
    /// permanently-empty library for a library this instance otherwise has
    /// no reason to know about.
    async fn apply_unknown_sync_id(
        txn: &sea_orm::DatabaseTransaction,
        incoming: &SyncPresentation,
    ) -> anyhow::Result<(SyncApplyOutcome, Option<presenter_core::PresentationId>)> {
        if !sync_should_apply(incoming.updated_at, incoming.deleted_at.is_some(), None) {
            info!("sync skip (unknown tombstone past prune horizon)");
            return Ok((SyncApplyOutcome::SkippedNotNewer, None));
        }
        // #578: a brand-new TOMBSTONE (a fresh peer's recently-trashed song we
        // never held) must NOT spin up a fresh EMPTY LIVE library shell —
        // attach it to any existing same-named library (live or tombstoned),
        // or, if none exists, a TOMBSTONED library. A LIVE entry ensures a
        // live library as before.
        let library_id = if incoming.deleted_at.is_some() {
            Self::ensure_library_for_tombstone(txn, &incoming.library_name).await?
        } else {
            Self::ensure_library(txn, &incoming.library_name, incoming.updated_at).await?
        };
        let new_id = uuid::Uuid::new_v4().to_string();
        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(new_id.clone()),
            library_id: Set(library_id),
            name: Set(incoming.name.clone()),
            search_name: Set(fold_query(&incoming.name)),
            created_at: Set(Utc::now().into()),
            updated_at: Set(incoming.updated_at.into()),
            sync_id: Set(incoming.sync_id.clone()),
            deleted_at: Set(incoming.deleted_at.map(Into::into)),
        })
        .exec(txn)
        .await?;
        Self::replace_slides(txn, &new_id, incoming).await?;
        info!("sync created");
        let id = presenter_core::PresentationId::from_uuid(uuid::Uuid::parse_str(&new_id)?);
        Ok((SyncApplyOutcome::Created, Some(id)))
    }

    /// Update an existing local row IN PLACE (preserving its id + playlist refs):
    /// name, search_name, library, sync_id (adopt), deleted_at, and the PEER's
    /// updated_at (never now() — that is what prevents echo). Then replace slides.
    async fn write_synced_row<C: sea_orm::ConnectionTrait>(
        conn: &C,
        local_id: &str,
        library_id: &str,
        incoming: &SyncPresentation,
    ) -> anyhow::Result<()> {
        use presentation_entity::Column;
        let deleted = incoming
            .deleted_at
            .map(|d| Expr::value(d.to_rfc3339()))
            .unwrap_or_else(|| Expr::value(Option::<String>::None));
        presentation_entity::Entity::update_many()
            .col_expr(Column::Name, Expr::value(incoming.name.clone()))
            .col_expr(Column::SearchName, Expr::value(fold_query(&incoming.name)))
            .col_expr(Column::LibraryId, Expr::value(library_id))
            .col_expr(Column::SyncId, Expr::value(incoming.sync_id.clone()))
            .col_expr(
                Column::UpdatedAt,
                Expr::value(incoming.updated_at.to_rfc3339()),
            )
            .col_expr(Column::DeletedAt, deleted)
            .filter(Column::Id.eq(local_id))
            .exec(conn)
            .await?;
        Self::replace_slides(conn, local_id, incoming).await
    }

    /// Wholesale slide replacement carrying the peer's slide ids (global v4 uniqueness
    /// makes id collisions a non-issue).
    async fn replace_slides<C: sea_orm::ConnectionTrait>(
        conn: &C,
        presentation_id: &str,
        incoming: &SyncPresentation,
    ) -> anyhow::Result<()> {
        // #558 S9 + R4: capture the OLD stage-layout markers BEFORE the
        // wholesale slide replacement, together with the OLD slide's own
        // CONTENT and 0-based position — the new slides carry the PEER's
        // ids, so the old slide_id is about to stop existing.
        // `remap_markers` matches by CONTENT identity first (a pure
        // reorder keeps the marker on the right verse even though its
        // index moved) and falls back to position only when content
        // doesn't settle it (e.g. the marked slide's own text was edited);
        // a marker that resolves to neither is dropped, logged.
        //
        // R3: applying a peer's TOMBSTONE must mirror
        // `delete_presentation`'s local-delete behavior and CLEAR the
        // song's markers instead of carrying them across the trash
        // boundary — else a later restore flips the stage to a stale
        // layout the operator never re-applied.
        let (old_markers, old_content) = if incoming.deleted_at.is_some() {
            (Vec::new(), Vec::new())
        } else {
            Self::markers_with_content(conn, presentation_id).await?
        };

        slide_entity::Entity::delete_many()
            .filter(slide_entity::Column::PresentationId.eq(presentation_id))
            .exec(conn)
            .await?;
        slide_stage_layout::Entity::delete_many()
            .filter(slide_stage_layout::Column::PresentationId.eq(presentation_id))
            .exec(conn)
            .await?;

        // #558 W6: clamp EVERY incoming slide ONCE, up front — the SAME
        // clamped slides are both what gets INSERTED and what
        // `remap_markers` compares against for content identity. The old
        // code compared markers against the RAW, unclamped `incoming.slides`
        // while inserting the CLAMPED version, so an oversize marked slide
        // could never content-match what was actually stored (the clamped
        // text is shorter than the raw incoming text whenever a legacy
        // value gets truncated), silently losing precision to the position
        // fallback — or, on a reorder, reattaching to the WRONG slide.
        let clamped_slides: Vec<Slide> = incoming
            .slides
            .iter()
            .map(|slide| clamp_incoming_slide(slide, &incoming.sync_id))
            .collect();
        for (index, clamped) in clamped_slides.iter().enumerate() {
            let active = build_slide_active_model(clamped, presentation_id, index as i32);
            slide_entity::Entity::insert(active).exec(conn).await?;
        }

        if !old_markers.is_empty() {
            Self::remap_markers(
                conn,
                presentation_id,
                &clamped_slides,
                old_markers,
                &old_content,
            )
            .await?;
        }
        Ok(())
    }

    /// The presentation's CURRENT stage-layout markers, each paired with its
    /// OLD slide's 0-based position (matching `build_slide_active_model`'s
    /// `position` column) and full content, PLUS every OLD slide's content
    /// (marked or not) so `remap_markers` can tell whether a marked slide's
    /// content is unique among the old set (#558 S9 + R4).
    ///
    /// #558 round-3 T7: the OLD slide's content is read as RAW strings —
    /// never rebuilt through `SlideText::new`, which VALIDATES a 4000-char
    /// limit. A legacy row (or one that arrived over the wire before/
    /// without that validation ever running) can already be stored
    /// oversize; re-validating it here on every future sync apply would
    /// make that marked song's sync fail FOREVER. This function only ever
    /// COMPARES content for matching, never re-inserts it as a
    /// `SlideText`, so raw strings are all it needs.
    async fn markers_with_content<C: sea_orm::ConnectionTrait>(
        conn: &C,
        presentation_id: &str,
    ) -> anyhow::Result<(Vec<OldMarker>, Vec<RawSlideContent>)> {
        let old_markers = slide_stage_layout::Entity::find()
            .filter(slide_stage_layout::Column::PresentationId.eq(presentation_id))
            .all(conn)
            .await?;
        if old_markers.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        // #558 R8: project only the columns needed to rebuild each old
        // slide's position + content (never the full row — drops the
        // `*_search` index columns and `created_at`).
        let old_slides: Vec<(String, i32, String, String, String, Option<String>)> =
            slide_entity::Entity::find()
                .select_only()
                .column(slide_entity::Column::Id)
                .column(slide_entity::Column::Position)
                .column(slide_entity::Column::WorshipMain)
                .column(slide_entity::Column::WorshipTranslate)
                .column(slide_entity::Column::WorshipStage)
                .column(slide_entity::Column::WorshipGroup)
                .filter(slide_entity::Column::PresentationId.eq(presentation_id))
                .order_by_asc(slide_entity::Column::Position)
                .into_tuple()
                .all(conn)
                .await?;
        let mut by_slide_id: HashMap<String, (i32, RawSlideContent)> =
            HashMap::with_capacity(old_slides.len());
        let mut all_content = Vec::with_capacity(old_slides.len());
        for (id, position, main, translate, stage, group) in old_slides {
            let content = RawSlideContent {
                main,
                translation: translate,
                stage,
                group,
            };
            all_content.push(content.clone());
            by_slide_id.insert(id, (position, content));
        }
        let markers = old_markers
            .into_iter()
            .filter_map(|marker| {
                by_slide_id
                    .get(marker.slide_id.as_str())
                    .map(|(position, content)| OldMarker {
                        position: *position,
                        content: content.clone(),
                        layout_code: marker.layout_code,
                    })
            })
            .collect();
        Ok((markers, all_content))
    }

    /// Re-attach each OLD marker to a NEW slide: match by CONTENT identity
    /// first (when the old slide's content is UNIQUE among the old slide
    /// set); fall back to POSITION when the old content is ambiguous
    /// (shared by 2+ old slides) or has no content match at all in the new
    /// list (e.g. the marked slide's own text was edited by the peer). A
    /// marker that resolves to neither is dropped, logged — never silently
    /// (#558 S9, hardened by R4).
    ///
    /// Two PASSES, deliberately, so the outcome never depends on the
    /// arbitrary order `old_markers` came back from the DB in (#558
    /// round-3 T3): pass 1 resolves every eligible marker's CONTENT match
    /// first — a unique-content marker always gets its rightful slide
    /// before ANY position fallback runs. Pass 2 then runs the POSITION
    /// fallback for whatever is left, but ONLY into a slot pass 1 (or an
    /// earlier pass-2 marker) hasn't already claimed — claimed-or-out-of-
    /// range is dropped, never inserted. Interleaving the two (the old,
    /// single-pass code) let iteration order decide which marker "won" a
    /// contested slide, and a position fallback could target the exact
    /// slide_id an earlier content match had just claimed — a genuine PK
    /// violation on `slide_stage_layouts` (its primary key is `slide_id`
    /// alone).
    async fn remap_markers<C: sea_orm::ConnectionTrait>(
        conn: &C,
        presentation_id: &str,
        new_slides: &[presenter_core::Slide],
        old_markers: Vec<OldMarker>,
        old_content: &[RawSlideContent],
    ) -> anyhow::Result<()> {
        let mut claimed = vec![false; new_slides.len()];
        let mut targets: Vec<Option<(usize, presenter_core::SlideId)>> =
            vec![None; old_markers.len()];

        // #558 round-4 U6: precompute each NEW slide's raw content ONCE —
        // pass 1 used to call `raw_content_of` inside the inner `.find()`
        // closure, recomputing it for the same candidate slides on every
        // marker's scan instead of once per slide up front.
        let new_content: Vec<RawSlideContent> = new_slides
            .iter()
            .map(|slide| raw_content_of(&slide.content))
            .collect();

        // Pass 1: CONTENT matches, order-independent. Compared as RAW
        // strings (#558 round-3 T7) — never re-validated.
        for (i, marker) in old_markers.iter().enumerate() {
            let content_unique_among_old =
                old_content.iter().filter(|c| **c == marker.content).count() == 1;
            if !content_unique_among_old {
                continue;
            }
            if let Some(idx) = new_content
                .iter()
                .enumerate()
                .find(|(idx, content)| !claimed[*idx] && **content == marker.content)
                .map(|(idx, _)| idx)
            {
                claimed[idx] = true;
                targets[i] = Some((idx, new_slides[idx].id));
            }
        }

        // Pass 2: POSITION fallback for whatever pass 1 left unresolved —
        // only into a slot nothing has claimed yet, and only in range.
        // #558 round-4 U9: `claimed.len() == new_slides.len()` always (set
        // above), so a single bounds check against `claimed` is enough —
        // the old code ALSO checked `new_slides.get(idx)`, a second check
        // that could never itself be the deciding factor.
        for (i, marker) in old_markers.iter().enumerate() {
            if targets[i].is_some() {
                continue;
            }
            let idx = marker.position as usize;
            if idx < claimed.len() && !claimed[idx] {
                claimed[idx] = true;
                targets[i] = Some((idx, new_slides[idx].id));
            }
        }

        let mut remapped = 0usize;
        let mut dropped = 0usize;
        for (marker, target) in old_markers.into_iter().zip(targets) {
            match target {
                Some((_idx, slide_id)) => {
                    slide_stage_layout::Entity::insert(slide_stage_layout::ActiveModel {
                        slide_id: Set(slide_id.to_string()),
                        presentation_id: Set(presentation_id.to_string()),
                        layout_code: Set(marker.layout_code),
                    })
                    .exec(conn)
                    .await?;
                    remapped += 1;
                }
                None => dropped += 1,
            }
        }
        if dropped > 0 {
            warn!(
                presentation_id,
                remapped,
                dropped,
                "sync apply dropped stage-layout markers with no content or position match"
            );
        } else {
            info!(
                presentation_id,
                remapped,
                "sync apply remapped stage-layout markers by content identity (position fallback)"
            );
        }
        Ok(())
    }
}

/// One OLD stage-layout marker with enough context to remap it by CONTENT
/// identity first (#558 R4): the marker's own layout code, the OLD slide's
/// 0-based position (a secondary/fallback signal — #558 S9), and the OLD
/// slide's full content (main/translation/stage/group) as RAW strings
/// (#558 round-3 T7).
struct OldMarker {
    position: i32,
    content: RawSlideContent,
    layout_code: String,
}

/// A slide's content as RAW strings — deliberately never re-validated
/// through `SlideText::new` (#558 round-3 T7). `markers_with_content` only
/// ever COMPARES an OLD stored slide's content for matching purposes; it
/// never re-inserts it as a `SlideText`. Re-validating a STORED value on
/// every future sync apply would make a legacy/wire-path row that already
/// exceeds the 4000-char `SlideText` limit fail sync forever, for a song
/// that otherwise works fine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawSlideContent {
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
}

/// #558 V1: the APPLY boundary is the last stop before storage — clamp any
/// oversize incoming slide field to the 4000-char cap the LOCAL read path
/// (`to_domain_slide`, via `fetch_presentation_detail`) enforces, instead of
/// blindly replicating a pathological legacy/wire-path row from the peer.
/// A `Slide` deserialized off the wire never validates (`SlideText`'s
/// `Deserialize` is a plain derive), so an oversize field can arrive here
/// completely unchecked. Truncating (never rejecting) keeps the sync
/// converging — the alternative, rejecting the whole song, would leave it
/// unsynced forever; the alternative of storing it raw is the U1 bug this
/// closes (it broke the RECEIVING site's own normal read path for that
/// song too, overwriting its good copy with the peer's bad one). Every
/// truncated field is logged WARN naming the sync_id + field so the loss is
/// visible, never silent.
fn clamp_incoming_slide(slide: &Slide, sync_id: &str) -> Slide {
    let (main, main_truncated) = SlideText::from_stored_or_truncate(slide.content.main.value());
    let (translation, translation_truncated) =
        SlideText::from_stored_or_truncate(slide.content.translation.value());
    let (stage, stage_truncated) = SlideText::from_stored_or_truncate(slide.content.stage.value());

    for (truncated, field) in [
        (main_truncated, "main"),
        (translation_truncated, "translation"),
        (stage_truncated, "stage"),
    ] {
        if truncated {
            warn!(
                sync_id,
                slide_id = %slide.id,
                field,
                "sync apply truncated an oversize incoming slide field to the 4000-char cap"
            );
        }
    }

    let content = SlideContent::new(main, translation, stage, slide.content.group.clone());
    Slide::new(slide.order, content)
        .with_id(slide.id)
        .with_metadata(slide.metadata.clone())
}

/// Extract a NEW (already-validated) peer slide's content as raw strings,
/// for comparison against an `OldMarker`'s `RawSlideContent` — never
/// re-validates either side.
fn raw_content_of(content: &presenter_core::SlideContent) -> RawSlideContent {
    RawSlideContent {
        main: content.main.value().to_string(),
        translation: content.translation.value().to_string(),
        stage: content.stage.value().to_string(),
        group: content.group.as_ref().map(|g| g.name().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{sync_should_apply, PRUNE_HORIZON};
    use chrono::{Duration, Utc};

    #[test]
    fn lww_matrix() {
        let now = Utc::now();
        assert!(
            sync_should_apply(now, false, None),
            "unknown locally, peer not deleted → apply"
        );
        assert!(
            sync_should_apply(now, true, None),
            "unknown locally AND peer deleted, but the tombstone is RECENT → apply \
             (a fresh peer's first sync must not skip trash it never held; #558 R2)"
        );
        assert!(
            !sync_should_apply(now - PRUNE_HORIZON - Duration::days(1), true, None),
            "unknown locally AND peer deleted, tombstone OLDER than the prune horizon → \
             never resurrect an already-pruned row (S7)"
        );
        assert!(
            sync_should_apply(now, false, Some(now - Duration::seconds(1))),
            "peer newer → apply"
        );
        assert!(
            !sync_should_apply(now, false, Some(now + Duration::seconds(1))),
            "peer older → skip"
        );
        assert!(
            !sync_should_apply(now, false, Some(now)),
            "equal → skip (no echo)"
        );
        assert!(
            sync_should_apply(now, true, Some(now - Duration::seconds(1))),
            "peer newer delete of a row we STILL HOLD applies normally (trash propagation)"
        );
    }

    #[test]
    fn unknown_locally_distinguishes_a_fresh_tombstone_from_an_already_pruned_one() {
        // R2 regression: `None => !peer_deleted` treated EVERY tombstone the
        // same when we hold no local row at all -- so a fresh/re-provisioned
        // peer's first sync PERMANENTLY skipped anything already trashed on
        // the other side, and the two instances' trash contents diverged
        // forever. Fix: a tombstone YOUNGER than PRUNE_HORIZON, for a row
        // we've never held, must be applied (created locally as trashed, so
        // trash contents converge); only a tombstone OLDER than the horizon
        // is skipped (that's the genuinely-pruned case S7 protects
        // against).
        let now = Utc::now();
        assert!(
            sync_should_apply(now - Duration::days(1), true, None),
            "a RECENT tombstone for a row we've never held must be applied (created as trashed)"
        );
        assert!(
            sync_should_apply(now, true, None),
            "a tombstone from right now, for a row we've never held, must be applied"
        );
        assert!(
            !sync_should_apply(now - PRUNE_HORIZON - Duration::days(1), true, None),
            "a tombstone OLDER than PRUNE_HORIZON must be skipped, never resurrected"
        );
    }
}
