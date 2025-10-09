use crate::id::{LibraryId, PresentationId};
use crate::presentation::Presentation;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryCategory {
    Worship,
    Bible,
    System,
}

impl Default for LibraryCategory {
    fn default() -> Self {
        LibraryCategory::Worship
    }
}

impl LibraryCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            LibraryCategory::Worship => "worship",
            LibraryCategory::Bible => "bible",
            LibraryCategory::System => "system",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "worship" => Some(LibraryCategory::Worship),
            "bible" => Some(LibraryCategory::Bible),
            "system" => Some(LibraryCategory::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSummary {
    pub id: PresentationId,
    pub name: String,
}

impl PresentationSummary {
    pub fn new(id: PresentationId, name: String) -> Self {
        Self { id, name }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LibraryError {
    #[error("presentation {0} not found in library")]
    PresentationNotFound(PresentationId),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: LibraryId,
    pub name: String,
    pub category: LibraryCategory,
    pub presentations: Vec<Presentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub id: LibraryId,
    pub name: String,
    pub category: LibraryCategory,
    pub presentation_count: usize,
    pub presentations: Vec<PresentationSummary>,
}

impl LibrarySummary {
    pub fn new(
        id: LibraryId,
        name: String,
        category: LibraryCategory,
        presentation_count: usize,
        presentations: Vec<PresentationSummary>,
    ) -> Self {
        Self {
            id,
            name,
            category,
            presentation_count,
            presentations,
        }
    }
}

impl Library {
    pub fn new<T: Into<String>>(
        name: T,
        category: LibraryCategory,
        presentations: Vec<Presentation>,
    ) -> Result<Self, LibraryError> {
        Ok(Self {
            id: LibraryId::new(),
            name: name.into(),
            category,
            presentations,
        })
    }

    pub fn with_id(mut self, id: LibraryId) -> Self {
        self.id = id;
        self
    }

    pub fn add_presentation(&mut self, presentation: Presentation) -> Result<(), LibraryError> {
        self.presentations.push(presentation);
        self.presentations
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(())
    }

    pub fn remove_presentation(
        &mut self,
        presentation_id: PresentationId,
    ) -> Result<Presentation, LibraryError> {
        let index = self
            .presentations
            .iter()
            .position(|p| p.id == presentation_id)
            .ok_or(LibraryError::PresentationNotFound(presentation_id))?;
        Ok(self.presentations.remove(index))
    }

    pub fn get(&self, presentation_id: PresentationId) -> Option<&Presentation> {
        self.presentations.iter().find(|p| p.id == presentation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::Presentation;
    use crate::slide::{Slide, SlideContent, SlideText};

    fn presentation_named(name: &str, order: u32) -> Presentation {
        Presentation::new(
            name,
            vec![Slide::new(
                order,
                SlideContent::new(
                    SlideText::new("Main").unwrap(),
                    SlideText::new("Translation").unwrap(),
                    SlideText::new("Stage").unwrap(),
                    None,
                ),
            )],
        )
        .unwrap()
    }

    #[test]
    fn remove_presentation_returns_removed_entity() {
        let mut library = Library::new(
            "Songs",
            LibraryCategory::Worship,
            vec![presentation_named("Song", 0)],
        )
        .unwrap();
        let id = library.presentations[0].id;
        let removed = library.remove_presentation(id).unwrap();
        assert_eq!(removed.id, id);
        assert!(library.presentations.is_empty());
    }

    #[test]
    fn duplicate_presentation_names_are_allowed() {
        let library = Library::new(
            "Songs",
            LibraryCategory::Worship,
            vec![presentation_named("Song", 0), presentation_named("Song", 1)],
        )
        .unwrap();
        assert_eq!(library.presentations.len(), 2);
    }

    #[test]
    fn remove_missing_presentation_errors() {
        let mut library = Library::new(
            "Songs",
            LibraryCategory::Worship,
            vec![presentation_named("Song", 0)],
        )
        .unwrap();
        let err = library
            .remove_presentation(PresentationId::new())
            .unwrap_err();
        assert!(matches!(err, LibraryError::PresentationNotFound(_)));
    }
}
