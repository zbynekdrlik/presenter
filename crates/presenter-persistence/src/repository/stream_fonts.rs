//! Stream-graphics font records — sha256-addressed uploaded web-font metadata
//! (#778, epic #718). One-file-per-domain sibling of `stream_assets.rs`. The
//! bytes live on disk (`<stream-assets>/fonts/<sha256>.<ext>`); this is only the
//! `stream_fonts` metadata row (one per FACE = family+weight+italic). Dedup by
//! sha256. Delete is refused (409, carrying the referencing scene names) when
//! removing the LAST remaining face of a family that a text element still uses.
//!
//! [`Repository::distinct_font_families`] feeds the core validation
//! (`validate_props(props, extra_families)`): allowed families = the built-in
//! whitelist UNION the uploaded families.

use super::util::RepositoryError;
use super::Repository;
use crate::entities::{stream_element, stream_font, stream_scene};
use chrono::Utc;
use presenter_core::stream::{StreamElementProps, StreamFont};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, NotSet, PaginatorTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::BTreeSet;
use tracing::instrument;

/// Input for [`Repository::insert_or_get_stream_font`]. The upload handler
/// (#778) fills this after hashing + parsing the font metadata; the repository
/// only records the row.
#[derive(Debug, Clone)]
pub struct NewStreamFont {
    pub sha256: String,
    pub original_filename: String,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// On-disk container format: `"ttf"` or `"otf"`.
    pub format: String,
    pub size_bytes: i64,
}

impl Repository {
    /// Insert a new font row, or return the existing one with the same sha256
    /// (content-addressed dedup — re-uploading the identical file is a no-op).
    #[instrument(skip_all)]
    pub async fn insert_or_get_stream_font(
        &self,
        font: NewStreamFont,
    ) -> anyhow::Result<StreamFont> {
        if font.size_bytes < 0 || font.size_bytes > i64::from(i32::MAX) {
            return Err(RepositoryError::Invalid(format!(
                "font size {} out of range",
                font.size_bytes
            ))
            .into());
        }
        if let Some(existing) = stream_font::Entity::find()
            .filter(stream_font::Column::Sha256.eq(font.sha256.as_str()))
            .one(&self.db)
            .await?
        {
            tracing::info!(sha256 = %font.sha256, family = %existing.family, "stream font dedup: existing row reused");
            return Ok(font_from_model(existing));
        }
        let inserted = stream_font::ActiveModel {
            id: NotSet,
            sha256: Set(font.sha256),
            original_filename: Set(font.original_filename),
            family: Set(font.family),
            weight: Set(i32::from(font.weight)),
            italic: Set(font.italic),
            format: Set(font.format),
            size_bytes: Set(font.size_bytes as i32),
            created_at: Set(Utc::now().into()),
        }
        .insert(&self.db)
        .await?;
        tracing::info!(id = inserted.id, family = %inserted.family, weight = inserted.weight, italic = inserted.italic, "stream font row inserted");
        Ok(font_from_model(inserted))
    }

    /// All font faces, newest first (the editor picker groups them by family).
    pub async fn list_stream_fonts(&self) -> anyhow::Result<Vec<StreamFont>> {
        let models = stream_font::Entity::find()
            .order_by_desc(stream_font::Column::CreatedAt)
            .order_by_desc(stream_font::Column::Id)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(font_from_model).collect())
    }

    pub async fn get_stream_font(&self, id: i64) -> anyhow::Result<StreamFont> {
        // Refuse an out-of-i32-range id rather than wrap-truncate into a wrong
        // row (the #705 IDOR guard).
        let id =
            i32::try_from(id).map_err(|_| RepositoryError::NotFound("stream font not found"))?;
        let model = stream_font::Entity::find_by_id(id)
            .one(&self.db)
            .await?
            .ok_or(RepositoryError::NotFound("stream font not found"))?;
        Ok(font_from_model(model))
    }

    /// The DISTINCT uploaded font families — the extra set unioned with the
    /// built-in whitelist by core validation (`validate_props`). Deduped in Rust
    /// (the font count is small; avoids a DISTINCT-projection query).
    pub async fn distinct_font_families(&self) -> anyhow::Result<Vec<String>> {
        let families: BTreeSet<String> = stream_font::Entity::find()
            .all(&self.db)
            .await?
            .into_iter()
            .map(|f| f.family)
            .collect();
        Ok(families.into_iter().collect())
    }

    /// Delete a font face. Refused with a 409 (carrying the referencing scene
    /// names) when it is the LAST remaining face of a family that a text element
    /// still uses — removing it would invalidate that element's stored props.
    /// When another face of the same family remains, the family stays valid and
    /// the delete proceeds.
    #[instrument(skip_all)]
    pub async fn delete_stream_font(&self, id: i64) -> anyhow::Result<()> {
        let id =
            i32::try_from(id).map_err(|_| RepositoryError::NotFound("stream font not found"))?;
        let txn = self.db.begin().await?;
        let font = stream_font::Entity::find_by_id(id)
            .one(&txn)
            .await?
            .ok_or(RepositoryError::NotFound("stream font not found"))?;

        // Any OTHER face of the same family left after this delete?
        let remaining_faces = stream_font::Entity::find()
            .filter(stream_font::Column::Family.eq(font.family.as_str()))
            .filter(stream_font::Column::Id.ne(font.id))
            .count(&txn)
            .await?;

        if remaining_faces == 0 {
            let referencing = Self::scenes_referencing_font_family(&txn, &font.family).await?;
            if !referencing.is_empty() {
                tracing::warn!(id = font.id, family = %font.family, scenes = ?referencing, "stream font delete refused: last face of a family still in use");
                return Err(RepositoryError::ConflictDetail(format!(
                    "font family {:?} is still used by scene(s): {}",
                    font.family,
                    referencing.join(", ")
                ))
                .into());
            }
        }

        stream_font::Entity::delete_by_id(font.id)
            .exec(&txn)
            .await?;
        txn.commit().await?;
        tracing::info!(id, family = %font.family, "stream font row deleted");
        Ok(())
    }

    /// Names of the scenes whose text elements reference `family` (deduped,
    /// sorted) — powers the guarded-delete 409 message.
    async fn scenes_referencing_font_family<C: ConnectionTrait>(
        conn: &C,
        family: &str,
    ) -> anyhow::Result<Vec<String>> {
        let elements = stream_element::Entity::find().all(conn).await?;
        let mut names = BTreeSet::new();
        for element in elements {
            let uses_family = serde_json::from_str::<StreamElementProps>(&element.props)
                .map(|props| props.font_families().iter().any(|f| *f == family))
                .unwrap_or(false);
            if !uses_family {
                continue;
            }
            if let Some(scene) = stream_scene::Entity::find_by_id(element.scene_id)
                .one(conn)
                .await?
            {
                names.insert(scene.name);
            }
        }
        Ok(names.into_iter().collect())
    }
}

fn font_from_model(model: stream_font::Model) -> StreamFont {
    StreamFont {
        id: model.id as i64,
        sha256: model.sha256,
        original_filename: model.original_filename,
        family: model.family,
        weight: model.weight as u16,
        italic: model.italic,
        format: model.format,
        size_bytes: model.size_bytes as i64,
    }
}
