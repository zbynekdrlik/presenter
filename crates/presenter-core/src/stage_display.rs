use crate::{
    slide::{ResolvedSlide, Slide as DomainSlide},
    PresentationId, SlideId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default stage layout code used across the application.
pub const DEFAULT_STAGE_LAYOUT_CODE: &str = "worship-snv";

/// Built-in stage display layouts exposed by the Presenter server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplayLayout {
    pub code: String,
    pub name: String,
    pub description: String,
}

impl StageDisplayLayout {
    /// Returns the canonical layouts expected by the stage display endpoints.
    pub fn built_in() -> Vec<Self> {
        vec![
            Self::new(
                DEFAULT_STAGE_LAYOUT_CODE,
                "WORSHIP SNV",
                "Lyrics current/next line with group labels",
            ),
            Self::new(
                "worship-pp",
                "WORSHIP PP",
                "Lyrics view plus presentation overview sidebar",
            ),
            Self::new(
                "timer",
                "TIMER",
                "Countdown emphasis for service start cues",
            ),
            Self::new(
                "preach",
                "PREACH",
                "Stopwatch view for preacher with overtime indicator",
            ),
        ]
    }

    fn new(code: &str, name: &str, description: &str) -> Self {
        Self {
            code: code.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplaySlide {
    pub main: String,
    pub translation: String,
    pub stage: String,
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplaySnapshot {
    pub layout: StageDisplayLayout,
    pub generated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<PresentationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_slide_id: Option<SlideId>,
    pub current: Option<StageDisplaySlide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_slide_id: Option<SlideId>,
    pub next: Option<StageDisplaySlide>,
    pub timers: crate::timer::TimersOverview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_position: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_slides: Option<u32>,
}

impl From<&DomainSlide> for StageDisplaySlide {
    fn from(slide: &DomainSlide) -> Self {
        let content = &slide.content;
        Self {
            main: content.main.value().to_string(),
            translation: content.translation.value().to_string(),
            stage: content.stage.value().to_string(),
            group: content.group.as_ref().map(|g| g.name().to_string()),
        }
    }
}

impl From<&ResolvedSlide> for StageDisplaySlide {
    fn from(slide: &ResolvedSlide) -> Self {
        Self {
            main: slide.main.value().to_string(),
            translation: slide.translation.value().to_string(),
            stage: slide.stage.value().to_string(),
            group: slide
                .effective_group
                .as_ref()
                .map(|group| group.name().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StageState {
    pub presentation_id: Option<PresentationId>,
    pub current_slide_id: Option<SlideId>,
    pub next_slide_id: Option<SlideId>,
}

impl StageState {
    pub fn new(
        presentation_id: Option<PresentationId>,
        current_slide_id: Option<SlideId>,
        next_slide_id: Option<SlideId>,
    ) -> Self {
        Self {
            presentation_id,
            current_slide_id,
            next_slide_id,
        }
    }

    pub fn cleared() -> Self {
        Self::new(None, None, None)
    }
}

impl StageDisplaySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        layout: StageDisplayLayout,
        generated_at: DateTime<Utc>,
        presentation_id: Option<PresentationId>,
        presentation_name: Option<String>,
        library_name: Option<String>,
        song_name: Option<String>,
        current_slide_id: Option<SlideId>,
        current: Option<StageDisplaySlide>,
        next_slide_id: Option<SlideId>,
        next: Option<StageDisplaySlide>,
        timers: crate::timer::TimersOverview,
        latency_ms: Option<f64>,
        current_position: Option<u32>,
        total_slides: Option<u32>,
    ) -> Self {
        Self {
            layout,
            generated_at,
            presentation_id,
            presentation_name,
            library_name,
            song_name,
            current_slide_id,
            current,
            next_slide_id,
            next,
            timers,
            latency_ms,
            current_position,
            total_slides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_layouts_cover_expected_variants() {
        let layouts = StageDisplayLayout::built_in();
        assert_eq!(layouts.len(), 4);
        let codes: Vec<_> = layouts.iter().map(|layout| layout.code.as_str()).collect();
        assert!(codes.contains(&DEFAULT_STAGE_LAYOUT_CODE));
        assert!(codes.contains(&"worship-pp"));
        assert!(codes.contains(&"timer"));
        assert!(codes.contains(&"preach"));
    }
}
