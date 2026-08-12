//! #555 song-sync read layer: the manifest (identity + timestamps for every song,
//! trashed included) and the full synced content for one song, plus the trash
//! list/restore/prune operations.
use super::util::{to_domain_slide_wire, RepositoryError};
use super::Repository;
use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use chrono::{DateTime, Utc};
use presenter_core::Slide;
use sea_orm::{
    sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait,
};
use tracing::instrument;

/// One manifest row — identity + timestamps, ALL songs including trashed.
#[derive(Debug, Clone)]
pub struct SyncManifestRow {
    pub sync_id: String,
    pub library_name: String,
    /// #647: the parent library's own `sync_id` — `None` only if the FK'd
    /// library row is somehow missing (never expected in practice; the
    /// schema enforces it). Lets the apply-side join by stable identity
    /// instead of the current name string.
    pub library_sync_id: Option<String>,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Full synced content for one song.
#[derive(Debug, Clone)]
pub struct SyncPresentation {
    pub sync_id: String,
    pub library_name: String,
    /// #647 — see `SyncManifestRow::library_sync_id`'s doc comment.
    pub library_sync_id: Option<String>,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub slides: Vec<Slide>,
}

/// A soft-deleted song, for the trash UI.
#[derive(Debug, Clone)]
pub struct TrashedPresentation {
    pub id: String,
    pub sync_id: String,
    pub name: String,
    pub library_name: String,
    pub deleted_at: DateTime<Utc>,
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn list_sync_manifest(&self) -> anyhow::Result<Vec<SyncManifestRow>> {
        let rows = presentation_entity::Entity::find()
            .find_also_related(library::Entity)
            .all(&self.db)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for (p, lib) in rows {
            let (library_name, library_sync_id) = match lib {
                Some(l) => (l.name, Some(l.sync_id)),
                None => (String::new(), None),
            };
            out.push(SyncManifestRow {
                sync_id: p.sync_id,
                library_name,
                library_sync_id,
                name: p.name,
                updated_at: p.updated_at.into(),
                deleted_at: p.deleted_at.map(Into::into),
            });
        }
        Ok(out)
    }

    /// Resolve the LOCAL presentation id currently holding `sync_id`, if any
    /// (#558 V2). The sync-apply caller uses this to acquire the SAME
    /// per-presentation lock a concurrent snapshot-replace edit op takes,
    /// BEFORE this song's apply transaction begins. A pure lookup — never
    /// creates anything.
    #[instrument(skip_all)]
    pub async fn find_presentation_id_by_sync_id(
        &self,
        sync_id: &str,
    ) -> anyhow::Result<Option<presenter_core::PresentationId>> {
        let row = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::SyncId.eq(sync_id))
            .one(&self.db)
            .await?;
        row.map(|row| {
            Ok(presenter_core::PresentationId::from_uuid(
                uuid::Uuid::parse_str(&row.id)?,
            ))
        })
        .transpose()
    }

    #[instrument(skip_all)]
    pub async fn fetch_sync_presentation(
        &self,
        sync_id: &str,
    ) -> anyhow::Result<Option<SyncPresentation>> {
        let Some(p) = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::SyncId.eq(sync_id))
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let (library_name, library_sync_id) =
            match library::Entity::find_by_id(p.library_id.clone())
                .one(&self.db)
                .await?
            {
                Some(l) => (l.name, Some(l.sync_id)),
                None => (String::new(), None),
            };
        let slides = slide_entity::Entity::find()
            .filter(slide_entity::Column::PresentationId.eq(p.id.clone()))
            .order_by_asc(slide_entity::Column::Position)
            .all(&self.db)
            .await?
            .into_iter()
            .map(to_domain_slide_wire)
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        Ok(Some(SyncPresentation {
            sync_id: p.sync_id,
            library_name,
            library_sync_id,
            name: p.name,
            updated_at: p.updated_at.into(),
            deleted_at: p.deleted_at.map(Into::into),
            slides,
        }))
    }

    #[instrument(skip_all)]
    pub async fn list_trashed_presentations(&self) -> anyhow::Result<Vec<TrashedPresentation>> {
        let rows = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::DeletedAt.is_not_null())
            .order_by_desc(presentation_entity::Column::DeletedAt)
            .find_also_related(library::Entity)
            .all(&self.db)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for (p, lib) in rows {
            if let Some(deleted) = p.deleted_at {
                out.push(TrashedPresentation {
                    id: p.id,
                    sync_id: p.sync_id,
                    name: p.name,
                    library_name: lib.map(|l| l.name).unwrap_or_default(),
                    deleted_at: deleted.into(),
                });
            }
        }
        Ok(out)
    }

    #[instrument(skip_all)]
    pub async fn restore_presentation(
        &self,
        presentation_id: presenter_core::PresentationId,
    ) -> anyhow::Result<()> {
        use presentation_entity::Column;
        // #646: the library probe + the update now share ONE transaction
        // (previously two independent, unlocked reads + a write) so a
        // concurrent library prune/tombstone can never land between the
        // check and the write.
        let txn = self.db.begin().await?;
        let existing = presentation_entity::Entity::find()
            .filter(Column::Id.eq(presentation_id.to_string()))
            .filter(Column::DeletedAt.is_not_null())
            .one(&txn)
            .await?
            .ok_or(RepositoryError::NotFound(
                "no trashed presentation to restore",
            ))?;

        // #636/#646: a restore that leaves the song under a STILL-tombstoned
        // library accomplishes nothing durable -- the library's own
        // deleted_at hides it from fetch_libraries, so the "restored" song
        // stays unreachable, and it is now LIVE, so the next
        // prune_deleted_libraries CASCADE hard-deletes it. A library row
        // that is MISSING ENTIRELY is a DIFFERENT situation (404, not 409)
        // -- see `classify_restore_library`.
        let library = library::Entity::find_by_id(existing.library_id.clone())
            .one(&txn)
            .await?;
        classify_restore_library(library.as_ref())?;

        let now = Utc::now().to_rfc3339();
        let result = presentation_entity::Entity::update_many()
            .col_expr(Column::DeletedAt, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(presentation_id.to_string()))
            .filter(Column::DeletedAt.is_not_null())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            // #587: typed refusal (#584 pattern) — the router downcasts to
            // `RepositoryError` and maps `NotFound` to 404 instead of a bare 500.
            return Err(RepositoryError::NotFound("no trashed presentation to restore").into());
        }
        txn.commit().await?;
        Ok(())
    }

    /// Hard-delete songs trashed longer than `retain`. FK cascade removes slides;
    /// stage-layout markers were cleared at soft-delete time. Returns rows removed.
    #[instrument(skip_all)]
    pub async fn prune_deleted_presentations(
        &self,
        retain: chrono::Duration,
    ) -> anyhow::Result<u64> {
        let cutoff = (Utc::now() - retain).to_rfc3339();
        let res = presentation_entity::Entity::delete_many()
            .filter(presentation_entity::Column::DeletedAt.is_not_null())
            .filter(presentation_entity::Column::DeletedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(res.rows_affected)
    }
}

/// #646: classify what `restore_presentation` must do based on the parent
/// library's state. `None` (the row is entirely missing) is `NotFound`
/// (404) — a DIFFERENT situation from `Some(tombstoned)` (still trashed,
/// 409) that the old `is_none_or` folded into the same `Conflict`. A pure
/// function so both branches are directly unit-testable without needing to
/// defeat the schema's own FK constraint (which makes a genuinely dangling
/// `library_id` unreachable through any normal write path) just to exercise
/// the "missing" branch.
fn classify_restore_library(library: Option<&library::Model>) -> Result<(), RepositoryError> {
    match library {
        None => Err(RepositoryError::NotFound(
            "the presentation's library no longer exists",
        )),
        Some(lib) if lib.deleted_at.is_some() => Err(RepositoryError::Conflict(
            "the presentation's library is still trashed — restore the library first",
        )),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::classify_restore_library;
    use crate::entities::library;
    use crate::RepositoryError;
    use chrono::Utc;

    fn library_model(deleted: bool) -> library::Model {
        let now = Utc::now();
        library::Model {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Songs".to_string(),
            search_name: "songs".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            sync_id: uuid::Uuid::new_v4().to_string(),
            deleted_at: deleted.then_some(now.into()),
        }
    }

    #[test]
    fn missing_library_row_is_not_found() {
        assert!(matches!(
            classify_restore_library(None),
            Err(RepositoryError::NotFound(_))
        ));
    }

    #[test]
    fn tombstoned_library_row_is_conflict() {
        let tombstoned = library_model(true);
        assert!(matches!(
            classify_restore_library(Some(&tombstoned)),
            Err(RepositoryError::Conflict(_))
        ));
    }

    #[test]
    fn live_library_row_is_ok() {
        let live = library_model(false);
        assert!(classify_restore_library(Some(&live)).is_ok());
    }
}
