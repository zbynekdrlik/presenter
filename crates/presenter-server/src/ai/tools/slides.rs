//! Presentation-slide editing and stage-trigger tools.

use super::{str_field, uuid_field};
use crate::state::AppState;
use presenter_core::{PresentationId, SlideId};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) async fn add_slide(args: &Value, state: &AppState) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    // Validate the required `main` field before mutating the
    // presentation, so a malformed call doesn't leave a blank slide.
    let main = str_field(args, "main")?;
    let position = args["position"].as_u64().map(|p| p as u32);
    let slides = state.insert_blank_slide(pres_id, position).await?;
    // Update the last inserted slide with content
    if let Some(slide) = slides.last() {
        let translation = args["translation"].as_str().unwrap_or("").to_string();
        let stage = args["stage"].as_str().unwrap_or("").to_string();
        let group = args["group"].as_str().map(String::from);
        state
            .update_slide_content(pres_id, slide.id, main, translation, stage, group, None)
            .await?;
    }
    let preview = format!("Added slide (now {} total)", slides.len());
    Ok((
        json!({"ok": true, "slide_count": slides.len()}).to_string(),
        preview,
    ))
}

pub(super) async fn update_slide(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let slide_id = SlideId::from_uuid(uuid_field(args, "slide_id")?);
    let main = str_field(args, "main")?;
    let translation = args["translation"].as_str().unwrap_or("").to_string();
    let stage = args["stage"].as_str().unwrap_or("").to_string();
    let group = args["group"].as_str().map(String::from);
    state
        .update_slide_content(pres_id, slide_id, main, translation, stage, group, None)
        .await?;
    Ok((json!({"ok": true}).to_string(), "Updated slide".to_string()))
}

pub(super) async fn delete_slide(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let slide_id = SlideId::from_uuid(uuid_field(args, "slide_id")?);
    let slides = state.delete_slide(pres_id, slide_id).await?;
    let preview = format!("Deleted slide ({} remaining)", slides.len());
    Ok((
        json!({"ok": true, "remaining": slides.len()}).to_string(),
        preview,
    ))
}

pub(super) async fn reorder_slides(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let ids: Vec<SlideId> = args["slide_ids"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| Uuid::parse_str(s).ok())
                .map(SlideId::from_uuid)
                .collect()
        })
        .unwrap_or_default();
    state.reorder_slides(pres_id, ids).await?;
    Ok((
        json!({"ok": true}).to_string(),
        "Reordered slides".to_string(),
    ))
}

pub(super) async fn trigger_slide(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let slide_id = SlideId::from_uuid(uuid_field(args, "slide_id")?);

    // Find the next slide
    let detail = state.presentation_detail(pres_id).await?;
    let next_slide_id = detail.and_then(|(_, _, pres)| {
        let pos = pres.slides.iter().position(|s| s.id == slide_id);
        pos.and_then(|i| pres.slides.get(i + 1)).map(|s| s.id)
    });

    state
        .update_stage_state(pres_id, slide_id, next_slide_id, None, None)
        .await?;
    Ok((
        json!({"ok": true}).to_string(),
        "Triggered slide on stage".to_string(),
    ))
}

pub(super) async fn clear_stage(state: &AppState) -> anyhow::Result<(String, String)> {
    state.clear_stage().await?;
    Ok((json!({"ok": true}).to_string(), "Stage cleared".to_string()))
}
