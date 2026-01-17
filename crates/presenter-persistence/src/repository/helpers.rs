use crate::entities::{
    ableset_settings, android_stage_display, bible_passage, bible_translation, osc_settings,
    playlist, playlist_entry, resolume_host, slide as slide_entity, stage_state, timers,
};
use anyhow::anyhow;
use chrono::{DateTime, Duration, Utc};
use presenter_core::{
    playlist::{MidiBinding, PlaylistEntryKind},
    slide::SlideMetadata,
    AbleSetSettings, AndroidStageDisplay, AndroidStageDisplayId, BiblePassage, BibleReference,
    BibleTranslation, CountdownTimer, OscSettings, Playlist, PlaylistEntry, PlaylistEntryId,
    PlaylistId, PreachTimer, PresentationId, ResolumeHost, ResolumeHostId, Slide, SlideContent,
    SlideGroup, SlideId, SlideText, StageState, TimerState, TimersState, VelocityMode,
};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

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
    #[error("unknown osc velocity mode '{0}' in persistence layer")]
    UnknownOscVelocityMode(String),
}

pub fn parse_uuid(id: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(id).map_err(|_| RepositoryError::InvalidUuid(id.to_string()))
}

pub fn to_domain_slide(model: slide_entity::Model) -> Result<Slide, RepositoryError> {
    let content = SlideContent::new(
        SlideText::new(model.main_text)?,
        SlideText::new(model.translation_text)?,
        SlideText::new(model.stage_text)?,
        model.group_name.map(SlideGroup::new),
    );
    let mut slide = Slide::new(model.position as u32, content)
        .with_id(SlideId::from_uuid(parse_uuid(&model.id)?));
    if let Some(raw) = model.metadata_json.as_deref() {
        if !raw.trim().is_empty() {
            if let Ok(meta) = serde_json::from_str::<SlideMetadata>(raw) {
                slide = slide.with_metadata(Some(meta));
            }
        }
    }
    Ok(slide)
}

pub fn to_domain_playlist_entry(
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

pub fn to_domain_playlist(
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

pub fn ableset_model_to_domain(
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

pub fn osc_model_to_domain(model: osc_settings::Model) -> Result<OscSettings, RepositoryError> {
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

pub fn velocity_mode_to_string(mode: VelocityMode) -> &'static str {
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

pub fn to_domain_translation(model: bible_translation::Model) -> BibleTranslation {
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

pub fn to_domain_passage(
    model: bible_passage::Model,
    translation: bible_translation::Model,
) -> Result<BiblePassage, RepositoryError> {
    let reference = BibleReference::new(
        model.book,
        model.chapter as u16,
        model.verse_start as u16,
        model.verse_end as u16,
    )?;
    let translation = to_domain_translation(translation);
    Ok(BiblePassage::new(reference, translation, model.content))
}

pub fn android_stage_display_model_to_domain(
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

pub fn resolume_model_to_domain(model: resolume_host::Model) -> anyhow::Result<ResolumeHost> {
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

pub fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle".to_string(),
        TimerState::Running => "running".to_string(),
        TimerState::Paused => "paused".to_string(),
        TimerState::Completed => "completed".to_string(),
    }
}

pub fn stage_state_model_to_state(
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
    Ok(StageState::new(
        presentation_id,
        current_slide_id,
        next_slide_id,
    ))
}

pub fn timers_model_to_state(model: timers::Model) -> Result<TimersState, RepositoryError> {
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
