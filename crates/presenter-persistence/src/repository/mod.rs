mod ableset;
mod android_stage;
mod app_settings;
mod bible;
mod libraries;
mod osc;
mod playlists;
mod presentations;
mod resolume;
mod search;
mod stage;
mod timer_state;
mod util;

use crate::entities::{
    ableset_settings, android_stage_display as android_stage_display_entity, bible_passage,
    bible_translation, library, osc_settings, playlist, playlist_entry, playlist_favorite,
    presentation as presentation_entity, resolume_host, slide as slide_entity, stage_state, timers,
};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Duration, Utc};
use presenter_core::{
    bible::BibleIngestionBatch,
    playlist::{MidiBinding, PlaylistEntryKind},
    search::{fold_query, query_tokens},
    AbleSetSettings, AbleSetSettingsDraft, AndroidStageDisplay, AndroidStageDisplayDraft,
    AndroidStageDisplayId, BiblePassage, BibleReference, BibleTranslation, CountdownTimer,
    LibraryId, OscSettings, OscSettingsDraft, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId,
    PreachTimer, Presentation, PresentationId, ResolumeHost, ResolumeHostDraft, ResolumeHostId,
    SearchMatchField, SearchResult, SearchResultKind, Slide, SlideContent, SlideGroup, SlideId,
    SlideText, StageState, TimerState, TimersState, VelocityMode,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, Database, DatabaseConnection,
    EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Schema,
    Set, TransactionTrait,
};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};
use thiserror::Error;
use tracing::instrument;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Repository {
    db: DatabaseConnection,
}

const TIMERS_SINGLETON_ID: &str = "timers";
const STAGE_STATE_SINGLETON_ID: &str = "stage-state";
const OSC_SETTINGS_SINGLETON_ID: &str = "osc";
const ABLESET_SETTINGS_SINGLETON_ID: &str = "ableset";

#[derive(Debug, Clone)]
pub struct DatabaseSettings {
    pub url: String,
}

impl DatabaseSettings {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn connect(settings: &DatabaseSettings) -> anyhow::Result<Self> {
        let db = Database::connect(settings.url.as_str())
            .await
            .with_context(|| format!("failed to connect to database at {}", settings.url))?;
        Self::migrate(&db).await?;
        Ok(Self { db })
    }

    #[instrument(skip_all)]
    pub async fn connect_in_memory() -> anyhow::Result<Self> {
        let db = Database::connect("sqlite::memory:?cache=shared")
            .await
            .context("failed to start in-memory sqlite")?;
        Self::migrate(&db).await?;
        Ok(Self { db })
    }

    async fn migrate(db: &DatabaseConnection) -> anyhow::Result<()> {
        Migrator::up(db, None).await?;
        Ok(())
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    // bible ingestion moved to bible.rs

    // bible list translations moved to bible.rs

    // bible search moved to bible.rs

    // bible-specific functions moved to bible.rs

    // osc + ableset moved to osc.rs and ableset.rs

    // resolume list moved to resolume.rs

    // android stage moved to android_stage.rs

    // resolume create moved to resolume.rs

    // android stage create moved to android_stage.rs

    // resolume update moved to resolume.rs

    // android stage update moved to android_stage.rs

    // resolume + android deletion moved to resolume.rs and android_stage.rs

    // stage state moved to stage.rs

    // stage state moved to stage.rs

    // timers moved to timer_state.rs

    // timers moved to timer_state.rs
}

#[derive(Debug, Error)]
enum RepositoryError {
    #[error("invalid uuid stored in database: {0}")]
    InvalidUuid(String),
    #[error(transparent)]
    Domain(#[from] presenter_core::presentation::PresentationError),
    #[error(transparent)]
    Slide(#[from] presenter_core::slide::SlideValidationError),
    #[error(transparent)]
    Library(#[from] presenter_core::library::LibraryError),
    #[error(transparent)]
    Reference(#[from] presenter_core::bible::BibleReferenceError),
    #[error(transparent)]
    Playlist(#[from] presenter_core::playlist::PlaylistError),
    #[error("unknown timer state '{0}' in persistence layer")]
    UnknownTimerState(String),
    #[error("unknown playlist entry type '{0}' in persistence layer")]
    UnknownPlaylistEntryType(String),
    #[error("osc port {0} out of range")]
    InvalidOscPort(i32),
    #[error("ableset port {0} out of range")]
    InvalidAbleSetPort(i32),
    #[error("ableset song prefix length {0} out of range")]
    InvalidAbleSetPrefix(i32),
    #[error("unknown osc velocity mode '{0}' in persistence layer")]
    UnknownOscVelocityMode(String),
}

fn parse_uuid(id: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(id).map_err(|_| RepositoryError::InvalidUuid(id.to_string()))
}

fn to_domain_slide(model: slide_entity::Model) -> Result<Slide, RepositoryError> {
    let content = SlideContent::new(
        SlideText::new(model.main_text)?,
        SlideText::new(model.translation_text)?,
        SlideText::new(model.stage_text)?,
        model.group_name.map(SlideGroup::new),
    );
    let slide = Slide::new(model.position as u32, content)
        .with_id(SlideId::from_uuid(parse_uuid(&model.id)?));
    Ok(slide)
}

fn to_domain_playlist_entry(
    model: playlist_entry::Model,
) -> Result<PlaylistEntry, RepositoryError> {
    let id = PlaylistEntryId::from_uuid(parse_uuid(&model.id)?);
    let entry_type = model.entry_type.to_lowercase();
    let kind = match entry_type.as_str() {
        "separator" => {
            let name = model
                .label
                .unwrap_or_else(|| "Separator".to_string())
                .trim()
                .to_string();
            PlaylistEntryKind::Separator { name }
        }
        "presentation" | "song" | "item" | "default" | "" => {
            let presentation_id = model
                .presentation_id
                .as_ref()
                .ok_or_else(|| RepositoryError::UnknownPlaylistEntryType(entry_type.clone()))
                .and_then(|id| Ok(PresentationId::from_uuid(parse_uuid(id)?)))?;
            let midi_binding = match model.midi_note {
                Some(note) => {
                    if !(0..=127).contains(&note) {
                        return Err(presenter_core::playlist::PlaylistError::MidiOutOfRange(
                            note.clamp(0, 127) as u8,
                        )
                        .into());
                    }
                    Some(MidiBinding::new(note as u8)?)
                }
                None => None,
            };
            PlaylistEntryKind::Presentation {
                presentation_id,
                midi_binding,
            }
        }
        other => return Err(RepositoryError::UnknownPlaylistEntryType(other.to_string())),
    };
    Ok(PlaylistEntry { id, kind })
}

fn to_domain_playlist(
    favorite_ids: &HashSet<String>,
    model: playlist::Model,
    entries: Vec<PlaylistEntry>,
) -> Result<Playlist, RepositoryError> {
    let id = PlaylistId::from_uuid(parse_uuid(&model.id)?);
    let show_in_dashboard = favorite_ids.contains(&model.id);
    Ok(Playlist::from_parts(
        id,
        model.name,
        show_in_dashboard,
        entries,
    )?)
}

fn ableset_model_to_domain(
    model: ableset_settings::Model,
) -> Result<AbleSetSettings, RepositoryError> {
    let osc_port = cast_ableset_port(model.osc_port)?;
    let http_port = cast_ableset_port(model.http_port)?;
    let prefix_length = cast_prefix_length(model.song_prefix_length)?;
    Ok(AbleSetSettings::new(
        model.enabled,
        model.host,
        osc_port,
        http_port,
        model.library_name,
        prefix_length,
        model.created_at.into(),
        model.updated_at.into(),
    ))
}

fn cast_ableset_port(value: i32) -> Result<u16, RepositoryError> {
    if value <= 0 || value > u16::MAX as i32 {
        return Err(RepositoryError::InvalidAbleSetPort(value));
    }
    Ok(value as u16)
}

fn cast_prefix_length(value: i32) -> Result<u8, RepositoryError> {
    if value <= 0 || value > u8::MAX as i32 {
        return Err(RepositoryError::InvalidAbleSetPrefix(value));
    }
    Ok(value as u8)
}

fn osc_model_to_domain(model: osc_settings::Model) -> Result<OscSettings, RepositoryError> {
    let mode = parse_velocity_mode(&model.velocity_mode)?;
    let port_i32 = model.listen_port;
    if port_i32 <= 0 || port_i32 > u16::MAX as i32 {
        return Err(RepositoryError::InvalidOscPort(port_i32));
    }
    let port = port_i32 as u16;
    Ok(OscSettings::new(
        model.enabled,
        port,
        model.address_pattern,
        mode,
        model.created_at.into(),
        model.updated_at.into(),
    ))
}

fn velocity_mode_to_string(mode: VelocityMode) -> &'static str {
    match mode {
        VelocityMode::ZeroBased => "zero_based",
        VelocityMode::OneBased => "one_based",
    }
}

fn parse_velocity_mode(value: &str) -> Result<VelocityMode, RepositoryError> {
    match value.to_lowercase().as_str() {
        "zero_based" | "zero-based" | "zero" | "0" => Ok(VelocityMode::ZeroBased),
        "one_based" | "one-based" | "one" | "1" => Ok(VelocityMode::OneBased),
        other => Err(RepositoryError::UnknownOscVelocityMode(other.to_string())),
    }
}

fn to_domain_translation(model: bible_translation::Model) -> BibleTranslation {
    let mut translation = BibleTranslation::new(
        model.code.clone(),
        model.name.clone(),
        model.language.clone(),
    );
    if let Some(source) = model.source.clone() {
        translation = translation.with_source(source);
    }
    translation
}

fn to_domain_passage(
    model: bible_passage::Model,
    translation: bible_translation::Model,
) -> Result<BiblePassage, RepositoryError> {
    let reference = if let (Some(code), Some(number)) =
        (Some(model.book_code.clone()), Some(model.book_number))
    {
        if !code.is_empty() && number > 0 {
            BibleReference::new_with_code(
                model.book.clone(),
                code,
                number as u16,
                model.chapter as u16,
                model.verse_start as u16,
                model.verse_end as u16,
            )?
        } else {
            BibleReference::new(
                model.book.clone(),
                model.chapter as u16,
                model.verse_start as u16,
                model.verse_end as u16,
            )?
        }
    } else {
        BibleReference::new(
            model.book.clone(),
            model.chapter as u16,
            model.verse_start as u16,
            model.verse_end as u16,
        )?
    };
    let translation = to_domain_translation(translation);
    Ok(BiblePassage::new(reference, translation, model.content))
}

fn android_stage_display_model_to_domain(
    model: android_stage_display_entity::Model,
) -> anyhow::Result<AndroidStageDisplay> {
    let id = AndroidStageDisplayId::from_uuid(
        Uuid::parse_str(&model.id).map_err(|_| anyhow!("invalid android stage display id"))?,
    );
    let port = u16::try_from(model.port)
        .map_err(|_| anyhow!("android stage display port out of range"))?;
    if port == 0 {
        return Err(anyhow!("android stage display port cannot be zero"));
    }
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(AndroidStageDisplay::new(
        id,
        model.label,
        model.host,
        port,
        model.launch_component,
        model.is_enabled,
        created_at,
        updated_at,
    ))
}

fn resolume_model_to_domain(model: resolume_host::Model) -> anyhow::Result<ResolumeHost> {
    let id = ResolumeHostId::from_uuid(
        Uuid::parse_str(&model.id).map_err(|_| anyhow!("invalid resolume host id"))?,
    );
    let port = u16::try_from(model.port).map_err(|_| anyhow!("resolume host port out of range"))?;
    if port == 0 {
        return Err(anyhow!("resolume host port cannot be zero"));
    }
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(ResolumeHost::new(
        id,
        model.label,
        model.host,
        port,
        model.is_enabled,
        created_at,
        updated_at,
    ))
}

fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle".to_string(),
        TimerState::Running => "running".to_string(),
        TimerState::Paused => "paused".to_string(),
        TimerState::Completed => "completed".to_string(),
    }
}

fn stage_state_model_to_state(model: stage_state::Model) -> Result<StageState, RepositoryError> {
    let presentation_id = match model.presentation_id {
        Some(value) => Some(PresentationId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    let current_slide_id = match model.current_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    let next_slide_id = match model.next_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    Ok(StageState::new(
        presentation_id,
        current_slide_id,
        next_slide_id,
    ))
}

fn timers_model_to_state(model: timers::Model) -> Result<TimersState, RepositoryError> {
    let countdown_state = parse_timer_state(&model.countdown_state)?;
    let preach_state = parse_timer_state(&model.preach_state)?;

    let countdown = CountdownTimer {
        target: model.countdown_target.into(),
        state: countdown_state,
    };

    let preach = PreachTimer::from_parts(
        preach_state,
        model.preach_started_at.map(Into::into),
        Duration::seconds(model.preach_accumulated_seconds),
    );

    Ok(TimersState::new(countdown, preach))
}

fn parse_timer_state(value: &str) -> Result<TimerState, RepositoryError> {
    match value {
        "idle" => Ok(TimerState::Idle),
        "running" => Ok(TimerState::Running),
        "paused" => Ok(TimerState::Paused),
        "completed" => Ok(TimerState::Completed),
        other => Err(RepositoryError::UnknownTimerState(other.to_string())),
    }
}

const BIBLE_INSERT_CHUNK: usize = 500;

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{
        bible::BibleIngestionBatch, playlist::PlaylistEntryKind, BiblePassage, BibleReference,
        BibleTranslation, Library, PlaylistEntryId, ResolumeHostDraft, SearchResultKind,
        StageState, DEFAULT_ADB_PORT, DEFAULT_LAUNCH_COMPONENT,
    };
    fn sample_library() -> Library {
        let presentation = Presentation::new(
            "Welcome",
            vec![
                Slide::new(
                    0,
                    SlideContent::new(
                        SlideText::new("Nádej v Pánovi").unwrap(),
                        SlideText::new("Hope in the Lord").unwrap(),
                        SlideText::new("Stage").unwrap(),
                        Some(SlideGroup::new("Intro")),
                    ),
                )
                .with_id(SlideId::new()),
                Slide::new(
                    1,
                    SlideContent::new(
                        SlideText::new("Second").unwrap(),
                        SlideText::new("Druga").unwrap(),
                        SlideText::new("Stage 2").unwrap(),
                        None,
                    ),
                )
                .with_id(SlideId::new()),
            ],
        )
        .unwrap()
        .with_id(PresentationId::new());
        Library::new("Default", vec![presentation])
            .unwrap()
            .with_id(LibraryId::new())
    }

    #[tokio::test]
    async fn round_trip_library() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();
        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Default");
        assert_eq!(libraries[0].presentations.len(), 1);
        assert_eq!(libraries[0].presentations[0].slides.len(), 2);
    }

    #[tokio::test]
    async fn fetch_first_presentation_detail_returns_sample() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();

        let detail = repo.fetch_first_presentation_detail().await.unwrap();
        assert!(detail.is_some());
        let (_, name, presentation) = detail.unwrap();
        assert_eq!(name, "Default");
        assert_eq!(presentation.slides.len(), 2);
    }

    #[tokio::test]
    async fn update_slide_content_persists_changes() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        let slide = presentation.slides[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let new_content = SlideContent::new(
            SlideText::new("Updated main").unwrap(),
            SlideText::new("Updated translation").unwrap(),
            SlideText::new("Updated stage").unwrap(),
            Some(SlideGroup::new("Updated Group")),
        );

        repo.update_slide_content(presentation.id, slide.id, &new_content)
            .await
            .unwrap();

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("presentation detail");
        let updated = detail
            .2
            .slides
            .iter()
            .find(|candidate| candidate.id == slide.id)
            .expect("slide present");

        assert_eq!(updated.content.main.value(), "Updated main");
        assert_eq!(updated.content.translation.value(), "Updated translation");
        assert_eq!(updated.content.stage.value(), "Updated stage");
        assert_eq!(
            updated.content.group.as_ref().map(|group| group.name()),
            Some("Updated Group")
        );
    }

    #[tokio::test]
    async fn purge_presentation_content_clears_tables() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();

        let presentation = &library.presentations[0];
        let current = presentation.slides[0].id;
        let state = StageState::new(Some(presentation.id), Some(current), None);
        repo.upsert_stage_state(&state).await.unwrap();

        repo.purge_presentation_content().await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert!(libraries.is_empty());
        let stage = repo.get_stage_state().await.unwrap();
        assert!(stage.is_none());
    }

    #[tokio::test]
    async fn bible_translation_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let translation = BibleTranslation::new("en-kjv", "King James Version", "en")
            .with_source("https://example.org/kjv");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference.clone(),
            translation.clone(),
            "For God so loved the world".to_string(),
        );
        let batch = BibleIngestionBatch::new(translation.clone(), vec![passage.clone()]).unwrap();
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let translations = repo.list_bible_translations().await.unwrap();
        assert_eq!(translations.len(), 1);
        assert_eq!(translations[0], translation);

        let fetched = repo
            .find_bible_passage(&translation.code, &reference)
            .await
            .unwrap()
            .expect("passage to exist");
        assert_eq!(fetched.translation, translation);
        assert_eq!(fetched.reference, reference);
        assert_eq!(fetched.text, passage.text);
    }

    #[tokio::test]
    async fn bible_translation_large_batch_is_persisted() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let translation = BibleTranslation::new("en-load", "Load Test", "en");
        let mut passages = Vec::new();
        for verse in 1..=1_200u16 {
            let reference = BibleReference::new("Psalm", 119, verse, verse).unwrap();
            let passage =
                BiblePassage::new(reference, translation.clone(), format!("Verse {verse}"));
            passages.push(passage);
        }

        let batch = BibleIngestionBatch::new(translation.clone(), passages).unwrap();
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let tail_reference = BibleReference::new("Psalm", 119, 1_200, 1_200).unwrap();
        let tail = repo
            .find_bible_passage(&translation.code, &tail_reference)
            .await
            .unwrap();
        assert!(tail.is_some());

        let head_reference = BibleReference::new("Psalm", 119, 1, 1).unwrap();
        let head = repo
            .find_bible_passage(&translation.code, &head_reference)
            .await
            .unwrap();
        assert!(head.is_some());
    }

    #[tokio::test]
    async fn osc_settings_default_is_seeded() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let settings = repo.get_osc_settings().await.unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.listen_port, 39051);
        assert_eq!(settings.address_pattern, "/note");
        assert_eq!(settings.velocity_mode, VelocityMode::OneBased);
    }

    #[tokio::test]
    async fn osc_settings_upsert_updates_values() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let draft = OscSettingsDraft {
            enabled: true,
            listen_port: 10023,
            address_pattern: "/presenter/trigger".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let updated = repo.upsert_osc_settings(&draft).await.unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.listen_port, 10023);
        assert_eq!(updated.address_pattern, "/presenter/trigger");
        assert_eq!(updated.velocity_mode, VelocityMode::OneBased);

        // upsert idempotency
        let second = repo.get_osc_settings().await.unwrap();
        assert_eq!(second.address_pattern, updated.address_pattern);
        assert_eq!(second.listen_port, updated.listen_port);
    }

    #[tokio::test]
    async fn resolume_host_crud_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let draft = ResolumeHostDraft::new("Arena", "resolume.lan", 8090);
        let created = repo.create_resolume_host(&draft).await.unwrap();
        assert_eq!(created.label, "Arena");
        assert_eq!(created.host, "resolume.lan");
        assert_eq!(created.port, 8090);
        assert!(created.is_enabled);

        let hosts = repo.list_resolume_hosts().await.unwrap();
        assert_eq!(hosts.len(), 1);

        let updated_draft =
            ResolumeHostDraft::new("Arena North", "resolume.lan", 8090).with_enabled(false);
        let updated = repo
            .update_resolume_host(created.id, &updated_draft)
            .await
            .unwrap();
        assert_eq!(updated.label, "Arena North");
        assert!(!updated.is_enabled);

        repo.delete_resolume_host(created.id).await.unwrap();
        let after_delete = repo.list_resolume_hosts().await.unwrap();
        assert!(after_delete.is_empty());
    }

    #[tokio::test]
    async fn android_stage_display_crud_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let draft = AndroidStageDisplayDraft::new("Stage Left", "sd1l.lan");
        let created = repo.create_android_stage_display(&draft).await.unwrap();
        assert_eq!(created.label, "Stage Left");
        assert_eq!(created.host, "sd1l.lan");
        assert_eq!(created.port, DEFAULT_ADB_PORT);
        assert_eq!(
            created.launch_component,
            DEFAULT_LAUNCH_COMPONENT.to_string()
        );
        assert!(created.is_enabled);

        let displays = repo.list_android_stage_displays().await.unwrap();
        assert_eq!(displays.len(), 1);

        let updated_draft = AndroidStageDisplayDraft::new("Stage Right", "sd2l.lan")
            .with_port(5566)
            .with_launch_component("com.example/.Main")
            .with_enabled(false);
        let updated = repo
            .update_android_stage_display(created.id, &updated_draft)
            .await
            .unwrap();
        assert_eq!(updated.label, "Stage Right");
        assert_eq!(updated.host, "sd2l.lan");
        assert_eq!(updated.port, 5566);
        assert_eq!(updated.launch_component, "com.example/.Main");
        assert!(!updated.is_enabled);

        repo.delete_android_stage_display(created.id).await.unwrap();
        let after_delete = repo.list_android_stage_displays().await.unwrap();
        assert!(after_delete.is_empty());
    }

    #[tokio::test]
    async fn timers_state_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let now = Utc::now();
        let target = now + Duration::minutes(10);
        let countdown = CountdownTimer {
            target,
            state: TimerState::Running,
        };
        let mut preach = PreachTimer::new();
        preach.start(now);
        let pause_instant = now + Duration::minutes(1);
        preach.pause(pause_instant);
        let expected_preach = preach.clone();
        let state = TimersState::new(countdown, preach);

        repo.upsert_timers_state(&state).await.unwrap();

        let stored = repo.get_timers_state().await.unwrap().unwrap();
        assert_eq!(stored.countdown.state, TimerState::Running);
        assert_eq!(stored.countdown.target, target);
        assert_eq!(stored.preach.state, expected_preach.state);
        assert!(stored.preach.elapsed(pause_instant).num_seconds() >= 60);
    }

    #[tokio::test]
    async fn playlist_round_trip_with_separator() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let playlist = repo
            .create_playlist("Good Fest Friday", true)
            .await
            .unwrap();

        let entries = vec![
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Separator {
                    name: "Doors Open".to_string(),
                },
            },
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: presentation.id,
                    midi_binding: None,
                },
            },
        ];

        repo.replace_playlist_entries(playlist.id, &entries)
            .await
            .unwrap();

        let playlists = repo.list_playlists().await.unwrap();
        assert_eq!(playlists.len(), 1);

        let fetched = &playlists[0];
        assert_eq!(fetched.name, "Good Fest Friday");
        assert!(fetched.show_in_dashboard);
        assert_eq!(fetched.entries.len(), 2);
        assert!(matches!(
            fetched.entries[0].kind,
            PlaylistEntryKind::Separator { .. }
        ));
        assert!(matches!(
            fetched.entries[1].kind,
            PlaylistEntryKind::Presentation {
                presentation_id,
                ..
            } if presentation_id == presentation.id
        ));
    }

    #[tokio::test]
    async fn create_library_persists_new_row() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let created = repo.create_library("Autotest Library").await.unwrap();
        assert_eq!(created.name, "Autotest Library");
        assert!(created.presentations.is_empty());

        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id, created.id);
        assert_eq!(libraries[0].name, "Autotest Library");
    }

    #[tokio::test]
    async fn rename_library_updates_name() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let created = repo.create_library("Original").await.unwrap();

        repo.rename_library(created.id, "Renamed").await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Renamed");
    }

    #[tokio::test]
    async fn delete_library_removes_presentations_and_slides() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        repo.delete_library(library.id).await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert!(libraries.is_empty());

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap();
        assert!(detail.is_none());
    }

    #[tokio::test]
    async fn replace_presentation_slides_overwrites_rows() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let mut slides = presentation.slides.clone();
        let new_slide = Slide::new(
            slides.len() as u32,
            SlideContent::new(
                SlideText::new("New main").unwrap(),
                SlideText::new("New translation").unwrap(),
                SlideText::new("New stage").unwrap(),
                None,
            ),
        );
        slides.push(new_slide);
        for (index, slide) in slides.iter_mut().enumerate() {
            slide.order = index as u32;
        }

        repo.replace_presentation_slides(presentation.id, &slides)
            .await
            .unwrap();

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("detail");
        assert_eq!(detail.2.slides.len(), slides.len());
        assert_eq!(
            detail.2.slides.last().unwrap().content.main.value(),
            "New main",
        );
    }

    #[tokio::test]
    async fn search_presenter_returns_hits() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let mut library = sample_library();
        library.name = "Chvály Nedeľa".to_string();
        repo.upsert_library(&library).await.unwrap();

        let mut other = sample_library();
        other.name = "Youth Set".to_string();
        repo.upsert_library(&other).await.unwrap();

        let accent_slide = Slide::new(
            0,
            SlideContent::new(
                SlideText::new("Ježiš, ja Ťa chválim").unwrap(),
                SlideText::new("Jesus I praise you").unwrap(),
                SlideText::new("Stage cue").unwrap(),
                None,
            ),
        );
        let accent_presentation = Presentation::new("Ježiš, ja", vec![accent_slide]).unwrap();
        let accent_library = Library::new("TYMY Worship", vec![accent_presentation]).unwrap();
        repo.upsert_library(&accent_library).await.unwrap();

        let results = repo.search_presenter("Chvaly", 10).await.unwrap();
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Library)
                && result.library_name == "Chvály Nedeľa"));

        let presentation_results = repo.search_presenter("Welcome", 10).await.unwrap();
        assert!(presentation_results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Presentation)));

        let slide_results = repo.search_presenter("Nadej", 10).await.unwrap();
        assert!(slide_results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Slide)));

        let compound_results = repo.search_presenter("jezis, ja tymy", 10).await.unwrap();
        assert!(compound_results.iter().any(|result| {
            matches!(
                result.kind,
                SearchResultKind::Slide | SearchResultKind::Presentation
            ) && result.library_name == "TYMY Worship"
        }));
    }

    #[tokio::test]
    async fn stage_state_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let current = presentation.slides[0].id;
        let next = presentation.slides.get(1).map(|slide| slide.id);
        let state = StageState::new(Some(presentation.id), Some(current), next);

        repo.upsert_stage_state(&state).await.unwrap();

        let stored = repo.get_stage_state().await.unwrap().unwrap();
        assert_eq!(stored.presentation_id, Some(presentation.id));
        assert_eq!(stored.current_slide_id, Some(current));
        assert_eq!(stored.next_slide_id, next);
    }
}
