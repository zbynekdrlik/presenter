use crate::id::VideoSourceId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoSourceValidationError {
    #[error("label cannot be empty")]
    EmptyLabel,
    #[error("NDI name cannot be empty")]
    EmptyNdiName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSource {
    pub id: VideoSourceId,
    pub label: String,
    pub ndi_name: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl VideoSource {
    pub fn new(
        id: VideoSourceId,
        label: String,
        ndi_name: String,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            label,
            ndi_name,
            is_active,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceDraft {
    pub label: String,
    pub ndi_name: String,
}

impl VideoSourceDraft {
    pub fn new(label: impl Into<String>, ndi_name: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ndi_name: ndi_name.into(),
        }
    }

    pub fn validate(&self) -> Result<(), VideoSourceValidationError> {
        if self.label.trim().is_empty() {
            return Err(VideoSourceValidationError::EmptyLabel);
        }
        if self.ndi_name.trim().is_empty() {
            return Err(VideoSourceValidationError::EmptyNdiName);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_passes_with_valid_draft() {
        let draft = VideoSourceDraft::new("Main Camera", "CAM1 (usb)");
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn validation_fails_with_empty_label() {
        let draft = VideoSourceDraft::new("", "CAM1");
        assert!(matches!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyLabel)
        ));
    }

    #[test]
    fn validation_fails_with_whitespace_label() {
        let draft = VideoSourceDraft::new("  ", "CAM1");
        assert!(matches!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyLabel)
        ));
    }

    #[test]
    fn validation_fails_with_empty_ndi_name() {
        let draft = VideoSourceDraft::new("Camera", "");
        assert!(matches!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyNdiName)
        ));
    }

    #[test]
    fn validation_fails_with_whitespace_ndi_name() {
        let draft = VideoSourceDraft::new("Camera", "  ");
        assert!(matches!(
            draft.validate(),
            Err(VideoSourceValidationError::EmptyNdiName)
        ));
    }
}
