use crate::entities::{playlist, playlist_entry, playlist_favorite};
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{playlist::PlaylistEntryKind, Playlist, PlaylistId};
use sea_orm::{
    sea_query::Expr, sea_query::OnConflict, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use std::collections::HashSet;
use tracing::instrument;
use uuid::Uuid;

use super::util::{parse_uuid, to_domain_playlist, to_domain_playlist_entry, RepositoryError};
use super::Repository;

impl Repository {
    #[instrument(skip_all)]
    pub async fn list_playlists(&self) -> anyhow::Result<Vec<Playlist>> {
        let models = playlist::Entity::find()
            .order_by_asc(playlist::Column::CreatedAt)
            .all(&self.db)
            .await?;
        let favorites = playlist_favorite::Entity::find().all(&self.db).await?;
        let favorite_ids: HashSet<_> = favorites.into_iter().map(|fav| fav.playlist_id).collect();
        let mut playlists = Vec::with_capacity(models.len());
        for model in models {
            let entries_models = playlist_entry::Entity::find()
                .filter(playlist_entry::Column::PlaylistId.eq(model.id.clone()))
                .order_by_asc(playlist_entry::Column::Position)
                .all(&self.db)
                .await?;
            let entries = entries_models
                .into_iter()
                .map(to_domain_playlist_entry)
                .collect::<Result<Vec<_>, RepositoryError>>()?;
            let playlist = to_domain_playlist(&favorite_ids, model, entries)?;
            playlists.push(playlist);
        }
        Ok(playlists)
    }

    #[instrument(skip_all)]
    pub async fn create_playlist(
        &self,
        name: &str,
        show_in_dashboard: bool,
    ) -> anyhow::Result<Playlist> {
        let id = Uuid::new_v4();
        let playlist_id = PlaylistId::from_uuid(id);
        let active = playlist::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            created_at: Set(Utc::now().into()),
        };
        playlist::Entity::insert(active).exec(&self.db).await?;
        if show_in_dashboard {
            self.set_playlist_favorite_internal(playlist_id, true)
                .await?;
        }
        let playlist =
            Playlist::from_parts(playlist_id, name.to_string(), show_in_dashboard, vec![])?;
        Ok(playlist)
    }

    #[instrument(skip_all)]
    pub async fn rename_playlist(&self, playlist_id: PlaylistId, name: &str) -> anyhow::Result<()> {
        playlist::Entity::update_many()
            .col_expr(playlist::Column::Name, Expr::value(name))
            .filter(playlist::Column::Id.eq(playlist_id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn list_playlist_favorites(&self) -> anyhow::Result<Vec<PlaylistId>> {
        let favorites = playlist_favorite::Entity::find().all(&self.db).await?;
        let mut ids = Vec::with_capacity(favorites.len());
        for fav in favorites {
            let uuid = parse_uuid(&fav.playlist_id)?;
            ids.push(PlaylistId::from_uuid(uuid));
        }
        Ok(ids)
    }

    #[instrument(skip_all)]
    pub async fn set_playlist_favorite(
        &self,
        playlist_id: PlaylistId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        self.ensure_playlist_exists(playlist_id).await?;
        self.set_playlist_favorite_internal(playlist_id, favorite)
            .await
    }

    pub(super) async fn set_playlist_favorite_internal(
        &self,
        playlist_id: PlaylistId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let id_string = playlist_id.to_string();
        if favorite {
            playlist_favorite::Entity::insert(playlist_favorite::ActiveModel {
                playlist_id: Set(id_string.clone()),
            })
            .on_conflict(
                OnConflict::column(playlist_favorite::Column::PlaylistId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        } else {
            playlist_favorite::Entity::delete_by_id(id_string)
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    async fn ensure_playlist_exists(&self, playlist_id: PlaylistId) -> anyhow::Result<()> {
        let count = playlist::Entity::find()
            .filter(playlist::Column::Id.eq(playlist_id.to_string()))
            .count(&self.db)
            .await?;
        if count > 0 {
            Ok(())
        } else {
            Err(anyhow!("playlist not found"))
        }
    }

    #[instrument(skip_all)]
    pub async fn delete_playlist(&self, playlist_id: PlaylistId) -> anyhow::Result<()> {
        playlist::Entity::delete_by_id(playlist_id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_playlist_entries(
        &self,
        playlist_id: PlaylistId,
        entries: &[presenter_core::PlaylistEntry],
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        playlist_entry::Entity::delete_many()
            .filter(playlist_entry::Column::PlaylistId.eq(playlist_id.to_string()))
            .exec(&txn)
            .await?;

        for (index, entry) in entries.iter().enumerate() {
            let (entry_type, presentation_id, midi_note, label) = match &entry.kind {
                PlaylistEntryKind::Presentation {
                    presentation_id,
                    midi_binding,
                } => (
                    "presentation".to_string(),
                    Some(presentation_id.to_string()),
                    midi_binding.map(|binding| binding.note() as i32),
                    None,
                ),
                PlaylistEntryKind::Separator { name } => {
                    ("separator".to_string(), None, None, Some(name.to_string()))
                }
            };
            let entry_id = entry.id.into_uuid();
            let active = playlist_entry::ActiveModel {
                id: Set(entry_id.to_string()),
                playlist_id: Set(playlist_id.to_string()),
                entry_type: Set(entry_type),
                presentation_id: Set(presentation_id),
                position: Set(index as i32),
                midi_note: Set(midi_note),
                label: Set(label),
            };
            playlist_entry::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;
        Ok(())
    }
}
