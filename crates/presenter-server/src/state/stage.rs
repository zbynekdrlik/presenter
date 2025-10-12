use std::collections::HashMap;

use chrono::{DateTime, Utc};
use presenter_core::{
    Presentation, PresentationId, Slide, SlideContent, SlideId, SlideText, StageDisplayLayout,
    StageDisplaySlide, StageDisplaySnapshot, StageState, TimersOverview,
};
use serde::{Deserialize, Serialize};

use crate::{live::LiveEvent, resolume::StageUpdate};

use super::AppState;

#[derive(Debug, Clone)]
pub(crate) struct StageContext {
    pub(super) generated_at: DateTime<Utc>,
    pub(super) overview: TimersOverview,
    pub(super) resolution: StageResolution,
    pub(super) latency_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StageResolution {
    pub(super) presentation_id: Option<PresentationId>,
    pub(super) presentation_name: Option<String>,
    pub(super) library_name: Option<String>,
    pub(super) current_slide_id: Option<SlideId>,
    pub(super) current: Option<StageDisplaySlide>,
    pub(super) next_slide_id: Option<SlideId>,
    pub(super) next: Option<StageDisplaySlide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) current_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_slides: Option<u32>,
}

impl StageResolution {
    pub(super) fn cleared() -> Self {
        Self {
            presentation_id: None,
            presentation_name: None,
            library_name: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            current_index: None,
            total_slides: None,
        }
    }
}

impl AppState {
    pub async fn stage_display_snapshot(
        &self,
        layout_code: &str,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == layout_code);
        let Some(layout) = layout else {
            return Ok(None);
        };
        let Some(context) = self.build_stage_context().await? else {
            return Ok(None);
        };
        Ok(Some(build_stage_snapshot(layout, &context)))
    }

    pub async fn selected_stage_display_snapshot(
        &self,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let code = {
            let guard = self.stage_layout.read().await;
            guard.clone()
        };
        self.stage_display_snapshot(&code).await
    }

    pub async fn stage_layout_code(&self) -> String {
        self.stage_layout.read().await.clone()
    }

    pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == code)
            .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
        {
            let mut guard = self.stage_layout.write().await;
            if *guard == layout.code {
                return Ok(layout);
            }
            *guard = layout.code.clone();
        }
        self.live_hub.publish(LiveEvent::StageLayout {
            code: layout.code.clone(),
        });
        self.broadcast_stage_snapshots().await?;
        Ok(layout)
    }

    // Keep async signature to match other AppState accessors used in tests/handlers.
    #[allow(clippy::unused_async)]
    pub async fn stage_displays(&self) -> anyhow::Result<Vec<StageDisplayLayout>> {
        Ok(StageDisplayLayout::built_in())
    }

    pub async fn update_stage_state(
        &self,
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
    ) -> anyhow::Result<()> {
        let Some((_, library_name, presentation)) =
            self.presentation_detail(presentation_id).await?
        else {
            anyhow::bail!("presentation not found");
        };

        if !presentation
            .slides
            .iter()
            .any(|slide| slide.id == current_slide_id)
        {
            anyhow::bail!("current slide not found in presentation");
        }

        if let Some(next_slide_id) = next_slide_id {
            if !presentation
                .slides
                .iter()
                .any(|slide| slide.id == next_slide_id)
            {
                anyhow::bail!("next slide not found in presentation");
            }
        }

        let stage_state =
            StageState::new(Some(presentation_id), Some(current_slide_id), next_slide_id);
        self.repository.upsert_stage_state(&stage_state).await?;
        let resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            Some(current_slide_id),
            next_slide_id,
        );
        self.broadcast_stage_resolution(resolution).await?;
        Ok(())
    }

    pub async fn clear_stage(&self) -> anyhow::Result<()> {
        let cleared = StageState::cleared();
        self.repository.upsert_stage_state(&cleared).await?;
        self.broadcast_stage_resolution(StageResolution::cleared())
            .await?;
        Ok(())
    }

    pub(super) async fn reconcile_stage_state_after_edit(
        &self,
        presentation_id: PresentationId,
        slides: &[Slide],
    ) -> anyhow::Result<()> {
        let Some(mut state) = self.repository.get_stage_state().await? else {
            return Ok(());
        };
        if state.presentation_id != Some(presentation_id) {
            return Ok(());
        }

        if slides.is_empty() {
            if state.current_slide_id.is_some() || state.next_slide_id.is_some() {
                state.current_slide_id = None;
                state.next_slide_id = None;
                self.repository.upsert_stage_state(&state).await?;
            }
            return Ok(());
        }

        let mut changed = false;
        let contains = |id: Option<SlideId>| {
            id.map_or(true, |target| slides.iter().any(|slide| slide.id == target))
        };
        if !contains(state.current_slide_id) {
            state.current_slide_id = Some(slides[0].id);
            state.next_slide_id = slides.get(1).map(|slide| slide.id);
            changed = true;
        } else if !contains(state.next_slide_id) {
            if let Some(current) = state.current_slide_id {
                if let Some(position) = slides.iter().position(|slide| slide.id == current) {
                    state.next_slide_id = slides.get(position + 1).map(|slide| slide.id);
                } else {
                    state.next_slide_id = slides.get(1).map(|slide| slide.id);
                }
            } else {
                state.next_slide_id = slides.get(1).map(|slide| slide.id);
            }
            changed = true;
        }

        if changed {
            self.repository.upsert_stage_state(&state).await?;
        }
        Ok(())
    }

    pub(super) async fn broadcast_stage_snapshots(&self) -> anyhow::Result<()> {
        let Some(context) = self.build_stage_context().await? else {
            return Ok(());
        };
        self.publish_stage_context(&context).await?;
        Ok(())
    }

    pub(super) async fn broadcast_stage_resolution(
        &self,
        resolution: StageResolution,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let latency_ms = self.sample_resolume_latency().await;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
            latency_ms,
        };
        self.publish_stage_context(&context).await?;
        let current_main = context
            .resolution
            .current
            .as_ref()
            .map_or_else(String::new, |slide| slide.main.clone());
        let current_translation = context
            .resolution
            .current
            .as_ref()
            .map_or_else(String::new, |slide| slide.translation.clone());
        let song_name = context
            .resolution
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name))
            .unwrap_or_default();
        let band_name = context.resolution.library_name.clone().unwrap_or_default();
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
            song_name: Some(song_name),
            band_name: Some(band_name),
        };
        self.resolume_client.stage_update(stage_update).await;
        Ok(())
    }

    pub(super) async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
        let mut layouts = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| (layout.code.clone(), layout))
            .collect::<HashMap<_, _>>();
        let Some(layout) = layouts
            .remove(&code)
            .or_else(|| layouts.remove("worship-snv"))
        else {
            return Ok(());
        };
        let snapshot = build_stage_snapshot(layout, context);
        self.publish_stage_update(snapshot);
        Ok(())
    }

    pub(super) async fn build_stage_context(&self) -> anyhow::Result<Option<StageContext>> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let overview = timers_state.overview(now);
        let stage_state = self.repository.get_stage_state().await?;

        let resolution = if let Some(state) = stage_state {
            match self.resolve_stage_from_state(&state).await? {
                Some(resolution) => resolution,
                None => match self.resolve_default_stage().await? {
                    Some(resolution) => resolution,
                    None => return Ok(None),
                },
            }
        } else if let Some(resolution) = self.resolve_default_stage().await? {
            resolution
        } else {
            return Ok(None);
        };

        let latency_ms = self.sample_resolume_latency().await;
        Ok(Some(StageContext {
            generated_at: now,
            overview,
            resolution,
            latency_ms,
        }))
    }

    pub(super) async fn resolve_stage_from_state(
        &self,
        stage_state: &StageState,
    ) -> anyhow::Result<Option<StageResolution>> {
        let Some(presentation_id) = stage_state.presentation_id else {
            return Ok(Some(StageResolution::cleared()));
        };
        let detail = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?;
        let Some((_, library_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            stage_state.current_slide_id,
            stage_state.next_slide_id,
        );
        Ok(Some(resolution))
    }

    pub(super) async fn resolve_default_stage(&self) -> anyhow::Result<Option<StageResolution>> {
        let detail = self.repository.fetch_first_presentation_detail().await?;
        let Some((_, library_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution =
            stage_resolution_from_presentation(&presentation, Some(library_name), None, None);
        Ok(Some(resolution))
    }

    pub(super) async fn sample_resolume_latency(&self) -> Option<f64> {
        let snapshot = self.resolume_client.snapshot().await;
        snapshot
            .values()
            .filter_map(|status| status.last_latency_ms)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn publish_stage_update(&self, snapshot: StageDisplaySnapshot) {
        self.live_hub.publish(LiveEvent::Stage { snapshot });
    }
}

pub(crate) fn stage_resolution_from_presentation(
    presentation: &Presentation,
    library_name: Option<String>,
    current_slide_id: Option<SlideId>,
    next_slide_id: Option<SlideId>,
) -> StageResolution {
    #[derive(Clone)]
    struct SlideCtx<'a> {
        slide: &'a Slide,
        effective_group: Option<String>,
    }

    fn to_stage_display(ctx: &SlideCtx<'_>) -> StageDisplaySlide {
        StageDisplaySlide {
            main: ctx.slide.content.main.value().to_string(),
            translation: ctx.slide.content.translation.value().to_string(),
            stage: ctx.slide.content.stage.value().to_string(),
            group: ctx.effective_group.clone(),
        }
    }

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
            current_index: None,
            total_slides: Some(total_slides),
        };
    }

    let mut effective_group: Option<String> = None;
    let mut first: Option<SlideCtx<'_>> = None;
    let mut second: Option<SlideCtx<'_>> = None;
    let mut current_ctx: Option<SlideCtx<'_>> = None;
    let mut current_order: Option<u32> = None;
    let mut next_by_id: Option<SlideCtx<'_>> = None;
    let mut next_after_current: Option<SlideCtx<'_>> = None;

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
        next_after_current.clone()
    } else {
        second.clone()
    };

    let current_slide_id_value = resolved_current.as_ref().map(|ctx| ctx.slide.id);
    let next_slide_id_value = resolved_next.as_ref().map(|ctx| ctx.slide.id);
    let current_slide = resolved_current.as_ref().map(to_stage_display);
    let next_slide = resolved_next.as_ref().map(to_stage_display);

    let current_index_value = resolved_current
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
        current_index: current_index_value,
        total_slides: Some(total_slides),
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
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name)),
        context.resolution.current_slide_id,
        context.resolution.current.clone(),
        context.resolution.next_slide_id,
        context.resolution.next.clone(),
        context.overview.clone(),
        context.latency_ms,
        context.resolution.current_index,
        context.resolution.total_slides,
    )
}

pub(crate) fn sanitize_song_title(name: &str) -> String {
    let trimmed = name.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
    {
        let remainder = trimmed[4..].trim_start();
        remainder.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn blank_slide_content() -> SlideContent {
    SlideContent::new(
        SlideText::new("").expect("empty main within limit"),
        SlideText::new("").expect("empty translation within limit"),
        SlideText::new("").expect("empty stage within limit"),
        None,
    )
}
