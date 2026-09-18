//! Stream-graphics lower-third nameplate repository (#779, epic #718).
//!
//! CRUD + reorder for the PERSON plate list (`stream_nameplates`), addressed by
//! output SLUG for list/create/reorder and by numeric id for
//! update/delete/lookup. The SONG plate is virtual (resolved server-side from
//! the live stage snapshot) and never a row here. Refusals are the typed
//! `RepositoryError` variants (`.claude/rules/repository-error-pattern.md`):
//! `NotFound` → 404, `Conflict` → 409, `Invalid` → 422. The nameplate list is
//! NOT part of the element/scene `config_revision` (its own `StreamNameplatesChanged`
//! live event drives the editor/Companion refetch), so these writes do NOT bump
//! `config_revision`.

use super::util::RepositoryError;
use super::Repository;
use crate::entities::{stream_nameplate, stream_output};
use chrono::Utc;
use presenter_core::stream::{Nameplate, NameplateKind};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, NotSet,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

/// Maximum plate-text length (name / role), in characters — a guard against a
/// paste of unbounded text into a plate field.
const NAMEPLATE_TEXT_MAX: usize = 200;

impl Repository {
    /// List an output's person plates, ordered by `position` then id.
    pub async fn list_stream_nameplates(&self, slug: &str) -> anyhow::Result<Vec<Nameplate>> {
        let output_id = Self::nameplate_output_id_by_slug(&self.db, slug).await?;
        let models = stream_nameplate::Entity::find()
            .filter(stream_nameplate::Column::OutputId.eq(output_id))
            .order_by_asc(stream_nameplate::Column::Position)
            .order_by_asc(stream_nameplate::Column::Id)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(nameplate_from_model).collect())
    }

    /// Create a person plate at the end of the output's list.
    #[instrument(skip_all)]
    pub async fn create_stream_nameplate(
        &self,
        slug: &str,
        primary_text: &str,
        secondary_text: &str,
    ) -> anyhow::Result<Nameplate> {
        let primary = validate_nameplate_text("primary_text", primary_text, false)?;
        let secondary = validate_nameplate_text("secondary_text", secondary_text, true)?;
        let txn = self.db.begin().await?;
        let output_id = Self::nameplate_output_id_by_slug(&txn, slug).await?;
        let position = Self::next_nameplate_position(&txn, output_id).await?;
        let now = Utc::now();
        let inserted = stream_nameplate::ActiveModel {
            id: NotSet,
            output_id: Set(output_id),
            primary_text: Set(primary),
            secondary_text: Set(secondary),
            position: Set(position),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        Ok(nameplate_from_model(inserted))
    }

    /// Update a person plate's texts.
    #[instrument(skip_all)]
    pub async fn update_stream_nameplate(
        &self,
        nameplate_id: i64,
        primary_text: &str,
        secondary_text: &str,
    ) -> anyhow::Result<Nameplate> {
        let primary = validate_nameplate_text("primary_text", primary_text, false)?;
        let secondary = validate_nameplate_text("secondary_text", secondary_text, true)?;
        let txn = self.db.begin().await?;
        let plate = Self::nameplate_by_id(&txn, nameplate_id).await?;
        let mut active = plate.into_active_model();
        active.primary_text = Set(primary);
        active.secondary_text = Set(secondary);
        active.updated_at = Set(Utc::now().into());
        let updated = active.update(&txn).await?;
        txn.commit().await?;
        Ok(nameplate_from_model(updated))
    }

    /// Delete a person plate.
    #[instrument(skip_all)]
    pub async fn delete_stream_nameplate(&self, nameplate_id: i64) -> anyhow::Result<()> {
        let plate = Self::nameplate_by_id(&self.db, nameplate_id).await?;
        stream_nameplate::Entity::delete_by_id(plate.id)
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Fetch one person plate by id (used to resolve its texts at show time).
    pub async fn get_stream_nameplate(&self, nameplate_id: i64) -> anyhow::Result<Nameplate> {
        Ok(nameplate_from_model(
            Self::nameplate_by_id(&self.db, nameplate_id).await?,
        ))
    }

    /// The owning output's slug for a plate id — the id-addressed
    /// update/delete handlers must broadcast `StreamNameplatesChanged` on the
    /// owning output. A missing plate id surfaces `NotFound` (404).
    pub async fn stream_nameplate_output_slug(&self, nameplate_id: i64) -> anyhow::Result<String> {
        let plate = Self::nameplate_by_id(&self.db, nameplate_id).await?;
        let output = stream_output::Entity::find_by_id(plate.output_id)
            .one(&self.db)
            .await?
            .ok_or(RepositoryError::NotFound("stream output not found"))?;
        Ok(output.slug)
    }

    /// Rewrite the plate order: `ids` MUST be exactly the output's full plate set
    /// (no dupes, none missing); `position` is rewritten 0..n by list order. A
    /// partial/duplicate set is `Invalid` (422). Mirrors `set_scene_order`.
    #[instrument(skip_all)]
    pub async fn set_nameplate_order(&self, slug: &str, ids: Vec<i64>) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let output_id = Self::nameplate_output_id_by_slug(&txn, slug).await?;
        let plates = stream_nameplate::Entity::find()
            .filter(stream_nameplate::Column::OutputId.eq(output_id))
            .all(&txn)
            .await?;
        let existing: HashSet<i64> = plates.iter().map(|p| p.id as i64).collect();
        let requested: HashSet<i64> = ids.iter().copied().collect();
        if requested.len() != ids.len() {
            return Err(RepositoryError::Invalid(
                "nameplate order contains duplicate ids".to_string(),
            )
            .into());
        }
        if existing != requested {
            return Err(RepositoryError::Invalid(
                "nameplate order id set does not match".to_string(),
            )
            .into());
        }
        let by_id: HashMap<i64, &stream_nameplate::Model> =
            plates.iter().map(|p| (p.id as i64, p)).collect();
        for (position, id) in ids.iter().enumerate() {
            let Some(plate) = by_id.get(id).copied() else {
                continue;
            };
            let mut active = plate.clone().into_active_model();
            active.position = Set(position as i32);
            active.updated_at = Set(Utc::now().into());
            active.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    // ---- Private helpers --------------------------------------------------

    async fn nameplate_output_id_by_slug<C: ConnectionTrait>(
        conn: &C,
        slug: &str,
    ) -> anyhow::Result<i32> {
        let output = stream_output::Entity::find()
            .filter(stream_output::Column::Slug.eq(slug))
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream output not found"))?;
        Ok(output.id)
    }

    async fn nameplate_by_id<C: ConnectionTrait>(
        conn: &C,
        nameplate_id: i64,
    ) -> anyhow::Result<stream_nameplate::Model> {
        // An out-of-i32-range id can never be a real row — refuse rather than
        // wrap-truncate into a wrong id (stream-graphics.md i32/i64 rule).
        let id = i32::try_from(nameplate_id)
            .map_err(|_| RepositoryError::NotFound("stream nameplate not found"))?;
        stream_nameplate::Entity::find_by_id(id)
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream nameplate not found").into())
    }

    async fn next_nameplate_position<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
    ) -> anyhow::Result<i32> {
        let plates = stream_nameplate::Entity::find()
            .filter(stream_nameplate::Column::OutputId.eq(output_id))
            .all(conn)
            .await?;
        Ok(plates.iter().map(|p| p.position).max().unwrap_or(-1) + 1)
    }
}

fn nameplate_from_model(model: stream_nameplate::Model) -> Nameplate {
    Nameplate {
        id: model.id as i64,
        // Every persisted row is a PERSON plate (the song plate is virtual).
        kind: NameplateKind::Person,
        primary_text: model.primary_text,
        secondary_text: model.secondary_text,
        sort_order: model.position,
    }
}

/// Trim + bound a plate text. `allow_empty` is true for the secondary (role) —
/// a person plate may carry only a name. A blank primary is `Invalid` (422).
fn validate_nameplate_text(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if !allow_empty && trimmed.is_empty() {
        return Err(RepositoryError::Invalid(format!("{field} must be non-empty")).into());
    }
    if trimmed.chars().count() > NAMEPLATE_TEXT_MAX {
        return Err(
            RepositoryError::Invalid(format!("{field} must be at most 200 characters")).into(),
        );
    }
    Ok(trimmed.to_string())
}
