//! #578 library sync: the library manifest read + the LWW apply of one peer
//! library row + the tombstone prune. A library has no "content" to fetch
//! (just its name), so — unlike a presentation — the manifest row carries
//! everything the apply needs. Mirrors `apply_sync_presentation`'s
//! sync_id-match → adopt-by-name → create-or-skip shape under the SAME
//! `sync_should_apply` LWW gate, so library deletes + renames converge across
//! the PP<->SNV pair and a deleted library never resurrects.
use super::sync_apply::{sync_should_apply, SyncApplyOutcome};
use super::Repository;
use crate::entities::library;
use chrono::{DateTime, Utc};
use presenter_core::search::fold_query;
use sea_orm::{
    sea_query::Expr, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use tracing::{info, instrument};

/// One library manifest row — identity + name + timestamps, ALL libraries
/// including tombstoned (mirrors `SyncManifestRow` for presentations, so a
/// tombstone propagates instead of the library silently vanishing from the
/// wire).
#[derive(Debug, Clone)]
pub struct SyncLibraryManifestRow {
    pub sync_id: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn list_library_sync_manifest(&self) -> anyhow::Result<Vec<SyncLibraryManifestRow>> {
        let rows = library::Entity::find().all(&self.db).await?;
        Ok(rows
            .into_iter()
            .map(|l| SyncLibraryManifestRow {
                sync_id: l.sync_id,
                name: l.name,
                updated_at: l.updated_at.into(),
                deleted_at: l.deleted_at.map(Into::into),
            })
            .collect())
    }

    /// Apply one peer library row under LWW (#578). Returns the outcome so the
    /// caller can count applied rows and evict caches when something changed.
    #[instrument(skip_all, fields(sync_id = %incoming.sync_id, name = %incoming.name))]
    pub async fn apply_sync_library(
        &self,
        incoming: &SyncLibraryManifestRow,
    ) -> anyhow::Result<SyncApplyOutcome> {
        let txn = self.db.begin().await?;

        // 1. Match by sync_id — the authoritative identity, independent of name
        //    (so a peer rename applies in place, preserving the local id and
        //    every presentation.library_id FK pointing at it).
        if let Some(existing) = library::Entity::find()
            .filter(library::Column::SyncId.eq(incoming.sync_id.clone()))
            .one(&txn)
            .await?
        {
            let local: DateTime<Utc> = existing.updated_at.into();
            if !sync_should_apply(
                incoming.updated_at,
                incoming.deleted_at.is_some(),
                Some(local),
            ) {
                txn.commit().await?;
                return Ok(SyncApplyOutcome::SkippedNotNewer);
            }
            // #636: a rename (this write leaves the row LIVE) can target a
            // name a DIFFERENT live local library already owns —
            // `idx_libraries_name_live_unique` would otherwise reject the
            // UPDATE outright, and the caller (`reconcile_libraries`) only
            // warn!ed and dropped it, forever, every cycle. A tombstone
            // write is never at risk (the partial index only guards LIVE
            // rows), so it always writes `incoming.name` verbatim.
            let write_name = if incoming.deleted_at.is_none() {
                Self::resolve_conflict_free_name(&txn, &incoming.name, &existing.id).await?
            } else {
                incoming.name.clone()
            };
            Self::write_library_row(&txn, &existing.id, incoming, &write_name).await?;
            txn.commit().await?;
            info!("sync library updated");
            return Ok(SyncApplyOutcome::Updated);
        }

        // 2. Adopt-by-name: a LIVE local library with an unknown sync_id but the
        //    same name — two peers that independently created the library (each
        //    with its own backfilled random sync_id) converge here on first
        //    sync. ONLY for a LIVE incoming: a peer TOMBSTONE with an unknown
        //    sync_id must never reach across and trash a live same-named local
        //    library (mirrors the presentation adopt-by-name Decision B). A
        //    tombstone falls through to step 3, which creates its own separate
        //    tombstoned row instead.
        if incoming.deleted_at.is_none() {
            if let Some(existing) = Self::find_live_library_by_name(&txn, &incoming.name).await? {
                let local: DateTime<Utc> = existing.updated_at.into();
                if !sync_should_apply(incoming.updated_at, false, Some(local)) {
                    // Local wins; the peer will adopt OUR sync_id when it pulls us.
                    txn.commit().await?;
                    return Ok(SyncApplyOutcome::SkippedNotNewer);
                }
                // `existing` was found BY this exact name (`find_live_library_by_name`),
                // so writing `incoming.name` verbatim can never newly collide.
                Self::write_library_row(&txn, &existing.id, incoming, &incoming.name).await?;
                txn.commit().await?;
                info!("sync library adopted-by-name");
                return Ok(SyncApplyOutcome::AdoptedByName);
            }
        }

        // 3. Unknown sync_id → create-or-skip, gated on the SAME prune-horizon
        //    tombstone-age rule as presentations (`sync_should_apply`'s None
        //    branch): a genuinely-pruned tombstone (older than the horizon) is
        //    never resurrected as a fresh trashed row; a recent tombstone or
        //    any live entry is created with the peer's identity + timestamps.
        if !sync_should_apply(incoming.updated_at, incoming.deleted_at.is_some(), None) {
            txn.commit().await?;
            return Ok(SyncApplyOutcome::SkippedNotNewer);
        }
        library::Entity::insert(library::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            name: Set(incoming.name.clone()),
            search_name: Set(fold_query(&incoming.name)),
            created_at: Set(Utc::now().into()),
            updated_at: Set(incoming.updated_at.into()),
            sync_id: Set(incoming.sync_id.clone()),
            deleted_at: Set(incoming.deleted_at.map(Into::into)),
        })
        .exec(&txn)
        .await?;
        txn.commit().await?;
        info!("sync library created");
        Ok(SyncApplyOutcome::Created)
    }

    /// The single LIVE (non-tombstoned) local library with this name, if any —
    /// the partial name-unique index guarantees at most one, so `.one()` is
    /// unambiguous here (unlike presentations, which can have same-name twins).
    async fn find_live_library_by_name(
        txn: &DatabaseTransaction,
        name: &str,
    ) -> anyhow::Result<Option<library::Model>> {
        Ok(library::Entity::find()
            .filter(library::Column::Name.eq(name.to_string()))
            .filter(library::Column::DeletedAt.is_null())
            .one(txn)
            .await?)
    }

    /// Update a local library row IN PLACE (preserving its id — and thus every
    /// `presentation.library_id` FK pointing at it): name, search_name, sync_id
    /// (adopt), deleted_at, and the PEER's `updated_at` (never `now()` — an
    /// applied change is not a new local edit, which is what prevents an
    /// echo/ping-pong loop).
    ///
    /// `write_name` (#636) is the name actually WRITTEN to `Name`/`SearchName`
    /// — usually `incoming.name` verbatim, but the rename call site passes an
    /// already-disambiguated name (`resolve_conflict_free_name`) when
    /// `incoming.name` collides with a DIFFERENT live library.
    async fn write_library_row(
        txn: &DatabaseTransaction,
        local_id: &str,
        incoming: &SyncLibraryManifestRow,
        write_name: &str,
    ) -> anyhow::Result<()> {
        use library::Column;
        let deleted = incoming
            .deleted_at
            .map(|d| Expr::value(d.to_rfc3339()))
            .unwrap_or_else(|| Expr::value(Option::<String>::None));
        library::Entity::update_many()
            .col_expr(Column::Name, Expr::value(write_name.to_string()))
            .col_expr(Column::SearchName, Expr::value(fold_query(write_name)))
            .col_expr(Column::SyncId, Expr::value(incoming.sync_id.clone()))
            .col_expr(
                Column::UpdatedAt,
                Expr::value(incoming.updated_at.to_rfc3339()),
            )
            .col_expr(Column::DeletedAt, deleted)
            .filter(Column::Id.eq(local_id))
            .exec(txn)
            .await?;
        Ok(())
    }

    /// Resolve a name that is safe to write LIVE for `exclude_id` — i.e. one
    /// no OTHER live library already owns (#636). Returns `desired` unchanged
    /// when it's free; otherwise appends `" (2)"`, `" (3)"`, … until a free
    /// name is found. This is what lets a peer rename that collides with a
    /// DIFFERENT, already-known local identity always SUCCEED (both
    /// libraries survive, live, under distinct names) instead of throwing a
    /// `idx_libraries_name_live_unique` violation that used to be caught and
    /// silently dropped forever by `reconcile_libraries`.
    async fn resolve_conflict_free_name(
        txn: &DatabaseTransaction,
        desired: &str,
        exclude_id: &str,
    ) -> anyhow::Result<String> {
        use sea_orm::QuerySelect;
        let taken: std::collections::HashSet<String> = library::Entity::find()
            .filter(library::Column::DeletedAt.is_null())
            .filter(library::Column::Id.ne(exclude_id.to_string()))
            .select_only()
            .column(library::Column::Name)
            .into_tuple()
            .all(txn)
            .await?
            .into_iter()
            .collect();
        if !taken.contains(desired) {
            return Ok(desired.to_string());
        }
        let mut suffix = 2u32;
        loop {
            let candidate = format!("{desired} ({suffix})");
            if !taken.contains(&candidate) {
                return Ok(candidate);
            }
            suffix += 1;
        }
    }

    /// Hard-delete libraries tombstoned longer than `retain`. The FK
    /// ON DELETE CASCADE removes any presentations still under them (all
    /// already tombstoned) + their slides. Returns rows removed. Mirrors
    /// `prune_deleted_presentations`, driven by the SAME `PRUNE_HORIZON`.
    #[instrument(skip_all)]
    pub async fn prune_deleted_libraries(&self, retain: chrono::Duration) -> anyhow::Result<u64> {
        let cutoff = (Utc::now() - retain).to_rfc3339();
        let res = library::Entity::delete_many()
            .filter(library::Column::DeletedAt.is_not_null())
            .filter(library::Column::DeletedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(res.rows_affected)
    }
}
