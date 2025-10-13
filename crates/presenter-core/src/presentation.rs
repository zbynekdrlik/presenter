#![allow(clippy::cast_possible_truncation)]

use crate::id::{PresentationId, SlideId};
use crate::slide::{resolve_sequence, ResolvedSlide, Slide};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresentationError {
    #[error("slide order must be unique; duplicate index {0}")]
    DuplicateOrder(u32),
    #[error("slide with id {0} not found")]
    SlideNotFound(SlideId),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    pub id: PresentationId,
    pub name: String,
    pub slides: Vec<Slide>,
}

impl Presentation {
    pub fn new<T: Into<String>>(name: T, slides: Vec<Slide>) -> Result<Self, PresentationError> {
        let presentation = Self {
            id: PresentationId::new(),
            name: name.into(),
            slides,
        };
        presentation.ensure_unique_orders()?;
        Ok(presentation)
    }

    pub fn with_id(mut self, id: PresentationId) -> Self {
        self.id = id;
        self
    }

    pub fn push_slide(&mut self, slide: Slide) -> Result<(), PresentationError> {
        if self
            .slides
            .iter()
            .any(|existing| existing.order == slide.order)
        {
            return Err(PresentationError::DuplicateOrder(slide.order));
        }
        self.slides.push(slide);
        self.slides.sort_by_key(|slide| slide.order);
        Ok(())
    }

    pub fn reorder_slide(
        &mut self,
        slide_id: SlideId,
        new_position: u32,
    ) -> Result<(), PresentationError> {
        let current_index = self
            .slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or(PresentationError::SlideNotFound(slide_id))?;
        let mut slide = self.slides.remove(current_index);
        let bounded_position = new_position.min(self.slides.len() as u32);
        slide.order = bounded_position;
        self.slides.insert(bounded_position as usize, slide);
        self.reindex_orders();
        Ok(())
    }

    pub fn resolved_slides(&self) -> Vec<ResolvedSlide> {
        resolve_sequence(&self.slides)
    }

    fn ensure_unique_orders(&self) -> Result<(), PresentationError> {
        let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
        for slide in &self.slides {
            *counts.entry(slide.order).or_default() += 1;
        }
        if let Some((order, _count)) = counts.iter().find(|(_, count)| **count > 1) {
            return Err(PresentationError::DuplicateOrder(*order));
        }
        Ok(())
    }

    fn reindex_orders(&mut self) {
        for (index, slide) in self.slides.iter_mut().enumerate() {
            slide.order = index as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slide::{SlideContent, SlideGroup, SlideText};

    fn sample_slide(order: u32, group: Option<&str>) -> Slide {
        Slide::new(
            order,
            SlideContent::new(
                SlideText::new(format!("Slide {order}")).unwrap(),
                SlideText::new("Translation").unwrap(),
                SlideText::new("Stage").unwrap(),
                group.map(SlideGroup::new),
            ),
        )
    }

    #[test]
    fn duplicate_orders_are_rejected() {
        let slides = vec![sample_slide(0, None), sample_slide(0, Some("Verse"))];
        let err = Presentation::new("Duplicate", slides).unwrap_err();
        assert_eq!(err, PresentationError::DuplicateOrder(0));
    }

    #[test]
    fn reorder_slide_updates_orders_and_groups() {
        let mut presentation = Presentation::new(
            "Test",
            vec![sample_slide(0, Some("Intro")), sample_slide(1, None)],
        )
        .unwrap();

        let second_id = presentation.slides[1].id;
        presentation.reorder_slide(second_id, 0).unwrap();

        let resolved = presentation.resolved_slides();
        assert_eq!(resolved[0].order, 0);
        assert!(resolved[0].effective_group.is_none());
        assert_eq!(resolved[1].order, 1);
        assert!(resolved[1]
            .effective_group
            .as_ref()
            .map(|g| g.name())
            .unwrap()
            .contains("Intro"));
    }

    #[test]
    fn resolved_slides_propagate_group() {
        let presentation = Presentation::new(
            "Group Test",
            vec![sample_slide(0, Some("Verse")), sample_slide(1, None)],
        )
        .unwrap();
        let resolved = presentation.resolved_slides();

        assert_eq!(
            resolved
                .iter()
                .map(|slide| slide.effective_group.as_ref().map(|g| g.name().to_string()))
                .collect::<Vec<_>>(),
            vec![Some("Verse".into()), Some("Verse".into())]
        );
    }

    #[test]
    fn reorder_errors_when_slide_missing() {
        let mut presentation = Presentation::new("Test", vec![sample_slide(0, None)]).unwrap();
        let random_id = SlideId::new();
        let err = presentation.reorder_slide(random_id, 1).unwrap_err();
        assert_eq!(err, PresentationError::SlideNotFound(random_id));
    }
}
