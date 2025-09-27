use presenter_core::{Library, Presentation, Slide, SlideContent, SlideGroup, SlideText};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct AppState {
    libraries: Arc<RwLock<Vec<Library>>>,
}

impl AppState {
    pub fn new(libraries: Vec<Library>) -> Self {
        Self {
            libraries: Arc::new(RwLock::new(libraries)),
        }
    }

    pub fn seeded() -> Self {
        let sample_library = Library::new(
            "Sample Library",
            vec![Presentation::new(
                "Welcome",
                vec![
                    Slide::new(
                        0,
                        SlideContent::new(
                            SlideText::new("Welcome to service").unwrap(),
                            SlideText::new("Vitajte").unwrap(),
                            SlideText::new("Stage cue").unwrap(),
                            Some(SlideGroup::new("Intro")),
                        ),
                    ),
                    Slide::new(
                        1,
                        SlideContent::new(
                            SlideText::new("Let's worship").unwrap(),
                            SlideText::new("Poďme chváliť").unwrap(),
                            SlideText::new("Cue").unwrap(),
                            None,
                        ),
                    ),
                ],
            )
            .unwrap()],
        )
        .unwrap();
        Self::new(vec![sample_library])
    }

    pub async fn libraries(&self) -> Vec<Library> {
        self.libraries.read().await.clone()
    }
}
