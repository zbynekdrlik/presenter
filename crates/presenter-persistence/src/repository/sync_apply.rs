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
use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use crate::SyncPresentation;
use chrono::{DateTime, Utc};
use presenter_core::search::fold_query;
use sea_orm::{
    sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use tracing::{info, instrument};

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

/// LWW: apply the peer row iff it is strictly newer than what we hold (or unknown).
/// `local` is `None` when we have no matching song at all. `peer_deleted` is
/// whether the PEER's row is currently a tombstone.
///
/// #558 S7: when `local` is `None` AND the peer entry is a tombstone, this
/// returns `false` — never re-create a trashed row we don't hold. Without
/// this, a row we've already pruned (our own 30-day schedule fired) would be
/// resurrected by a peer whose manifest still lists the tombstone (its own
/// prune hasn't fired yet). Each side prunes on its own schedule and nothing
/// resurrects. A peer delete of a row we DO still hold (`local: Some(_)`)
/// is unaffected — that's normal trash propagation, gated on `peer > local`
/// like any other edit.
pub fn sync_should_apply(
    peer: DateTime<Utc>,
    peer_deleted: bool,
    local: Option<DateTime<Utc>>,
) -> bool {
    match local {
        None => !peer_deleted,
        Some(local) => peer > local,
    }
}

impl Repository {
    #[instrument(skip_all, fields(sync_id = %incoming.sync_id, name = %incoming.name))]
    pub async fn apply_sync_presentation(
        &self,
        incoming: &SyncPresentation,
    ) -> anyhow::Result<SyncApplyOutcome> {
        let txn = self.db.begin().await?;

        // Ensure a library with the peer's library name exists; reuse or create.
        let lib = library::Entity::find()
            .filter(library::Column::Name.eq(incoming.library_name.clone()))
            .one(&txn)
            .await?;
        let library_id = match lib {
            Some(l) => l.id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                library::Entity::insert(library::ActiveModel {
                    id: Set(id.clone()),
                    name: Set(incoming.library_name.clone()),
                    search_name: Set(fold_query(&incoming.library_name)),
                    created_at: Set(Utc::now().into()),
                })
                .exec(&txn)
                .await?;
                id
            }
        };

        // 1. Match by sync_id.
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
                return Ok(SyncApplyOutcome::SkippedNotNewer);
            }
            Self::write_synced_row(&txn, &existing.id, &library_id, incoming).await?;
            txn.commit().await?;
            info!("sync updated");
            return Ok(SyncApplyOutcome::Updated);
        }

        // 2. Adopt-by-name: same name in the same-named library, unknown sync_id,
        // among LIVE (non-trashed) candidates only (#558 S4). A trashed local row
        // must never be silently resurrected via adoption, and an AMBIGUOUS match
        // (2+ live candidates sharing the same name) must never guess which one
        // to adopt — it falls through to create instead. `.one()` with no
        // ORDER BY picks an arbitrary row, so the candidate set is fetched in
        // full, filtered to live rows, ordered deterministically, and adoption
        // proceeds ONLY when exactly one live candidate remains.
        let mut live_by_name_candidates = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.eq(library_id.clone()))
            .filter(presentation_entity::Column::Name.eq(incoming.name.clone()))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .order_by_asc(presentation_entity::Column::CreatedAt)
            .order_by_asc(presentation_entity::Column::Id)
            .all(&txn)
            .await?;
        let by_name =
            (live_by_name_candidates.len() == 1).then(|| live_by_name_candidates.remove(0));
        if let Some(existing) = by_name {
            let local_updated: DateTime<Utc> = existing.updated_at.into();
            if !sync_should_apply(
                incoming.updated_at,
                incoming.deleted_at.is_some(),
                Some(local_updated),
            ) {
                // Local wins; the peer will adopt OUR sync_id when it pulls us.
                txn.commit().await?;
                info!("sync skip (adopt-by-name, local newer)");
                return Ok(SyncApplyOutcome::SkippedNotNewer);
            }
            Self::write_synced_row(&txn, &existing.id, &library_id, incoming).await?;
            txn.commit().await?;
            info!("sync adopted-by-name");
            return Ok(SyncApplyOutcome::AdoptedByName);
        }

        // 3. Unknown → create with the peer's identity + timestamps.
        let new_id = uuid::Uuid::new_v4().to_string();
        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(new_id.clone()),
            library_id: Set(library_id.clone()),
            name: Set(incoming.name.clone()),
            search_name: Set(fold_query(&incoming.name)),
            created_at: Set(Utc::now().into()),
            updated_at: Set(incoming.updated_at.into()),
            sync_id: Set(incoming.sync_id.clone()),
            deleted_at: Set(incoming.deleted_at.map(Into::into)),
        })
        .exec(&txn)
        .await?;
        Self::replace_slides(&txn, &new_id, incoming).await?;
        txn.commit().await?;
        info!("sync created");
        Ok(SyncApplyOutcome::Created)
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
        slide_entity::Entity::delete_many()
            .filter(slide_entity::Column::PresentationId.eq(presentation_id))
            .exec(conn)
            .await?;
        for (index, slide) in incoming.slides.iter().enumerate() {
            let active = build_slide_active_model(slide, presentation_id, index as i32);
            slide_entity::Entity::insert(active).exec(conn).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::sync_should_apply;
    use chrono::{Duration, Utc};

    #[test]
    fn lww_matrix() {
        let now = Utc::now();
        assert!(
            sync_should_apply(now, false, None),
            "unknown locally, peer not deleted → apply"
        );
        assert!(
            !sync_should_apply(now, true, None),
            "unknown locally AND peer deleted → never resurrect a pruned row (S7)"
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
}
