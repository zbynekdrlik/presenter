//! Stream-graphics asset records — sha256-addressed uploaded-image metadata
//! (#705, epic #718 §5). One-file-per-domain sibling of `stream.rs`. The bytes
//! live on disk (`<workdir>/stream-assets/<sha256>.<ext>`, arch decision #4);
//! this is only the `stream_assets` metadata row. Dedup by sha256; delete is
//! refused (409, carrying the referencing scene names) while any `image`
//! element's props references the asset.

use super::util::RepositoryError;
use super::Repository;
use crate::entities::{stream_asset, stream_element, stream_scene};
use chrono::Utc;
use presenter_core::stream::{StreamAsset, StreamElementProps};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, NotSet, QueryFilter, QueryOrder,
    Set, TransactionTrait,
};
use tracing::instrument;

/// Input for [`Repository::insert_or_get_stream_asset`]. The upload handler
/// (#706+) fills this after hashing + validating the bytes; the repository only
/// records the metadata.
#[derive(Debug, Clone)]
pub struct NewStreamAsset {
    pub sha256: String,
    pub original_filename: String,
    pub mime: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

impl Repository {
    /// Insert a new asset row, or return the existing one with the same sha256
    /// (content-addressed dedup).
    #[instrument(skip_all)]
    pub async fn insert_or_get_stream_asset(
        &self,
        asset: NewStreamAsset,
    ) -> anyhow::Result<StreamAsset> {
        if asset.size_bytes < 0 || asset.size_bytes > i64::from(i32::MAX) {
            return Err(RepositoryError::Invalid(format!(
                "asset size {} out of range",
                asset.size_bytes
            ))
            .into());
        }
        if let Some(existing) = stream_asset::Entity::find()
            .filter(stream_asset::Column::Sha256.eq(asset.sha256.as_str()))
            .one(&self.db)
            .await?
        {
            return Ok(asset_from_model(existing));
        }
        let inserted = stream_asset::ActiveModel {
            id: NotSet,
            sha256: Set(asset.sha256),
            original_filename: Set(asset.original_filename),
            mime: Set(asset.mime),
            size_bytes: Set(asset.size_bytes as i32),
            width: Set(asset.width),
            height: Set(asset.height),
            created_at: Set(Utc::now().into()),
        }
        .insert(&self.db)
        .await?;
        Ok(asset_from_model(inserted))
    }

    pub async fn get_stream_asset(&self, id: i64) -> anyhow::Result<StreamAsset> {
        let model = stream_asset::Entity::find_by_id(id as i32)
            .one(&self.db)
            .await?
            .ok_or(RepositoryError::NotFound("stream asset not found"))?;
        Ok(asset_from_model(model))
    }

    pub async fn list_stream_assets(&self) -> anyhow::Result<Vec<StreamAsset>> {
        let models = stream_asset::Entity::find()
            .order_by_desc(stream_asset::Column::CreatedAt)
            .order_by_desc(stream_asset::Column::Id)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(asset_from_model).collect())
    }

    /// Delete an asset row. Refused with a 409 (carrying the referencing scene
    /// names) while any `image` element's props still references it.
    #[instrument(skip_all)]
    pub async fn delete_stream_asset(&self, id: i64) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let asset = stream_asset::Entity::find_by_id(id as i32)
            .one(&txn)
            .await?
            .ok_or(RepositoryError::NotFound("stream asset not found"))?;
        let referencing = Self::scenes_referencing_asset(&txn, asset.id as i64).await?;
        if !referencing.is_empty() {
            return Err(RepositoryError::ConflictDetail(format!(
                "asset is still referenced by scene(s): {}",
                referencing.join(", ")
            ))
            .into());
        }
        stream_asset::Entity::delete_by_id(asset.id)
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// Names of the scenes whose `image` elements reference `asset_id`
    /// (deduped, sorted) — powers the guarded-delete 409 message.
    async fn scenes_referencing_asset<C: ConnectionTrait>(
        conn: &C,
        asset_id: i64,
    ) -> anyhow::Result<Vec<String>> {
        let image_elements = stream_element::Entity::find()
            .filter(stream_element::Column::Kind.eq("image"))
            .all(conn)
            .await?;
        let mut names = Vec::new();
        for element in image_elements {
            let references = matches!(
                serde_json::from_str::<StreamElementProps>(&element.props),
                Ok(StreamElementProps::Image { asset_id: aid, .. }) if aid == asset_id
            );
            if !references {
                continue;
            }
            if let Some(scene) = stream_scene::Entity::find_by_id(element.scene_id)
                .one(conn)
                .await?
            {
                if !names.contains(&scene.name) {
                    names.push(scene.name);
                }
            }
        }
        names.sort();
        Ok(names)
    }
}

fn asset_from_model(model: stream_asset::Model) -> StreamAsset {
    StreamAsset {
        id: model.id as i64,
        sha256: model.sha256,
        original_filename: model.original_filename,
        mime: model.mime,
        size_bytes: model.size_bytes as i64,
        width: model.width,
        height: model.height,
    }
}
