use crate::id::{LibraryId, PresentationId};
use crate::presentation::Presentation;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LibraryError {
    #[error("presentation with name '{0}' already exists in library")]
    DuplicatePresentation(String),
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

impl Library {
    pub fn new<T: Into<String>>(
        name: T,
        presentations: Vec<Presentation>,
    ) -> Result<Self, LibraryError> {
        let library = Self {
            id: LibraryId::new(),
            name: name.into(),
            presentations,
        };
        library.ensure_unique_names()?;
        Ok(library)
    }

    pub fn with_id(mut self, id: LibraryId) -> Self {
        self.id = id;
        self
    }

    pub fn add_presentation(&mut self, presentation: Presentation) -> Result<(), LibraryError> {
        if self
            .presentations
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(&presentation.name))
        {
            return Err(LibraryError::DuplicatePresentation(
                presentation.name.clone(),
            ));
        }
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

    fn ensure_unique_names(&self) -> Result<(), LibraryError> {
        let mut seen: HashSet<String> = HashSet::new();
        for presentation in &self.presentations {
            let key = presentation.name.to_lowercase();
            if !seen.insert(key.clone()) {
                return Err(LibraryError::DuplicatePresentation(key));
            }
        }
        Ok(())
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
    fn duplicate_names_are_rejected() {
        let result = Library::new(
            "Songs",
            vec![presentation_named("Song", 0), presentation_named("song", 1)],
        );
        assert_eq!(
            result.unwrap_err(),
            LibraryError::DuplicatePresentation("song".into())
        );
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
    fn remove_missing_presentation_errors() {
        let mut library = Library::new("Songs", vec![presentation_named("Song", 0)]).unwrap();
        let err = library
            .remove_presentation(PresentationId::new())
            .unwrap_err();
        assert!(matches!(err, LibraryError::PresentationNotFound(_)));
    }
}
