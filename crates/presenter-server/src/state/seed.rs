#[cfg(test)]
use presenter_core::{
    Library, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText,
};

#[cfg(test)]
#[async_trait::async_trait]
pub trait TestBibleIngestion {
    async fn ingest_default_translations(
        &self,
    ) -> anyhow::Result<Vec<presenter_bible::BibleImportSummary>>;
}

#[cfg(test)]
pub(crate) fn sample_library() -> Library {
    // These unwrap calls are safe because the sample data uses known-valid strings
    // that are well within the character limits
    let main1 = SlideText::new("Welcome to service")
        .unwrap_or_else(|_| SlideText::new("Welcome").unwrap_or_else(|_| unreachable!()));
    let trans1 = SlideText::new("Vitajte")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));
    let stage1 = SlideText::new("Stage cue")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));

    let main2 = SlideText::new("Let's worship")
        .unwrap_or_else(|_| SlideText::new("Worship").unwrap_or_else(|_| unreachable!()));
    let trans2 = SlideText::new("Poďme chváliť")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));
    let stage2 = SlideText::new("Cue")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));

    let presentation = Presentation::new(
        "Welcome",
        vec![
            Slide::new(
                0,
                SlideContent::new(main1, trans1, stage1, Some(SlideGroup::new("Intro"))),
            )
            .with_id(SlideId::new()),
            Slide::new(1, SlideContent::new(main2, trans2, stage2, None)).with_id(SlideId::new()),
        ],
    )
    .unwrap_or_else(|_| {
        Presentation::new("Welcome", vec![])
            .unwrap_or_else(|_| unreachable!("empty presentation should be valid"))
    })
    .with_id(PresentationId::new());

    Library::new("Sample Library", vec![presentation])
        .unwrap_or_else(|_| {
            Library::new("Sample", vec![])
                .unwrap_or_else(|_| unreachable!("empty library should be valid"))
        })
        .with_id(LibraryId::new())
}

#[cfg(test)]
pub(crate) async fn seed_sample_library(state: &super::AppState) -> anyhow::Result<()> {
    state.repository.upsert_library(&sample_library()).await?;
    Ok(())
}
