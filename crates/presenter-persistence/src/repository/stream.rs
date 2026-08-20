//! Stream-graphics repository — config CRUD (outputs / scenes / elements),
//! activation persistence, and full output-def assembly, so the router/state
//! layers never touch SeaORM for the stream tables (#705, epic #718 §5/§9).
//!
//! Self-contained `impl Repository` block in its own file, the same
//! one-file-per-domain pattern as `video_source.rs`/`resolume.rs`. Refusals are
//! the typed `RepositoryError` variants (`.claude/rules/repository-error-pattern.md`):
//! `NotFound` → 404, `Conflict`/`ConflictDetail` → 409, `Invalid` → 422.
//! Element props are validated + stored via the #704 core types
//! (`presenter_core::stream`). Stream config is NOT settings-audited (arch
//! decision #3 — it is authored content, not an audited user setting).
//!
//! `config_revision` is bumped on every CONFIG write (output/scene/element
//! create/rename/patch/delete + reorder) — NOT on activation, which is
//! show-state broadcast by the separate `StreamState` event (arch §6/§8).

use super::util::RepositoryError;
use super::Repository;
use crate::entities::{stream_element, stream_output, stream_scene};
use chrono::Utc;
use presenter_core::stream::{
    validate_props, validate_scene_name, validate_slug, SceneKind, StreamElementDef,
    StreamElementProps, StreamOutputDef, StreamOutputSummary, StreamSceneDef, StreamShowState,
    STREAM_TRANSITION_MAX_MS,
};
use sea_orm::{
    sea_query::Expr, ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    NotSet, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

/// Default `stream_outputs.default_transition_ms` for a freshly-created output
/// (matches the migration's column default).
const DEFAULT_OUTPUT_TRANSITION_MS: i32 = 400;

impl Repository {
    // ---- Output CRUD ------------------------------------------------------

    #[instrument(skip_all)]
    pub async fn create_stream_output(
        &self,
        slug: &str,
        name: &str,
    ) -> anyhow::Result<StreamOutputSummary> {
        validate_slug(slug).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
        let trimmed = non_empty_name(name)?;
        if stream_output::Entity::find()
            .filter(stream_output::Column::Slug.eq(slug))
            .one(&self.db)
            .await?
            .is_some()
        {
            return Err(RepositoryError::Conflict("stream output slug already exists").into());
        }
        let now = Utc::now();
        let inserted = stream_output::ActiveModel {
            id: NotSet,
            slug: Set(slug.to_string()),
            name: Set(trimmed),
            default_transition_ms: Set(DEFAULT_OUTPUT_TRANSITION_MS),
            active_scene_id: Set(None),
            config_revision: Set(0),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(&self.db)
        .await?;
        Ok(output_summary_from_model(inserted))
    }

    pub async fn list_stream_outputs(&self) -> anyhow::Result<Vec<StreamOutputSummary>> {
        let models = stream_output::Entity::find()
            .order_by_asc(stream_output::Column::Slug)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(output_summary_from_model).collect())
    }

    pub async fn get_stream_output(&self, slug: &str) -> anyhow::Result<StreamOutputSummary> {
        Ok(output_summary_from_model(
            Self::output_by_slug(&self.db, slug).await?,
        ))
    }

    #[instrument(skip_all)]
    pub async fn rename_stream_output(
        &self,
        slug: &str,
        name: &str,
    ) -> anyhow::Result<StreamOutputSummary> {
        let trimmed = non_empty_name(name)?;
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        let next_revision = output.config_revision + 1;
        let mut active = output.into_active_model();
        active.name = Set(trimmed);
        active.config_revision = Set(next_revision);
        active.updated_at = Set(Utc::now().into());
        let updated = active.update(&txn).await?;
        txn.commit().await?;
        Ok(output_summary_from_model(updated))
    }

    #[instrument(skip_all)]
    pub async fn set_stream_output_transition(
        &self,
        slug: &str,
        default_transition_ms: u32,
    ) -> anyhow::Result<StreamOutputSummary> {
        check_transition_ms(default_transition_ms)?;
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        let next_revision = output.config_revision + 1;
        let mut active = output.into_active_model();
        active.default_transition_ms = Set(default_transition_ms as i32);
        active.config_revision = Set(next_revision);
        active.updated_at = Set(Utc::now().into());
        let updated = active.update(&txn).await?;
        txn.commit().await?;
        Ok(output_summary_from_model(updated))
    }

    #[instrument(skip_all)]
    pub async fn delete_stream_output(&self, slug: &str) -> anyhow::Result<()> {
        let output = Self::output_by_slug(&self.db, slug).await?;
        // FK ON DELETE CASCADE removes the output's scenes + their elements.
        stream_output::Entity::delete_by_id(output.id)
            .exec(&self.db)
            .await?;
        Ok(())
    }

    // ---- Scene CRUD -------------------------------------------------------

    #[instrument(skip_all)]
    pub async fn create_stream_scene(
        &self,
        slug: &str,
        name: &str,
        kind: SceneKind,
    ) -> anyhow::Result<StreamSceneDef> {
        validate_scene_name(name).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
        let trimmed = name.trim().to_string();
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        Self::ensure_scene_name_unique(&txn, output.id, &trimmed, None).await?;
        let position = Self::next_scene_position(&txn, output.id, kind).await?;
        let now = Utc::now();
        let inserted = stream_scene::ActiveModel {
            id: NotSet,
            output_id: Set(output.id),
            name: Set(trimmed),
            kind: Set(kind.as_str().to_string()),
            position: Set(position),
            is_active: Set(0),
            transition_ms: Set(None),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(&txn)
        .await?;
        Self::bump_config_revision(&txn, output.id).await?;
        txn.commit().await?;
        scene_def_from_model(inserted, Vec::new())
    }

    #[instrument(skip_all)]
    pub async fn rename_stream_scene(
        &self,
        scene_id: i64,
        name: &str,
    ) -> anyhow::Result<StreamSceneDef> {
        validate_scene_name(name).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
        let trimmed = name.trim().to_string();
        let txn = self.db.begin().await?;
        let scene = Self::scene_by_id(&txn, scene_id).await?;
        Self::ensure_scene_name_unique(&txn, scene.output_id, &trimmed, Some(scene.id)).await?;
        let output_id = scene.output_id;
        let mut active = scene.into_active_model();
        active.name = Set(trimmed);
        active.updated_at = Set(Utc::now().into());
        let updated = active.update(&txn).await?;
        Self::bump_config_revision(&txn, output_id).await?;
        let elements = Self::elements_for_scene(&txn, updated.id).await?;
        txn.commit().await?;
        scene_def_from_model(updated, elements)
    }

    #[instrument(skip_all)]
    pub async fn set_stream_scene_transition(
        &self,
        scene_id: i64,
        transition_ms: Option<u32>,
    ) -> anyhow::Result<StreamSceneDef> {
        if let Some(ms) = transition_ms {
            check_transition_ms(ms)?;
        }
        let txn = self.db.begin().await?;
        let scene = Self::scene_by_id(&txn, scene_id).await?;
        let output_id = scene.output_id;
        let mut active = scene.into_active_model();
        active.transition_ms = Set(transition_ms.map(|v| v as i32));
        active.updated_at = Set(Utc::now().into());
        let updated = active.update(&txn).await?;
        Self::bump_config_revision(&txn, output_id).await?;
        let elements = Self::elements_for_scene(&txn, updated.id).await?;
        txn.commit().await?;
        scene_def_from_model(updated, elements)
    }

    #[instrument(skip_all)]
    pub async fn delete_stream_scene(&self, scene_id: i64) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let scene = Self::scene_by_id(&txn, scene_id).await?;
        let output_id = scene.output_id;
        let output = Self::output_by_id(&txn, output_id).await?;
        // Deleting the active BASE scene clears the output's activation.
        if output.active_scene_id == Some(scene.id) {
            let mut active = output.into_active_model();
            active.active_scene_id = Set(None);
            active.updated_at = Set(Utc::now().into());
            active.update(&txn).await?;
        }
        // FK ON DELETE CASCADE removes this scene's elements.
        stream_scene::Entity::delete_by_id(scene.id)
            .exec(&txn)
            .await?;
        Self::bump_config_revision(&txn, output_id).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Rewrite scene L→R order. `ids` MUST be exactly the output's full scene
    /// set (no dupes, no extras, none missing); positions are rewritten per
    /// kind in the order the ids appear.
    #[instrument(skip_all)]
    pub async fn set_scene_order(&self, slug: &str, ids: Vec<i64>) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        let scenes = stream_scene::Entity::find()
            .filter(stream_scene::Column::OutputId.eq(output.id))
            .all(&txn)
            .await?;
        Self::validate_scene_order_set(&scenes, &ids)?;
        let by_id: HashMap<i64, &stream_scene::Model> =
            scenes.iter().map(|s| (s.id as i64, s)).collect();
        let (mut base_pos, mut overlay_pos) = (0i32, 0i32);
        for id in &ids {
            let Some(scene) = by_id.get(id).copied() else {
                continue;
            };
            let position = match SceneKind::from_db(&scene.kind) {
                Some(SceneKind::Base) => next_index(&mut base_pos),
                Some(SceneKind::Overlay) => next_index(&mut overlay_pos),
                None => continue,
            };
            let mut active = scene.clone().into_active_model();
            active.position = Set(position);
            active.updated_at = Set(Utc::now().into());
            active.update(&txn).await?;
        }
        Self::bump_config_revision(&txn, output.id).await?;
        txn.commit().await?;
        Ok(())
    }

    // ---- Element CRUD -----------------------------------------------------

    #[instrument(skip_all)]
    pub async fn create_stream_element(
        &self,
        scene_id: i64,
        props: StreamElementProps,
    ) -> anyhow::Result<StreamElementDef> {
        validate_props(&props).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
        let props_json = serde_json::to_string(&props)?;
        let txn = self.db.begin().await?;
        let scene = Self::scene_by_id(&txn, scene_id).await?;
        let output_id = scene.output_id;
        let z_order = Self::next_element_z_order(&txn, scene.id).await?;
        let now = Utc::now();
        let inserted = stream_element::ActiveModel {
            id: NotSet,
            scene_id: Set(scene.id),
            kind: Set(props.kind_str().to_string()),
            z_order: Set(z_order),
            props: Set(props_json),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(&txn)
        .await?;
        Self::bump_config_revision(&txn, output_id).await?;
        txn.commit().await?;
        Ok(StreamElementDef {
            id: inserted.id as i64,
            z_order: inserted.z_order,
            props,
        })
    }

    #[instrument(skip_all)]
    pub async fn update_stream_element(
        &self,
        element_id: i64,
        props: StreamElementProps,
    ) -> anyhow::Result<StreamElementDef> {
        validate_props(&props).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
        let txn = self.db.begin().await?;
        let element = Self::element_by_id(&txn, element_id).await?;
        if element.kind.as_str() != props.kind_str() {
            return Err(RepositoryError::Invalid(format!(
                "props kind {:?} does not match element kind {:?}",
                props.kind_str(),
                element.kind
            ))
            .into());
        }
        let props_json = serde_json::to_string(&props)?;
        let (scene_id, z_order, element_id_i32) = (element.scene_id, element.z_order, element.id);
        let mut active = element.into_active_model();
        active.props = Set(props_json);
        active.updated_at = Set(Utc::now().into());
        active.update(&txn).await?;
        let output_id = Self::scene_output_id(&txn, scene_id).await?;
        Self::bump_config_revision(&txn, output_id).await?;
        txn.commit().await?;
        Ok(StreamElementDef {
            id: element_id_i32 as i64,
            z_order,
            props,
        })
    }

    #[instrument(skip_all)]
    pub async fn delete_stream_element(&self, element_id: i64) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let element = Self::element_by_id(&txn, element_id).await?;
        let output_id = Self::scene_output_id(&txn, element.scene_id).await?;
        stream_element::Entity::delete_by_id(element.id)
            .exec(&txn)
            .await?;
        Self::bump_config_revision(&txn, output_id).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Reorder a scene's elements by list order — the client sends the FULL set
    /// of the scene's element ids (no dupes, none missing) and z_order is
    /// reassigned 0..n by that order. Mirrors [`Self::set_scene_order`] (the
    /// per-output scene reorder); a partial/duplicate set is `Invalid` (422).
    /// Bumps `config_revision` (a CONFIG write).
    #[instrument(skip_all)]
    pub async fn set_element_order(&self, scene_id: i64, ids: Vec<i64>) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let scene = Self::scene_by_id(&txn, scene_id).await?;
        let elements = stream_element::Entity::find()
            .filter(stream_element::Column::SceneId.eq(scene.id))
            .all(&txn)
            .await?;
        Self::validate_element_order_set(&elements, &ids)?;
        let by_id: HashMap<i64, &stream_element::Model> =
            elements.iter().map(|e| (e.id as i64, e)).collect();
        let mut z = 0i32;
        for id in &ids {
            let Some(element) = by_id.get(id).copied() else {
                continue;
            };
            let mut active = element.clone().into_active_model();
            active.z_order = Set(next_index(&mut z));
            active.updated_at = Set(Utc::now().into());
            active.update(&txn).await?;
        }
        Self::bump_config_revision(&txn, scene.output_id).await?;
        txn.commit().await?;
        Ok(())
    }

    // ---- Activation (show-state; NO config_revision bump) -----------------

    #[instrument(skip_all)]
    pub async fn set_active_scene(&self, slug: &str, scene_id: Option<i64>) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        if let Some(sid) = scene_id {
            let scene = Self::scene_in_output(&txn, output.id, sid).await?;
            if SceneKind::from_db(&scene.kind) != Some(SceneKind::Base) {
                return Err(
                    RepositoryError::Invalid("scene is not a base scene".to_string()).into(),
                );
            }
        }
        let mut active = output.into_active_model();
        active.active_scene_id = Set(scene_id.map(|v| v as i32));
        active.updated_at = Set(Utc::now().into());
        active.update(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn set_overlay_active(
        &self,
        slug: &str,
        scene_id: i64,
        active: bool,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        let scene = Self::scene_in_output(&txn, output.id, scene_id).await?;
        if SceneKind::from_db(&scene.kind) != Some(SceneKind::Overlay) {
            return Err(
                RepositoryError::Invalid("scene is not an overlay scene".to_string()).into(),
            );
        }
        let mut model = scene.into_active_model();
        model.is_active = Set(i32::from(active));
        model.updated_at = Set(Utc::now().into());
        model.update(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn clear_stream_output(&self, slug: &str) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let output = Self::output_by_slug(&txn, slug).await?;
        let output_id = output.id;
        let mut active = output.into_active_model();
        active.active_scene_id = Set(None);
        active.updated_at = Set(Utc::now().into());
        active.update(&txn).await?;
        stream_scene::Entity::update_many()
            .col_expr(stream_scene::Column::IsActive, Expr::value(0))
            .col_expr(
                stream_scene::Column::UpdatedAt,
                Expr::value(Into::<sea_orm::prelude::DateTimeWithTimeZone>::into(
                    Utc::now(),
                )),
            )
            .filter(stream_scene::Column::OutputId.eq(output_id))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// The persisted activation snapshot for cold OBS load / server restart.
    pub async fn get_stream_show_state(&self, slug: &str) -> anyhow::Result<StreamShowState> {
        let output = Self::output_by_slug(&self.db, slug).await?;
        let overlays = stream_scene::Entity::find()
            .filter(stream_scene::Column::OutputId.eq(output.id))
            .filter(stream_scene::Column::Kind.eq(SceneKind::Overlay.as_str()))
            .filter(stream_scene::Column::IsActive.eq(1))
            .order_by_asc(stream_scene::Column::Position)
            .order_by_asc(stream_scene::Column::Id)
            .all(&self.db)
            .await?;
        Ok(StreamShowState {
            active_scene_id: output.active_scene_id.map(|v| v as i64),
            active_overlay_ids: overlays.iter().map(|s| s.id as i64).collect(),
            config_revision: output.config_revision.max(0) as u64,
        })
    }

    // ---- Full definition assembly ----------------------------------------

    /// Assemble the full output definition: scenes ordered (base before
    /// overlay, then position, then id), each scene's elements ordered by
    /// z_order. An element whose stored props JSON fails to parse is SKIPPED
    /// (warned) — never a whole-def failure.
    pub async fn load_output_def(&self, slug: &str) -> anyhow::Result<StreamOutputDef> {
        let output = Self::output_by_slug(&self.db, slug).await?;
        let mut scene_models = stream_scene::Entity::find()
            .filter(stream_scene::Column::OutputId.eq(output.id))
            .all(&self.db)
            .await?;
        scene_models.sort_by(|a, b| {
            kind_rank(&a.kind)
                .cmp(&kind_rank(&b.kind))
                .then(a.position.cmp(&b.position))
                .then(a.id.cmp(&b.id))
        });
        let mut scenes = Vec::with_capacity(scene_models.len());
        for scene in scene_models {
            let elements = Self::elements_for_scene(&self.db, scene.id).await?;
            match scene_def_from_model(scene, elements) {
                Ok(def) => scenes.push(def),
                Err(error) => {
                    tracing::warn!(%error, "skipping stream scene with unrecognized kind")
                }
            }
        }
        Ok(StreamOutputDef {
            id: output.id as i64,
            slug: output.slug,
            name: output.name,
            default_transition_ms: output.default_transition_ms.max(0) as u32,
            active_scene_id: output.active_scene_id.map(|v| v as i64),
            config_revision: output.config_revision.max(0) as u64,
            scenes,
        })
    }

    // ---- Owning-output resolution (router notify targeting, #707) ---------

    /// The owning output's slug for a scene id. The #707 scene-patch/delete
    /// handlers are addressed by scene id but must broadcast the CONFIG change
    /// on the owning output — this resolves that slug. A missing scene id
    /// surfaces `NotFound` (404), reusing the same `scene_by_id` lookup as the
    /// mutating paths (so an out-of-`i32`-range id refuses rather than
    /// wrap-truncates).
    pub async fn stream_scene_output_slug(&self, scene_id: i64) -> anyhow::Result<String> {
        let scene = Self::scene_by_id(&self.db, scene_id).await?;
        Ok(Self::output_by_id(&self.db, scene.output_id).await?.slug)
    }

    /// The owning output's slug for an element id — the element-handler
    /// counterpart of [`Self::stream_scene_output_slug`] (#707). A missing
    /// element id surfaces `NotFound` (404).
    pub async fn stream_element_output_slug(&self, element_id: i64) -> anyhow::Result<String> {
        let element = Self::element_by_id(&self.db, element_id).await?;
        let scene = Self::scene_by_id(&self.db, element.scene_id as i64).await?;
        Ok(Self::output_by_id(&self.db, scene.output_id).await?.slug)
    }

    // ---- Private helpers --------------------------------------------------

    async fn output_by_slug<C: ConnectionTrait>(
        conn: &C,
        slug: &str,
    ) -> anyhow::Result<stream_output::Model> {
        let model = stream_output::Entity::find()
            .filter(stream_output::Column::Slug.eq(slug))
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream output not found"))?;
        Ok(model)
    }

    async fn output_by_id<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
    ) -> anyhow::Result<stream_output::Model> {
        let model = stream_output::Entity::find_by_id(output_id)
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream output not found"))?;
        Ok(model)
    }

    async fn scene_by_id<C: ConnectionTrait>(
        conn: &C,
        scene_id: i64,
    ) -> anyhow::Result<stream_scene::Model> {
        // An out-of-i32-range id can never be a real row — refuse rather than
        // wrap-truncate into a wrong id (review WARNING).
        let id = i32::try_from(scene_id)
            .map_err(|_| RepositoryError::NotFound("stream scene not found"))?;
        let model = stream_scene::Entity::find_by_id(id)
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream scene not found"))?;
        Ok(model)
    }

    /// Fetch a scene that MUST belong to `output_id` — used by the activation
    /// paths. A missing body-referenced scene is `TargetNotFound` (422, the
    /// documented taxonomy for a missing body target); a wrong-output scene is
    /// `Invalid` (422, the target exists but is not valid for this request).
    async fn scene_in_output<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
        scene_id: i64,
    ) -> anyhow::Result<stream_scene::Model> {
        let id = i32::try_from(scene_id)
            .map_err(|_| RepositoryError::TargetNotFound("scene not found"))?;
        let scene = stream_scene::Entity::find_by_id(id)
            .one(conn)
            .await?
            .ok_or(RepositoryError::TargetNotFound("scene not found"))?;
        if scene.output_id != output_id {
            return Err(RepositoryError::Invalid(
                "scene does not belong to this output".to_string(),
            )
            .into());
        }
        Ok(scene)
    }

    async fn element_by_id<C: ConnectionTrait>(
        conn: &C,
        element_id: i64,
    ) -> anyhow::Result<stream_element::Model> {
        let id = i32::try_from(element_id)
            .map_err(|_| RepositoryError::NotFound("stream element not found"))?;
        let model = stream_element::Entity::find_by_id(id)
            .one(conn)
            .await?
            .ok_or(RepositoryError::NotFound("stream element not found"))?;
        Ok(model)
    }

    async fn scene_output_id<C: ConnectionTrait>(conn: &C, scene_id: i32) -> anyhow::Result<i32> {
        Ok(Self::scene_by_id(conn, scene_id as i64).await?.output_id)
    }

    async fn ensure_scene_name_unique<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
        name: &str,
        exclude_scene_id: Option<i32>,
    ) -> anyhow::Result<()> {
        let scenes = stream_scene::Entity::find()
            .filter(stream_scene::Column::OutputId.eq(output_id))
            .all(conn)
            .await?;
        let target = name.to_lowercase();
        let clash = scenes
            .iter()
            .any(|s| Some(s.id) != exclude_scene_id && s.name.to_lowercase() == target);
        if clash {
            return Err(
                RepositoryError::Conflict("scene name already exists in this output").into(),
            );
        }
        Ok(())
    }

    async fn next_scene_position<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
        kind: SceneKind,
    ) -> anyhow::Result<i32> {
        let scenes = stream_scene::Entity::find()
            .filter(stream_scene::Column::OutputId.eq(output_id))
            .filter(stream_scene::Column::Kind.eq(kind.as_str()))
            .all(conn)
            .await?;
        Ok(scenes.iter().map(|s| s.position).max().unwrap_or(-1) + 1)
    }

    async fn next_element_z_order<C: ConnectionTrait>(
        conn: &C,
        scene_id: i32,
    ) -> anyhow::Result<i32> {
        let elements = stream_element::Entity::find()
            .filter(stream_element::Column::SceneId.eq(scene_id))
            .all(conn)
            .await?;
        Ok(elements.iter().map(|e| e.z_order).max().unwrap_or(-1) + 1)
    }

    async fn elements_for_scene<C: ConnectionTrait>(
        conn: &C,
        scene_id: i32,
    ) -> anyhow::Result<Vec<StreamElementDef>> {
        let models = stream_element::Entity::find()
            .filter(stream_element::Column::SceneId.eq(scene_id))
            .order_by_asc(stream_element::Column::ZOrder)
            .order_by_asc(stream_element::Column::Id)
            .all(conn)
            .await?;
        Ok(models
            .into_iter()
            .filter_map(element_def_from_model)
            .collect())
    }

    /// Bump `config_revision` on the output (every CONFIG write). NOT called on
    /// activation writes.
    async fn bump_config_revision<C: ConnectionTrait>(
        conn: &C,
        output_id: i32,
    ) -> anyhow::Result<()> {
        let output = Self::output_by_id(conn, output_id).await?;
        let next = output.config_revision + 1;
        let mut active = output.into_active_model();
        active.config_revision = Set(next);
        active.updated_at = Set(Utc::now().into());
        active.update(conn).await?;
        Ok(())
    }

    fn validate_scene_order_set(scenes: &[stream_scene::Model], ids: &[i64]) -> anyhow::Result<()> {
        let requested: HashSet<i64> = ids.iter().copied().collect();
        if requested.len() != ids.len() {
            return Err(
                RepositoryError::Invalid("scene order contains duplicate ids".to_string()).into(),
            );
        }
        let existing: HashSet<i64> = scenes.iter().map(|s| s.id as i64).collect();
        if existing != requested {
            return Err(RepositoryError::Invalid(
                "scene order id set does not match this output's scenes".to_string(),
            )
            .into());
        }
        Ok(())
    }

    fn validate_element_order_set(
        elements: &[stream_element::Model],
        ids: &[i64],
    ) -> anyhow::Result<()> {
        let requested: HashSet<i64> = ids.iter().copied().collect();
        if requested.len() != ids.len() {
            return Err(RepositoryError::Invalid(
                "element order contains duplicate ids".to_string(),
            )
            .into());
        }
        let existing: HashSet<i64> = elements.iter().map(|e| e.id as i64).collect();
        if existing != requested {
            return Err(RepositoryError::Invalid(
                "element order id set does not match this scene's elements".to_string(),
            )
            .into());
        }
        Ok(())
    }
}

fn non_empty_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(RepositoryError::Invalid("output name must be non-empty".to_string()).into());
    }
    Ok(trimmed.to_string())
}

fn check_transition_ms(ms: u32) -> anyhow::Result<()> {
    if ms > STREAM_TRANSITION_MAX_MS {
        return Err(RepositoryError::Invalid(format!(
            "transition {ms}ms out of range (expected <=10000)"
        ))
        .into());
    }
    Ok(())
}

fn next_index(counter: &mut i32) -> i32 {
    let current = *counter;
    *counter += 1;
    current
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "base" => 0,
        "overlay" => 1,
        _ => 2,
    }
}

fn output_summary_from_model(model: stream_output::Model) -> StreamOutputSummary {
    StreamOutputSummary {
        id: model.id as i64,
        slug: model.slug,
        name: model.name,
        default_transition_ms: model.default_transition_ms.max(0) as u32,
        active_scene_id: model.active_scene_id.map(|v| v as i64),
        config_revision: model.config_revision.max(0) as u64,
    }
}

fn scene_def_from_model(
    model: stream_scene::Model,
    elements: Vec<StreamElementDef>,
) -> anyhow::Result<StreamSceneDef> {
    let kind = SceneKind::from_db(&model.kind).ok_or_else(|| {
        anyhow::anyhow!("scene {} has unrecognized kind {:?}", model.id, model.kind)
    })?;
    Ok(StreamSceneDef {
        id: model.id as i64,
        name: model.name,
        kind,
        position: model.position,
        is_active: model.is_active != 0,
        transition_ms: model.transition_ms.map(|v| v as u32),
        elements,
    })
}

fn element_def_from_model(model: stream_element::Model) -> Option<StreamElementDef> {
    match serde_json::from_str::<StreamElementProps>(&model.props) {
        Ok(props) => Some(StreamElementDef {
            id: model.id as i64,
            z_order: model.z_order,
            props,
        }),
        Err(error) => {
            tracing::warn!(
                element_id = model.id,
                %error,
                "skipping stream element with unparseable props"
            );
            None
        }
    }
}
