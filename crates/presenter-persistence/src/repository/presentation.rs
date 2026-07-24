use super::util::{build_slide_active_model, parse_uuid, to_domain_slide, RepositoryError};
use super::Repository;
use crate::entities::{presentation as presentation_entity, slide as slide_entity};
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{
    search::fold_query, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideId,
};
use sea_orm::{
    sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use tracing::instrument;

/// Bump a presentation's `updated_at` to now, on any connection. Every local song
/// mutation calls this so LWW sync has a monotone clock (#555).
async fn touch_presentation<C: sea_orm::ConnectionTrait>(
    conn: &C,
    presentation_id: &str,
) -> anyhow::Result<()> {
    presentation_entity::Entity::update_many()
        .col_expr(
            presentation_entity::Column::UpdatedAt,
            Expr::value(Utc::now().to_rfc3339()),
        )
        .filter(presentation_entity::Column::Id.eq(presentation_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Write a slide's worship-text columns on `conn` (any connection/transaction).
/// Shared by `update_slide_content` and `update_slide_content_with_metadata` so
/// the atomicity fix (#558 S6 — caller wraps this + `touch_presentation` in one
/// transaction) lives in exactly one place.
async fn update_slide_worship_columns<C: sea_orm::ConnectionTrait>(
    conn: &C,
    presentation_id: PresentationId,
    slide_id: SlideId,
    content: &SlideContent,
) -> anyhow::Result<()> {
    let result = slide_entity::Entity::update_many()
        .col_expr(
            slide_entity::Column::WorshipMain,
            Expr::value(content.main.value().to_owned()),
        )
        .col_expr(
            slide_entity::Column::WorshipMainSearch,
            Expr::value(fold_query(content.main.value())),
        )
        .col_expr(
            slide_entity::Column::WorshipTranslate,
            Expr::value(content.translation.value().to_owned()),
        )
        .col_expr(
            slide_entity::Column::WorshipTranslateSearch,
            Expr::value(fold_query(content.translation.value())),
        )
        .col_expr(
            slide_entity::Column::WorshipStage,
            Expr::value(content.stage.value().to_owned()),
        )
        .col_expr(
            slide_entity::Column::WorshipStageSearch,
            Expr::value(fold_query(content.stage.value())),
        )
        .col_expr(
            slide_entity::Column::WorshipGroup,
            Expr::value(content.group.as_ref().map(|group| group.name().to_owned())),
        )
        .filter(slide_entity::Column::Id.eq(slide_id.to_string()))
        .filter(slide_entity::Column::PresentationId.eq(presentation_id.to_string()))
        .exec(conn)
        .await?;

    if result.rows_affected == 0 {
        return Err(anyhow!(
            "slide {} not found in presentation {}",
            slide_id,
            presentation_id
        ));
    }
    Ok(())
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn create_presentation(
        &self,
        library_id: LibraryId,
        name: &str,
        slides: Option<&[Slide]>,
    ) -> anyhow::Result<(LibraryId, String, Presentation)> {
        let presentation_uuid = uuid::Uuid::new_v4();
        let library_uuid = library_id.to_string();
        let txn = self.db.begin().await?;

        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(presentation_uuid.to_string()),
            library_id: Set(library_uuid.clone()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(None),
        })
        .exec(&txn)
        .await?;

        let slide_list: Vec<Slide> = match slides {
            Some(s) if !s.is_empty() => s.to_vec(),
            _ => vec![Slide::new(
                0,
                SlideContent::new(
                    presenter_core::SlideText::new("")?,
                    presenter_core::SlideText::new("")?,
                    presenter_core::SlideText::new("")?,
                    None,
                ),
            )],
        };

        for (index, slide) in slide_list.iter().enumerate() {
            let pres_id_str = presentation_uuid.to_string();
            let active = build_slide_active_model(slide, &pres_id_str, index as i32);
            slide_entity::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;

        let detail = self
            .fetch_presentation_detail(PresentationId::from_uuid(presentation_uuid))
            .await?;

        detail.ok_or_else(|| anyhow!("failed to load newly created presentation"))
    }

    /// #570: deep-copy a presentation into another (or the same) library.
    /// The copy gets a NEW presentation id, a NEW sync_id (it syncs as its
    /// own song under #555 LWW — never pairs with the original), and NEW
    /// slide ids; per-slide stage-layout markers (#515) are remapped onto
    /// the fresh slide ids. The original is not touched, so playlists
    /// referencing it are unaffected.
    #[instrument(skip_all)]
    pub async fn copy_presentation_to_library(
        &self,
        presentation_id: PresentationId,
        target_library_id: LibraryId,
    ) -> anyhow::Result<(LibraryId, String, Presentation)> {
        let source_id = presentation_id.to_string();
        let target_lib = target_library_id.to_string();

        if crate::entities::library::Entity::find_by_id(target_lib.clone())
            .one(&self.db)
            .await?
            .is_none()
        {
            return Err(anyhow!("target library not found"));
        }

        let source = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::Id.eq(source_id.clone()))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("presentation not found"))?;

        let source_slides = slide_entity::Entity::find()
            .filter(slide_entity::Column::PresentationId.eq(source_id.clone()))
            .order_by_asc(slide_entity::Column::Position)
            .all(&self.db)
            .await?;

        let markers = crate::entities::slide_stage_layout::Entity::find()
            .filter(
                crate::entities::slide_stage_layout::Column::PresentationId.eq(source_id.clone()),
            )
            .all(&self.db)
            .await?;

        let new_uuid = uuid::Uuid::new_v4();
        let new_id_str = new_uuid.to_string();
        let txn = self.db.begin().await?;

        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(new_id_str.clone()),
            library_id: Set(target_lib),
            name: Set(source.name.clone()),
            search_name: Set(fold_query(&source.name)),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(None),
        })
        .exec(&txn)
        .await?;

        for (index, model) in source_slides.into_iter().enumerate() {
            let old_slide_id = model.id.clone();
            let fresh = Slide::new(index as u32, to_domain_slide(model)?.content);
            let active = build_slide_active_model(&fresh, &new_id_str, index as i32);
            slide_entity::Entity::insert(active).exec(&txn).await?;

            if let Some(marker) = markers.iter().find(|m| m.slide_id == old_slide_id) {
                crate::entities::slide_stage_layout::Entity::insert(
                    crate::entities::slide_stage_layout::ActiveModel {
                        slide_id: Set(fresh.id.to_string()),
                        presentation_id: Set(new_id_str.clone()),
                        layout_code: Set(marker.layout_code.clone()),
                    },
                )
                .exec(&txn)
                .await?;
            }
        }

        txn.commit().await?;

        let detail = self
            .fetch_presentation_detail(PresentationId::from_uuid(new_uuid))
            .await?;
        detail.ok_or_else(|| anyhow!("failed to load copied presentation"))
    }

    #[instrument(skip_all)]
    pub async fn rename_presentation(
        &self,
        presentation_id: PresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("presentation name cannot be empty"));
        }
        let id = presentation_id.to_string();
        // #558 S6: the rename and its updated_at bump are ONE transaction — two
        // separate autocommit statements let a concurrent sync apply read the
        // OLD (not-yet-bumped) updated_at between them, "win" the LWW race, and
        // overwrite this edit; the touch would then land AFTER and stamp a
        // bogus "locally newer" timestamp over the peer's content.
        let txn = self.db.begin().await?;
        let result = presentation_entity::Entity::update_many()
            .col_expr(presentation_entity::Column::Name, Expr::value(trimmed))
            .col_expr(
                presentation_entity::Column::SearchName,
                Expr::value(fold_query(trimmed)),
            )
            .filter(presentation_entity::Column::Id.eq(id.clone()))
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("presentation not found"));
        }
        touch_presentation(&txn, &id).await?;
        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_presentation(&self, presentation_id: PresentationId) -> anyhow::Result<()> {
        use crate::entities::playlist_entry;
        let id = presentation_id.to_string();
        // #555: delete is now a SOFT delete (the trash) so it syncs like any
        // edit under LWW and stays restorable for 30 days. One transaction —
        // the mark, the playlist-entry removal, and the stage-layout cleanup
        // commit (or fail) together (#515).
        let txn = self.db.begin().await?;

        let now = Utc::now().to_rfc3339();
        let result = presentation_entity::Entity::update_many()
            .col_expr(
                presentation_entity::Column::DeletedAt,
                Expr::value(now.clone()),
            )
            .col_expr(presentation_entity::Column::UpdatedAt, Expr::value(now))
            .filter(presentation_entity::Column::Id.eq(id.clone()))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("presentation not found"));
        }

        // Preserve today's behavior: a deleted song leaves every playlist.
        playlist_entry::Entity::delete_many()
            .filter(playlist_entry::Column::PresentationId.eq(id.clone()))
            .exec(&txn)
            .await?;

        // #515 markers go with the (now-hidden) song.
        crate::entities::slide_stage_layout::Entity::delete_many()
            .filter(crate::entities::slide_stage_layout::Column::PresentationId.eq(id))
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn purge_presentation_content(&self) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        slide_entity::Entity::delete_many().exec(&txn).await?;
        // #515: purging every slide purges every stage-layout marker with it.
        crate::entities::slide_stage_layout::Entity::delete_many()
            .exec(&txn)
            .await?;
        presentation_entity::Entity::delete_many()
            .exec(&txn)
            .await?;
        crate::entities::library::Entity::delete_many()
            .exec(&txn)
            .await?;
        crate::entities::stage_state::Entity::delete_by_id(
            super::STAGE_STATE_SINGLETON_ID.to_string(),
        )
        .exec(&txn)
        .await?;

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn fetch_presentation_detail(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Option<(LibraryId, String, Presentation)>> {
        let pres_model = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::Id.eq(presentation_id.to_string()))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .one(&self.db)
            .await?;
        let Some(pres_model) = pres_model else {
            return Ok(None);
        };

        let slides = slide_entity::Entity::find()
            .filter(slide_entity::Column::PresentationId.eq(pres_model.id.clone()))
            .order_by_asc(slide_entity::Column::Position)
            .all(&self.db)
            .await?;

        let slide_models = slides
            .into_iter()
            .map(to_domain_slide)
            .collect::<Result<Vec<_>, RepositoryError>>()?;

        let presentation = Presentation::new(pres_model.name.clone(), slide_models)?
            .with_id(PresentationId::from_uuid(parse_uuid(&pres_model.id)?));

        let library_id = LibraryId::from_uuid(parse_uuid(&pres_model.library_id)?);
        let library_name =
            crate::entities::library::Entity::find_by_id(pres_model.library_id.clone())
                .one(&self.db)
                .await?
                .map(|lib| lib.name)
                .unwrap_or_default();

        Ok(Some((library_id, library_name, presentation)))
    }

    #[instrument(skip_all)]
    pub async fn fetch_first_presentation_detail(
        &self,
    ) -> anyhow::Result<Option<(LibraryId, String, Presentation)>> {
        let presentation = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .order_by_asc(presentation_entity::Column::CreatedAt)
            .one(&self.db)
            .await?;
        let Some(model) = presentation else {
            return Ok(None);
        };
        let presentation_id = PresentationId::from_uuid(parse_uuid(&model.id)?);
        self.fetch_presentation_detail(presentation_id).await
    }

    #[instrument(skip_all)]
    pub async fn update_slide_content(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        content: &SlideContent,
    ) -> anyhow::Result<()> {
        // #558 S6: the slide-content write and its updated_at bump are ONE
        // transaction — two separate autocommit statements let a concurrent
        // sync apply read the OLD (not-yet-bumped) updated_at between them,
        // "win" the LWW race, and overwrite this edit; the touch would then
        // land AFTER and stamp a bogus "locally newer" timestamp over the
        // peer's content.
        let txn = self.db.begin().await?;
        update_slide_worship_columns(&txn, presentation_id, slide_id, content).await?;
        touch_presentation(&txn, &presentation_id.to_string()).await?;
        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn update_slide_content_with_metadata(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        content: &SlideContent,
        _metadata: Option<&presenter_core::slide::SlideMetadata>,
    ) -> anyhow::Result<()> {
        // Worship slides no longer carry metadata — bible slides live in a separate table.
        // The metadata parameter is accepted for API compatibility but ignored.
        // #558 S6: same mutation+touch atomicity as update_slide_content.
        let txn = self.db.begin().await?;
        update_slide_worship_columns(&txn, presentation_id, slide_id, content).await?;
        touch_presentation(&txn, &presentation_id.to_string()).await?;
        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_presentation_slides(
        &self,
        presentation_id: PresentationId,
        slides: &[Slide],
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        slide_entity::Entity::delete_many()
            .filter(slide_entity::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(&txn)
            .await?;

        for (index, slide) in slides.iter().enumerate() {
            let pres_id_str = presentation_id.to_string();
            let active = build_slide_active_model(slide, &pres_id_str, index as i32);
            slide_entity::Entity::insert(active).exec(&txn).await?;
        }

        touch_presentation(&txn, &presentation_id.to_string()).await?;
        txn.commit().await?;
        Ok(())
    }
}
