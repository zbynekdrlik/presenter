use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::{BiblePresentationId, BibleSlideId};
use crate::slide::{BibleSlideMetadata, SlideText};

/// A user-curated collection of bible slides (e.g., for a sermon series).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentation {
    pub id: BiblePresentationId,
    pub name: String,
    pub slides: Vec<BibleSlide>,
    pub created_at: DateTime<Utc>,
}

/// A single bible slide within a `BiblePresentation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlide {
    pub id: BibleSlideId,
    pub order: u32,
    pub main: SlideText,
    pub main_reference: String,
    pub secondary: SlideText,
    pub secondary_reference: String,
    pub metadata: Option<BibleSlideMetadata>,
}

/// Lightweight summary of a bible presentation for list views.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentationSummary {
    pub id: BiblePresentationId,
    pub name: String,
    pub slide_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn bible_presentation_id_roundtrips_uuid() {
        let original = Uuid::new_v4();
        let id = BiblePresentationId::from_uuid(original);
        assert_eq!(id.into_uuid(), original);
    }

    #[test]
    fn bible_slide_id_roundtrips_uuid() {
        let original = Uuid::new_v4();
        let id = BibleSlideId::from_uuid(original);
        assert_eq!(id.into_uuid(), original);
    }

    #[test]
    fn bible_presentation_id_serializes_as_uuid_string() {
        let id = BiblePresentationId::from_uuid(
            Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
        );
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""01234567-89ab-cdef-0123-456789abcdef""#);
    }

    #[test]
    fn bible_presentation_serialization_uses_camel_case() {
        let pres = BiblePresentation {
            id: BiblePresentationId::from_uuid(Uuid::nil()),
            name: "Test".to_string(),
            slides: vec![],
            created_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        };
        let json = serde_json::to_string(&pres).unwrap();
        assert!(json.contains(r#""createdAt""#));
        assert!(!json.contains(r#""created_at""#));
    }
}
