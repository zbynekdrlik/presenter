use presenter_core::{
    Library, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText,
};
use presenter_persistence::{DatabaseSettings, Repository};
use std::env;
use tracing::instrument;

#[derive(Clone)]
pub struct AppState {
    repository: Repository,
}

impl AppState {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }

    #[instrument(skip_all)]
    pub async fn from_env() -> anyhow::Result<Self> {
        let db_url = env::var("PRESENTER_DB_URL")
            .unwrap_or_else(|_| "sqlite://presenter_dev.db".to_string());
        let repo = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
        let state = Self::new(repo);
        state.ensure_seed_library().await?;
        Ok(state)
    }

    #[instrument(skip_all)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        let repo = Repository::connect_in_memory().await?;
        let state = Self::new(repo);
        state.ensure_seed_library().await?;
        Ok(state)
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        self.repository.fetch_libraries().await
    }

    async fn ensure_seed_library(&self) -> anyhow::Result<()> {
        if self.repository.fetch_libraries().await?.is_empty() {
            self.repository.upsert_library(&sample_library()).await?;
        }
        Ok(())
    }
}

fn sample_library() -> Library {
    let presentation = Presentation::new(
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
            )
            .with_id(SlideId::new()),
            Slide::new(
                1,
                SlideContent::new(
                    SlideText::new("Let's worship").unwrap(),
                    SlideText::new("Poďme chváliť").unwrap(),
                    SlideText::new("Cue").unwrap(),
                    None,
                ),
            )
            .with_id(SlideId::new()),
        ],
    )
    .unwrap()
    .with_id(PresentationId::new());

    Library::new("Sample Library", vec![presentation])
        .unwrap()
        .with_id(LibraryId::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seeded_state_contains_library() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Sample Library");
    }
}
