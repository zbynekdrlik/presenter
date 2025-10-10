use std::collections::HashMap;

use chrono::{DateTime, Utc};
use presenter_core::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSongSnapshot,
    LibrarySummary, PresentationId,
};

use crate::ableset::AbleSetStatusSnapshot;

use super::AppState;

#[derive(Default)]
pub(crate) struct AbleSetLibraryCache {
    pub(super) library_name: Option<String>,
    pub(super) song_prefix_length: u8,
    pub(super) entries: HashMap<String, PresentationId>,
    pub(super) last_updated: Option<DateTime<Utc>>,
    pub(super) last_error: Option<String>,
}

impl AbleSetLibraryCache {
    pub(super) fn invalidate(&mut self) {
        self.entries.clear();
        self.library_name = None;
        self.song_prefix_length = 0;
        self.last_updated = None;
        self.last_error = None;
    }

    pub(super) fn matches(&self, library_name: &str, prefix_len: u8) -> bool {
        if let Some(current) = &self.library_name {
            current.eq_ignore_ascii_case(library_name) && self.song_prefix_length == prefix_len
        } else {
            false
        }
    }
}

impl AppState {
    pub async fn ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
        self.repository.get_ableset_settings().await
    }

    pub async fn update_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
    ) -> anyhow::Result<AbleSetSettings> {
        let settings = self.repository.upsert_ableset_settings(&draft).await?;
        self.ableset_bridge.apply_settings(settings.clone()).await?;
        {
            let mut cache = self.ableset_cache.write().await;
            cache.invalidate();
            cache.library_name = None;
            cache.song_prefix_length = settings.song_prefix_length;
        }
        Ok(settings)
    }

    pub async fn ableset_status_snapshot(&self) -> AbleSetStatusSnapshot {
        self.ableset_bridge.status_snapshot().await
    }

    pub async fn set_ableset_follow(&self, enabled: bool) -> AbleSetStatusSnapshot {
        self.ableset_bridge.set_follow_enabled(enabled).await
    }

    pub async fn current_ableset_song(&self) -> Option<AbleSetSongSnapshot> {
        self.ableset_bridge.song_snapshot().await
    }

    pub async fn resolve_ableset_presentation(
        &self,
        prefix: &str,
    ) -> anyhow::Result<Option<PresentationId>> {
        let key = prefix.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let settings = self.ableset_bridge.status_snapshot().await;
        if !settings.enabled {
            return Ok(None);
        }
        self.ensure_ableset_cache(&settings).await?;
        let lookup = key.to_ascii_lowercase();
        let cache = self.ableset_cache.read().await;
        Ok(cache.entries.get(&lookup).copied())
    }

    pub(super) async fn ensure_ableset_cache(
        &self,
        settings: &AbleSetStatusSnapshot,
    ) -> anyhow::Result<()> {
        let needs_refresh = {
            let cache = self.ableset_cache.read().await;
            !cache.matches(&settings.library_name, settings.song_prefix_length)
                || cache.entries.is_empty()
        };
        if needs_refresh {
            self.refresh_ableset_cache(settings).await?;
        }
        Ok(())
    }

    pub(super) async fn refresh_ableset_cache(
        &self,
        settings: &AbleSetStatusSnapshot,
    ) -> anyhow::Result<()> {
        let summaries = self.repository.list_library_summaries(None).await?;
        let target = summaries.into_iter().find(|summary: &LibrarySummary| {
            summary.name.eq_ignore_ascii_case(&settings.library_name)
        });
        let mut cache = self.ableset_cache.write().await;
        cache.library_name = Some(settings.library_name.clone());
        cache.song_prefix_length = settings.song_prefix_length;
        cache.entries.clear();
        cache.last_updated = Some(Utc::now());
        cache.last_error = None;
        if let Some(summary) = target {
            for presentation in summary.presentations {
                if let Some(prefix) =
                    extract_song_prefix(&presentation.name, settings.song_prefix_length)
                {
                    cache
                        .entries
                        .insert(prefix.to_ascii_lowercase(), presentation.id);
                }
            }
            if cache.entries.is_empty() {
                cache.last_error = Some("no presentations with valid prefix".to_string());
            }
        } else {
            cache.last_error = Some("library not found".to_string());
        }
        Ok(())
    }
}
