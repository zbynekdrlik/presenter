use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{
    bible::BibleReference,
    playlist::{MidiBinding, PlaylistEntryKind},
    AbleSetSettings, AndroidStageDisplay, AndroidStageDisplayId, BiblePassage, BibleTranslation,
    CountdownTimer, OscSettings, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId, PreachTimer,
    PresentationId, ResolumeHost, ResolumeHostId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText, StageState, TimerState, TimersState, VelocityMode, VideoSource, VideoSourceId,
};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

use crate::entities::{
    ableset_settings, android_stage_display, bible_passage, bible_translation, osc_settings,
    playlist, playlist_entry, resolume_host, slide as slide_entity, stage_state, timers,
    video_source,
};

// Shared error and helpers for repository modules

#[derive(Debug, Error)]
pub enum RepositoryError {
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
}

pub(super) fn parse_uuid(id: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(id).map_err(|_| RepositoryError::InvalidUuid(id.to_string()))
}

/// Sanitizes user input for LIKE patterns by removing SQLite LIKE metacharacters
/// (`%` and `_`). This prevents query amplification attacks where malicious
/// patterns like `%%%%` cause expensive full-table scans.
///
/// For text search in presentations and Bible passages, users don't need to
/// search for these metacharacters, so removing them has no practical impact.
pub(super) fn sanitize_like_input(input: &str) -> String {
    input.chars().filter(|c| *c != '%' && *c != '_').collect()
}

pub(super) fn to_domain_slide(model: slide_entity::Model) -> Result<Slide, RepositoryError> {
    let content = SlideContent::new(
        SlideText::new(model.worship_main)?,
        SlideText::new(model.worship_translate)?,
        SlideText::new(model.worship_stage)?,
        model.worship_group.map(SlideGroup::new),
    );
    let slide = Slide::new(model.position as u32, content)
        .with_id(SlideId::from_uuid(parse_uuid(&model.id)?));
    Ok(slide)
}
pub(super) fn to_domain_playlist_entry(
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
                presentation_name: None,
            }
        }
        other => return Err(RepositoryError::UnknownPlaylistEntryType(other.to_string())),
    };
    Ok(PlaylistEntry { id, kind })
}

pub(super) fn to_domain_playlist(
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

pub(super) fn ableset_model_to_domain(
    model: ableset_settings::Model,
) -> Result<AbleSetSettings, RepositoryError> {
    let osc_port = cast_ableset_port(model.osc_port)?;
    let http_port = cast_ableset_port(model.http_port)?;
    let prefix_length = cast_prefix_length(model.song_prefix_length)?;
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(AbleSetSettings::new(
        model.enabled,
        model.host,
        osc_port,
        http_port,
        model.library_name,
        prefix_length,
        created_at,
        updated_at,
    ))
}

fn cast_ableset_port(value: i32) -> Result<u16, RepositoryError> {
    u16::try_from(value).map_err(|_| RepositoryError::InvalidAbleSetPort(value))
}

fn cast_prefix_length(value: i32) -> Result<u8, RepositoryError> {
    u8::try_from(value).map_err(|_| RepositoryError::InvalidAbleSetPrefix(value))
}

pub(super) fn osc_model_to_domain(
    model: osc_settings::Model,
) -> Result<OscSettings, RepositoryError> {
    let mode = parse_velocity_mode(&model.velocity_mode)?;
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(OscSettings::new(
        model.enabled,
        u16::try_from(model.listen_port)
            .map_err(|_| RepositoryError::InvalidOscPort(model.listen_port))?,
        model.address_pattern,
        mode,
        created_at,
        updated_at,
    ))
}

pub(super) fn velocity_mode_to_string(_mode: VelocityMode) -> &'static str {
    // Project policy: we standardize on one-based velocity only.
    // Always persist the canonical hyphenated spelling.
    "one-based"
}

pub(super) fn parse_velocity_mode(value: &str) -> Result<VelocityMode, RepositoryError> {
    let v = value.trim().to_ascii_lowercase();
    match v.as_str() {
        "one-based" | "one_based" | "1" => Ok(VelocityMode::OneBased),
        // Previously stored zero-based values are coerced to OneBased.
        "zero-based" | "zero_based" | "0" => Ok(VelocityMode::OneBased),
        _ => Ok(VelocityMode::OneBased),
    }
}

pub(super) fn to_domain_translation(model: bible_translation::Model) -> BibleTranslation {
    let mut translation = BibleTranslation::new(model.code, model.name, model.language);
    translation = translation.with_show_in_dashboard(model.show_in_dashboard);
    if let Some(source) = model.source.clone() {
        translation = translation.with_source(source);
    }
    translation
}

pub(super) fn to_domain_passage(
    model: bible_passage::Model,
    translation: bible_translation::Model,
) -> Result<BiblePassage, RepositoryError> {
    // Prefer canonical code/number when the row contains them; fall back otherwise.
    let reference = if !model.book_code.is_empty() && model.book_number > 0 {
        BibleReference::new_with_code(
            model.book,
            model.book_code,
            model.book_number as u16,
            model.chapter as u16,
            model.verse_start as u16,
            model.verse_end as u16,
        )?
    } else {
        BibleReference::new(
            model.book,
            model.chapter as u16,
            model.verse_start as u16,
            model.verse_end as u16,
        )?
    };
    let translation = to_domain_translation(translation);
    Ok(BiblePassage::new(reference, translation, model.content))
}
pub(super) fn android_stage_display_model_to_domain(
    model: android_stage_display::Model,
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

pub(super) fn resolume_model_to_domain(
    model: resolume_host::Model,
) -> anyhow::Result<ResolumeHost> {
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

pub(super) fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle".to_string(),
        TimerState::Running => "running".to_string(),
        TimerState::Paused => "paused".to_string(),
        TimerState::Completed => "completed".to_string(),
    }
}

pub(super) fn stage_state_model_to_state(
    model: stage_state::Model,
) -> Result<StageState, RepositoryError> {
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
    let playlist_id = match model.playlist_id {
        Some(value) => Some(PlaylistId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    Ok(StageState::new(
        presentation_id,
        current_slide_id,
        next_slide_id,
        playlist_id,
    )
    .with_active_entry_index(model.active_entry_index.map(|i| i as u32)))
}

pub(super) fn timers_model_to_state(model: timers::Model) -> Result<TimersState, RepositoryError> {
    let countdown_state = parse_timer_state(&model.countdown_state)?;
    let preach_state = parse_timer_state(&model.preach_state)?;

    let countdown = CountdownTimer {
        target: model.countdown_target.into(),
        state: countdown_state,
    };

    let preach = PreachTimer::from_parts(
        preach_state,
        model.preach_started_at.map(Into::into),
        chrono::Duration::seconds(model.preach_accumulated_seconds),
        model.preach_limit_seconds.map(chrono::Duration::seconds),
    );

    Ok(TimersState::new(countdown, preach))
}

pub(super) fn parse_timer_state(value: &str) -> Result<TimerState, RepositoryError> {
    match value {
        "idle" => Ok(TimerState::Idle),
        "running" => Ok(TimerState::Running),
        "paused" => Ok(TimerState::Paused),
        "completed" => Ok(TimerState::Completed),
        other => Err(RepositoryError::UnknownTimerState(other.to_string())),
    }
}

/// Builds a worship slide entity ActiveModel from a domain Slide.
pub(super) fn build_slide_active_model(
    slide: &Slide,
    presentation_id: &str,
    position: i32,
) -> slide_entity::ActiveModel {
    use sea_orm::Set;

    slide_entity::ActiveModel {
        id: Set(slide.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        position: Set(position),
        worship_main: Set(slide.content.main.value().to_owned()),
        worship_main_search: Set(presenter_core::search::fold_query(
            slide.content.main.value(),
        )),
        worship_translate: Set(slide.content.translation.value().to_owned()),
        worship_translate_search: Set(presenter_core::search::fold_query(
            slide.content.translation.value(),
        )),
        worship_stage: Set(slide.content.stage.value().to_owned()),
        worship_stage_search: Set(presenter_core::search::fold_query(
            slide.content.stage.value(),
        )),
        worship_group: Set(slide.content.group.as_ref().map(|g| g.name().to_owned())),
        created_at: Set(chrono::Utc::now().into()),
    }
}

pub(super) const BIBLE_INSERT_CHUNK: usize = 500;

pub(super) fn video_source_model_to_domain(
    model: video_source::Model,
) -> anyhow::Result<VideoSource> {
    let id = VideoSourceId::from_uuid(
        Uuid::parse_str(&model.id).map_err(|_| anyhow!("invalid video source id"))?,
    );
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(VideoSource::new(
        id,
        model.label,
        model.ndi_name,
        model.is_active,
        created_at,
        updated_at,
    ))
}
