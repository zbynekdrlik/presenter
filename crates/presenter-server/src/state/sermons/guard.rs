//! Sermon slides are generated from the items (#851 spec §8.2): the slide write
//! paths of a Bible presentation (append, edit, delete, reorder; HTTP and the AI
//! tools alike, both call the same `AppState` methods) refuse a sermon's
//! presentation with 409. Renaming (= the sermon title) and deleting it stay
//! allowed.

use presenter_core::BiblePresentationId;
use presenter_persistence::RepositoryError;

use super::super::AppState;

pub(super) const SERMON_SLIDES_REFUSAL: &str =
    "Slajdy kázne sa generujú z jej položiek — upravuje sa v /ui/study";

impl AppState {
    /// 409 (`RepositoryError::Conflict`, central mapping) when the presentation
    /// is a sermon.
    pub(crate) async fn refuse_if_sermon(
        &self,
        presentation_id: BiblePresentationId,
    ) -> anyhow::Result<()> {
        if self.repository.is_sermon(presentation_id).await? {
            return Err(RepositoryError::Conflict(SERMON_SLIDES_REFUSAL).into());
        }
        Ok(())
    }
}
