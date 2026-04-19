use crate::id::{LibraryId, PresentationId};
use crate::presentation::Presentation;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub presentations: Vec<Presentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub id: LibraryId,
    pub name: String,
    pub presentation_count: usize,
    pub presentations: Vec<PresentationSummary>,
}

impl LibrarySummary {
    pub fn new(
        id: LibraryId,
        name: String,
        presentation_count: usize,
        presentations: Vec<PresentationSummary>,
    ) -> Self {
        Self {
            id,
            name,
            presentation_count,
            presentations,
        }
    }
}

impl Library {
    pub fn new<T: Into<String>>(
        name: T,
        presentations: Vec<Presentation>,
    ) -> Result<Self, LibraryError> {
        Ok(Self {
            id: LibraryId::new(),
            name: name.into(),
            presentations,
        })
    }

    pub fn with_id(mut self, id: LibraryId) -> Self {
        self.id = id;
        self
    }

    pub fn add_presentation(&mut self, presentation: Presentation) -> Result<(), LibraryError> {
        self.presentations.push(presentation);
        self.presentations.sort_by_key(|p| p.name.to_lowercase());
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
        let mut library = Library::new("Songs", vec![presentation_named("Song", 0)]).unwrap();
        let id = library.presentations[0].id;
        let removed = library.remove_presentation(id).unwrap();
        assert_eq!(removed.id, id);
        assert!(library.presentations.is_empty());
    }

    #[test]
    fn duplicate_presentation_names_are_allowed() {
        let library = Library::new(
            "Songs",
            vec![presentation_named("Song", 0), presentation_named("Song", 1)],
        )
        .unwrap();
        assert_eq!(library.presentations.len(), 2);
    }

    #[test]
    fn remove_missing_presentation_errors() {
        let mut library = Library::new("Songs", vec![presentation_named("Song", 0)]).unwrap();
        let err = library
            .remove_presentation(PresentationId::new())
            .unwrap_err();
        assert!(matches!(err, LibraryError::PresentationNotFound(_)));
    }
}
