//! An NDI video source mapped for the stage display.
//!
//! #789: the NDI name is the source's ONLY identity. An NDI name already reads
//! `MACHINE (Source)` — the sending PC and the source on it — so a second,
//! hand-typed label added nothing and could drift from reality. Display code
//! splits the name with [`ndi_name_parts`].

use crate::id::VideoSourceId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoSourceValidationError {
    #[error("NDI name cannot be empty")]
    EmptyNdiName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSource {
    pub id: VideoSourceId,
    pub ndi_name: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl VideoSource {
    pub fn new(
        id: VideoSourceId,
        ndi_name: String,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            ndi_name,
            is_active,
            created_at,
            updated_at,
        }
    }
}

/// What a caller supplies to create or rename a source. Deliberately NOT
/// `deny_unknown_fields`: a pre-#789 caller that still sends `label` keeps
/// working, and the label is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceDraft {
    pub ndi_name: String,
}

impl VideoSourceDraft {
    pub fn new(ndi_name: impl Into<String>) -> Self {
        Self {
            ndi_name: ndi_name.into(),
        }
    }

    pub fn validate(&self) -> Result<(), VideoSourceValidationError> {
        if self.ndi_name.trim().is_empty() {
            return Err(VideoSourceValidationError::EmptyNdiName);
        }
        Ok(())
    }
}

/// Split an NDI name `MACHINE (Source)` into `(Some(machine), source)` for
/// display, both trimmed. The machine is everything before the FIRST `(` (a
/// host name carries no parentheses); the source is everything up to the
/// closing `)` that ends the name, so `OBS-PC (Scene (1))` keeps `Scene (1)`.
///
/// Any other shape — no parenthesis, nothing before it, an empty source, or
/// text after the closing `)` — returns `(None, name)` with the whole trimmed
/// name as the source, so a display never loses information.
pub fn ndi_name_parts(name: &str) -> (Option<&str>, &str) {
    let trimmed = name.trim();
    let split = trimmed
        .strip_suffix(')')
        .and_then(|inner| inner.split_once('('))
        .map(|(machine, source)| (machine.trim(), source.trim()))
        .filter(|(machine, source)| !machine.is_empty() && !source.is_empty());
    match split {
        Some((machine, source)) => (Some(machine), source),
        None => (None, trimmed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_passes_with_only_an_ndi_name() {
        // #789: the NDI name is the whole identity — no second, hand-typed label.
        let draft = VideoSourceDraft::new("CAM1 (usb)");
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn validation_fails_with_empty_ndi_name() {
        let draft = VideoSourceDraft::new("");
        assert_eq!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyNdiName)
        );
    }

    #[test]
    fn validation_fails_with_whitespace_ndi_name() {
        let draft = VideoSourceDraft::new("  ");
        assert_eq!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyNdiName)
        );
    }

    #[test]
    fn draft_deserializes_from_ndi_name_alone_and_ignores_a_legacy_label() {
        let draft: VideoSourceDraft =
            serde_json::from_str(r#"{"ndiName":"RESOLUME-PP (cg-obs)"}"#).unwrap();
        assert_eq!(draft.ndi_name, "RESOLUME-PP (cg-obs)");
        let legacy: VideoSourceDraft =
            serde_json::from_str(r#"{"label":"Old","ndiName":"RESOLUME-PP (cg-obs)"}"#).unwrap();
        assert_eq!(legacy, draft);
    }

    #[test]
    fn video_source_serializes_without_a_label() {
        let now = Utc::now();
        let source = VideoSource::new(VideoSourceId::new(), "PC (src)".into(), false, now, now);
        let json = serde_json::to_value(&source).unwrap();
        assert!(json.get("label").is_none(), "no label on the wire: {json}");
        assert_eq!(json["ndiName"], "PC (src)");
    }

    #[test]
    fn ndi_name_parts_splits_machine_and_source() {
        assert_eq!(
            ndi_name_parts("RESOLUME-PP (cg-obs)"),
            (Some("RESOLUME-PP"), "cg-obs")
        );
        assert_eq!(
            ndi_name_parts("STREAM-SNV (stream)"),
            (Some("STREAM-SNV"), "stream")
        );
    }

    #[test]
    fn ndi_name_parts_trims_surrounding_and_inner_whitespace() {
        assert_eq!(
            ndi_name_parts("  RESOLUME-PP  (  cg-obs )  "),
            (Some("RESOLUME-PP"), "cg-obs")
        );
    }

    #[test]
    fn ndi_name_parts_keeps_nested_parentheses_in_the_source() {
        assert_eq!(
            ndi_name_parts("OBS-PC (Scene (1))"),
            (Some("OBS-PC"), "Scene (1)")
        );
    }

    #[test]
    fn ndi_name_parts_without_a_machine_returns_the_whole_name() {
        assert_eq!(ndi_name_parts("Plain"), (None, "Plain"));
        assert_eq!(ndi_name_parts("  Plain  "), (None, "Plain"));
        assert_eq!(
            ndi_name_parts("BOGUS_DOES_NOT_EXIST"),
            (None, "BOGUS_DOES_NOT_EXIST")
        );
    }

    #[test]
    fn ndi_name_parts_malformed_shapes_fall_back_to_the_whole_name() {
        // No machine before the parenthesis.
        assert_eq!(ndi_name_parts("(cg-obs)"), (None, "(cg-obs)"));
        // Empty source.
        assert_eq!(ndi_name_parts("PC ()"), (None, "PC ()"));
        assert_eq!(ndi_name_parts("PC (   )"), (None, "PC (   )"));
        // Unclosed / not at the end.
        assert_eq!(ndi_name_parts("PC (src"), (None, "PC (src"));
        assert_eq!(ndi_name_parts("PC (src) tail"), (None, "PC (src) tail"));
        assert_eq!(ndi_name_parts(""), (None, ""));
    }
}
