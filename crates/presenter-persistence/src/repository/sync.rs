//! #555 song-sync read layer: the manifest (identity + timestamps for every song,
//! trashed included) and the full synced content for one song, plus the trash
//! list/restore/prune operations.
use super::util::{to_domain_slide_wire, RepositoryError};
use super::Repository;
use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use chrono::{DateTime, Utc};
use presenter_core::Slide;
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use tracing::instrument;

/// One manifest row — identity + timestamps, ALL songs including trashed.
#[derive(Debug, Clone)]
pub struct SyncManifestRow {
    pub sync_id: String,
    pub library_name: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Full synced content for one song.
#[derive(Debug, Clone)]
pub struct SyncPresentation {
    pub sync_id: String,
    pub library_name: String,
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
            out.push(SyncManifestRow {
                sync_id: p.sync_id,
                library_name: lib.map(|l| l.name).unwrap_or_default(),
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
        let library_name = library::Entity::find_by_id(p.library_id.clone())
            .one(&self.db)
            .await?
            .map(|l| l.name)
            .unwrap_or_default();
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
        let now = Utc::now().to_rfc3339();
        let result = presentation_entity::Entity::update_many()
            .col_expr(Column::DeletedAt, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(presentation_id.to_string()))
            .filter(Column::DeletedAt.is_not_null())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow::anyhow!("no trashed presentation to restore"));
        }
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
