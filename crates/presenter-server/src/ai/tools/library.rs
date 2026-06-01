//! Library and (non-Bible) presentation tools.

use super::{make_slide, slide_to_json, str_field, uuid_field};
use crate::state::AppState;
use presenter_core::{LibraryId, PresentationId, Slide};
use serde_json::{json, Value};

pub(super) async fn list_libraries(state: &AppState) -> anyhow::Result<(String, String)> {
    let libs = state.libraries().await?;
    let summary: Vec<Value> = libs
        .iter()
        .map(|l| json!({"id": l.id.to_string(), "name": l.name}))
        .collect();
    let preview = format!("Found {} libraries", summary.len());
    Ok((serde_json::to_string(&summary)?, preview))
}

pub(super) async fn create_library(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let name_val = str_field(args, "name")?;
    let lib = state.create_library(&name_val).await?;
    let preview = format!("Created library '{}'", lib.name);
    Ok((
        json!({"id": lib.id.to_string(), "name": lib.name}).to_string(),
        preview,
    ))
}

pub(super) async fn list_presentations(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let lib_id = LibraryId::from_uuid(uuid_field(args, "library_id")?);
    let summaries = state.library_summaries(None).await?;
    let lib_summary = summaries.iter().find(|s| s.id == lib_id);
    let presentations: Vec<Value> = lib_summary
        .map(|ls| {
            ls.presentations
                .iter()
                .map(|p| json!({"id": p.id.to_string(), "name": p.name}))
                .collect()
        })
        .unwrap_or_default();
    let preview = format!("Found {} presentations", presentations.len());
    Ok((serde_json::to_string(&presentations)?, preview))
}

pub(super) async fn get_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let detail = state.presentation_detail(pres_id).await?;
    match detail {
        Some((lib_id, lib_name, pres)) => {
            let slides: Vec<Value> = pres.slides.iter().map(|s| slide_to_json(s)).collect();
            let preview = format!("'{}' - {} slides", pres.name, pres.slides.len());
            Ok((
                json!({
                    "id": pres.id.to_string(),
                    "name": pres.name,
                    "library_id": lib_id.to_string(),
                    "library_name": lib_name,
                    "slides": slides
                })
                .to_string(),
                preview,
            ))
        }
        None => Ok((
            json!({"error": "presentation not found"}).to_string(),
            "Not found".to_string(),
        )),
    }
}

pub(super) async fn create_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let lib_id = LibraryId::from_uuid(uuid_field(args, "library_id")?);
    let name_val = str_field(args, "name")?;
    let slides_arr = args["slides"].as_array();

    let slides: Option<Vec<Slide>> = slides_arr.map(|arr| {
        arr.iter()
            .enumerate()
            .map(|(i, s)| make_slide(i, s))
            .collect()
    });

    let (_, _, pres, _) = state
        .create_presentation(lib_id, &name_val, slides.as_deref())
        .await?;
    let preview = format!("Created '{}' with {} slides", pres.name, pres.slides.len());
    Ok((
        json!({
            "id": pres.id.to_string(),
            "name": pres.name,
            "slide_count": pres.slides.len()
        })
        .to_string(),
        preview,
    ))
}

pub(super) async fn rename_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let name_val = str_field(args, "name")?;
    state.rename_presentation(pres_id, &name_val).await?;
    let preview = format!("Renamed to '{name_val}'");
    Ok((json!({"ok": true}).to_string(), preview))
}

pub(super) async fn delete_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = PresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    state.delete_presentation(pres_id).await?;
    Ok((
        json!({"ok": true}).to_string(),
        "Deleted presentation".to_string(),
    ))
}
