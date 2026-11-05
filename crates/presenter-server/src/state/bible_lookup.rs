//! A translation as typed (#850 §5.8): `KJV`, `seb`, `slk-seb`. Companion's
//! default has always been `KJV`, which no exact code matches.

use super::AppState;
use presenter_core::bible::resolver::{
    lookup, resolve_translation_code, Resolution, Versification,
};
use presenter_persistence::RepositoryError;

impl AppState {
    /// The listed translation code `input` names; `NotFound` (404) when none does.
    pub async fn resolve_translation(&self, input: &str) -> anyhow::Result<String> {
        let translations = self.list_bible_translations().await?;
        let listed: Vec<&str> = translations.iter().map(|t| t.code.as_str()).collect();
        let code = resolve_translation_code(input, &listed)
            .ok_or(RepositoryError::NotFound("bible translation not found"))?;
        Ok(code)
    }

    /// `query` read against `translation`'s structure, or the main preference's
    /// when none is named. Returns the translation code it was read against.
    pub async fn lookup_bible_reference(
        &self,
        query: &str,
        translation: Option<&str>,
    ) -> anyhow::Result<(String, Resolution)> {
        let named = match translation.map(str::trim).filter(|t| !t.is_empty()) {
            Some(named) => named.to_string(),
            None => self
                .get_bible_preferences()
                .await?
                .main_translation
                .ok_or(RepositoryError::NotFound("no main bible translation"))?,
        };
        let code = self.resolve_translation(&named).await?;
        let summaries = self.bible_book_chapter_summaries(&code).await?;
        let structure = Versification::from_chapter_summaries(&summaries);
        Ok((code, lookup(query, &structure)))
    }
}
