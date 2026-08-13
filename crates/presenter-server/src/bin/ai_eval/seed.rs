//! Build a fresh, isolated `AppState` for one corpus case: real bible
//! translations for bible-authoring/adversarial cases (via the REAL
//! `AppState::refresh_default_bible_translations`, never synthetic
//! content), then `setup.seed` worship libraries / bible presentations,
//! then `setup.priorTurns` as literal `ChatMessage`s.
//!
//! Every case gets its OWN fresh `AppState::in_memory()` — no
//! cross-case state leakage, matching `setup.seed`'s framing as state
//! established BEFORE this one turn.

use crate::corpus::{Case, SeedBiblePresentation, SeedLibrary, SeedSlide, Setup};
use anyhow::Context;
use presenter_core::slide::{SlideContent, SlideGroup, SlideText};
use presenter_core::{BiblePresentationSlide, BibleSlideId, Slide};
use presenter_server::ai::ChatMessage;
use presenter_server::state::AppState;

/// Build the fresh `AppState` for `case`: bible-translation ingestion (when
/// the slice needs it) + `setup.seed`.
pub async fn build_state_for_case(case: &Case) -> anyhow::Result<AppState> {
    let state = AppState::in_memory()
        .await
        .context("building fresh in-memory AppState")?;

    if matches!(case.slice.as_str(), "bible-authoring" | "adversarial") {
        state
            .refresh_default_bible_translations()
            .await
            .with_context(|| {
                format!(
                    "case {}: ingesting real bible translations (needs network access)",
                    case.id
                )
            })?;
    }

    if let Some(setup) = &case.setup {
        if let Some(seed) = &setup.seed {
            seed_worship_libraries(&state, &seed.libraries)
                .await
                .with_context(|| format!("case {}: seeding worship libraries", case.id))?;
            seed_bible_presentations(&state, &seed.bible_presentations)
                .await
                .with_context(|| format!("case {}: seeding bible presentations", case.id))?;
        }
    }

    Ok(state)
}

/// Build the conversation history that must exist BEFORE `run_agent` is
/// called for this case: one `ChatMessage` per `setup.priorTurns` entry.
/// `run_agent` itself appends the current turn's `userMessage`.
pub fn prior_turns_to_messages(setup: Option<&Setup>) -> Vec<ChatMessage> {
    setup
        .map(|s| s.prior_turns.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|pt| ChatMessage {
            role: pt.role.clone(),
            content: Some(pt.content.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            preview: None,
        })
        .collect()
}

async fn seed_worship_libraries(state: &AppState, libraries: &[SeedLibrary]) -> anyhow::Result<()> {
    for lib in libraries {
        let library = state
            .create_library(&lib.name)
            .await
            .with_context(|| format!("creating seed library '{}'", lib.name))?;
        for pres in &lib.presentations {
            let slides = build_slides(&pres.slides)
                .with_context(|| format!("building seed slides for '{}'", pres.name))?;
            state
                .create_presentation(library.id, &pres.name, Some(&slides))
                .await
                .with_context(|| format!("creating seed presentation '{}'", pres.name))?;
        }
    }
    Ok(())
}

fn build_slides(seed_slides: &[SeedSlide]) -> anyhow::Result<Vec<Slide>> {
    seed_slides
        .iter()
        .enumerate()
        .map(|(order, s)| build_slide(order as u32, s))
        .collect()
}

fn build_slide(order: u32, seed: &SeedSlide) -> anyhow::Result<Slide> {
    let main = SlideText::new(&seed.main).map_err(|e| anyhow::anyhow!("seed slide main: {e}"))?;
    let translation = SlideText::new(&seed.translation)
        .map_err(|e| anyhow::anyhow!("seed slide translation: {e}"))?;
    let stage =
        SlideText::new(&seed.stage).map_err(|e| anyhow::anyhow!("seed slide stage: {e}"))?;
    let group = seed.group.as_deref().map(SlideGroup::new);
    let content = SlideContent::new(main, translation, stage, group);
    Ok(Slide::new(order, content))
}

async fn seed_bible_presentations(
    state: &AppState,
    presentations: &[SeedBiblePresentation],
) -> anyhow::Result<()> {
    for bp in presentations {
        let presentation = state
            .create_bible_presentation(&bp.name)
            .await
            .with_context(|| format!("creating seed bible presentation '{}'", bp.name))?;
        if bp.slides.is_empty() {
            continue;
        }
        let mut new_slides = Vec::with_capacity(bp.slides.len());
        for s in &bp.slides {
            let main = SlideText::new(&s.main)
                .map_err(|e| anyhow::anyhow!("seed bible slide main: {e}"))?;
            let secondary =
                SlideText::new("").map_err(|e| anyhow::anyhow!("empty SlideText failed: {e}"))?;
            new_slides.push(BiblePresentationSlide {
                id: BibleSlideId::new(),
                order: 0,
                main,
                main_reference: s.main_reference.clone(),
                secondary,
                secondary_reference: String::new(),
                metadata: None,
            });
        }
        state
            .append_bible_presentation_slides(presentation.id, new_slides)
            .await
            .with_context(|| format!("appending seed bible slides to '{}'", bp.name))?;
    }
    Ok(())
}
