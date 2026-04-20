use chrono::{DateTime, Utc};
use presenter_core::{
    playlist::PlaylistEntryKind, Playlist, PlaylistId, Presentation, PresentationId, Slide,
    SlideContent, SlideId, SlideText, StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot,
    StagePlaylistEntry, TimersOverview,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub(crate) struct StageContext {
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) overview: TimersOverview,
    pub(crate) resolution: StageResolution,
    pub(crate) latency_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StageResolution {
    pub(crate) presentation_id: Option<PresentationId>,
    pub(crate) presentation_name: Option<String>,
    pub(crate) library_name: Option<String>,
    pub(crate) current_slide_id: Option<SlideId>,
    pub(crate) current: Option<StageDisplaySlide>,
    pub(crate) next_slide_id: Option<SlideId>,
    pub(crate) next: Option<StageDisplaySlide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) override_song_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_song_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_slides: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) playlist_id: Option<PlaylistId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) playlist_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) playlist_entries: Option<Vec<StagePlaylistEntry>>,
}

impl StageResolution {
    pub(crate) fn cleared() -> Self {
        Self {
            presentation_id: None,
            presentation_name: None,
            library_name: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            override_song_name: None,
            next_song_name: None,
            current_index: None,
            total_slides: None,
            playlist_id: None,
            playlist_name: None,
            playlist_entries: None,
        }
    }
}

#[derive(Clone)]
struct SlideCtx<'a> {
    slide: &'a Slide,
    effective_group: Option<String>,
}

impl<'a> SlideCtx<'a> {
    fn to_stage_display(&self) -> StageDisplaySlide {
        StageDisplaySlide {
            main: self.slide.content.main.value().to_string(),
            translation: self.slide.content.translation.value().to_string(),
            stage: self.slide.content.stage.value().to_string(),
            group: self.effective_group.clone(),
            group_color: None,
        }
    }
}

struct ResolvedSlides<'a> {
    current: Option<SlideCtx<'a>>,
    next: Option<SlideCtx<'a>>,
}

pub(crate) fn stage_resolution_from_presentation(
    presentation: &Presentation,
    library_name: Option<String>,
    current_slide_id: Option<SlideId>,
    next_slide_id: Option<SlideId>,
) -> StageResolution {
    let total_slides = presentation.slides.len() as u32;

    if presentation.slides.is_empty() {
        return StageResolution {
            presentation_id: Some(presentation.id),
            presentation_name: Some(presentation.name.clone()),
            library_name,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            override_song_name: None,
            next_song_name: None,
            current_index: None,
            total_slides: Some(total_slides),
            playlist_id: None,
            playlist_name: None,
            playlist_entries: None,
        };
    }

    let resolved = resolve_slide_positions(presentation, current_slide_id, next_slide_id);

    let current_slide_id_value = resolved.current.as_ref().map(|ctx| ctx.slide.id);
    let next_slide_id_value = resolved.next.as_ref().map(|ctx| ctx.slide.id);
    let current_slide = resolved.current.as_ref().map(|ctx| ctx.to_stage_display());
    let next_slide = resolved.next.as_ref().map(|ctx| ctx.to_stage_display());

    let current_index_value = resolved
        .current
        .as_ref()
        .and_then(|ctx| {
            presentation
                .slides
                .iter()
                .position(|slide| slide.id == ctx.slide.id)
        })
        .map(|index| index as u32 + 1);

    StageResolution {
        presentation_id: Some(presentation.id),
        presentation_name: Some(presentation.name.clone()),
        library_name,
        current_slide_id: current_slide_id_value,
        current: current_slide,
        next_slide_id: next_slide_id_value,
        next: next_slide,
        override_song_name: None,
        next_song_name: None,
        current_index: current_index_value,
        total_slides: Some(total_slides),
        playlist_id: None,
        playlist_name: None,
        playlist_entries: None,
    }
}

fn resolve_slide_positions<'a>(
    presentation: &'a Presentation,
    current_slide_id: Option<SlideId>,
    next_slide_id: Option<SlideId>,
) -> ResolvedSlides<'a> {
    let mut effective_group: Option<String> = None;
    let mut first: Option<SlideCtx<'a>> = None;
    let mut second: Option<SlideCtx<'a>> = None;
    let mut current_ctx: Option<SlideCtx<'a>> = None;
    let mut current_order: Option<u32> = None;
    let mut next_by_id: Option<SlideCtx<'a>> = None;
    let mut next_after_current: Option<SlideCtx<'a>> = None;

    for slide in &presentation.slides {
        if let Some(group) = slide.content.group.as_ref() {
            effective_group = Some(group.name().to_string());
        }
        let ctx = SlideCtx {
            slide,
            effective_group: effective_group.clone(),
        };
        if first.is_none() {
            first = Some(ctx.clone());
        } else if second.is_none() {
            second = Some(ctx.clone());
        }

        if let Some(target_next) = next_slide_id {
            if slide.id == target_next {
                next_by_id = Some(ctx.clone());
            }
        }

        if current_ctx.is_none() {
            if let Some(target_current) = current_slide_id {
                if slide.id == target_current {
                    current_order = Some(slide.order);
                    current_ctx = Some(ctx.clone());
                }
            }
        } else if next_after_current.is_none() {
            if let Some(order) = current_order {
                if slide.order > order {
                    next_after_current = Some(ctx.clone());
                }
            }
        }
    }

    let resolved_current = current_ctx.or_else(|| first.clone());
    let resolved_next = if let Some(next_ctx) = next_by_id {
        Some(next_ctx)
    } else if current_order.is_some() {
        next_after_current
    } else {
        second
    };

    ResolvedSlides {
        current: resolved_current,
        next: resolved_next,
    }
}

pub(crate) fn build_stage_snapshot(
    layout: StageDisplayLayout,
    context: &StageContext,
) -> StageDisplaySnapshot {
    StageDisplaySnapshot::new(
        layout,
        context.generated_at,
        context.resolution.presentation_id,
        context.resolution.presentation_name.clone(),
        context.resolution.library_name.clone(),
        context
            .resolution
            .override_song_name
            .clone()
            .or_else(|| context.resolution.presentation_name.clone()),
        context
            .resolution
            .override_song_name
            .as_deref()
            .or(context.resolution.presentation_name.as_deref())
            .and_then(extract_song_number),
        context.resolution.next_song_name.clone(),
        context.resolution.current_slide_id,
        context.resolution.current.clone(),
        context.resolution.next_slide_id,
        context.resolution.next.clone(),
        context.overview.clone(),
        context.latency_ms,
        context.resolution.current_index,
        context.resolution.total_slides,
        context.resolution.playlist_id,
        context.resolution.playlist_name.clone(),
        context.resolution.playlist_entries.clone(),
    )
}

fn has_song_number_prefix(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
}

pub(crate) fn extract_song_number(name: &str) -> Option<String> {
    let trimmed = name.trim_start();
    if has_song_number_prefix(trimmed.as_bytes()) {
        Some(trimmed[..3].to_string())
    } else {
        None
    }
}

pub(crate) fn sanitize_song_title(name: &str) -> String {
    let trimmed = name.trim_start();
    if has_song_number_prefix(trimmed.as_bytes()) {
        trimmed[4..].trim_start().to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn blank_slide_content() -> SlideContent {
    // Empty strings are always within limits, so these unwrap_or_else calls are safe fallbacks
    let main = SlideText::new("").unwrap_or_else(|_| {
        // This should never happen as empty strings are valid
        SlideText::new("").unwrap_or_else(|_| unreachable!("empty string should always be valid"))
    });
    let translation = SlideText::new("").unwrap_or_else(|_| {
        SlideText::new("").unwrap_or_else(|_| unreachable!("empty string should always be valid"))
    });
    let stage = SlideText::new("").unwrap_or_else(|_| {
        SlideText::new("").unwrap_or_else(|_| unreachable!("empty string should always be valid"))
    });
    SlideContent::new(main, translation, stage, None)
}

pub(crate) fn build_stage_playlist_entries(
    playlist: &Playlist,
    active_presentation_id: Option<PresentationId>,
    name_lookup: &std::collections::HashMap<PresentationId, String>,
) -> Vec<StagePlaylistEntry> {
    playlist
        .entries
        .iter()
        .map(|entry| match &entry.kind {
            PlaylistEntryKind::Presentation {
                presentation_id, ..
            } => {
                let is_active = active_presentation_id == Some(*presentation_id);
                let raw_name = name_lookup
                    .get(presentation_id)
                    .cloned()
                    .unwrap_or_default();
                StagePlaylistEntry {
                    name: sanitize_song_title(&raw_name),
                    presentation_id: Some(*presentation_id),
                    is_active,
                    entry_type: "presentation".to_string(),
                }
            }
            PlaylistEntryKind::Separator { name } => StagePlaylistEntry {
                name: name.clone(),
                presentation_id: None,
                is_active: false,
                entry_type: "separator".to_string(),
            },
        })
        .collect()
}

pub(crate) fn format_countdown_text(seconds_remaining: i64) -> String {
    let total = seconds_remaining.max(0);
    if total < 60 {
        total.to_string()
    } else {
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_song_number_returns_prefix_for_numbered_songs() {
        assert_eq!(
            extract_song_number("042 Amazing Grace"),
            Some("042".to_string())
        );
        assert_eq!(
            extract_song_number("001 First Song"),
            Some("001".to_string())
        );
        assert_eq!(
            extract_song_number("115 Last Song"),
            Some("115".to_string())
        );
    }

    #[test]
    fn extract_song_number_returns_none_for_unnumbered_songs() {
        assert_eq!(extract_song_number("Amazing Grace"), None);
        assert_eq!(extract_song_number(""), None);
        assert_eq!(extract_song_number("12 Two Digit"), None);
        assert_eq!(extract_song_number("1 One Digit"), None);
    }

    #[test]
    fn extract_song_number_handles_leading_whitespace() {
        assert_eq!(extract_song_number("  042 Song"), Some("042".to_string()));
    }

    #[test]
    fn sanitize_song_title_strips_number_prefix() {
        assert_eq!(sanitize_song_title("042 Amazing Grace"), "Amazing Grace");
        assert_eq!(
            sanitize_song_title("Song Without Number"),
            "Song Without Number"
        );
    }
}
