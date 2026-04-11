use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use presenter_core::{
    search::{fold_query, query_tokens},
    LibraryId, PresentationId, SearchMatchField, SearchResult, SearchResultKind, SlideId,
};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

use super::util::parse_uuid;
use super::Repository;

struct SearchContext {
    tokens: Vec<String>,
    has_tokens: bool,
    folded: String,
    has_folded: bool,
    query_lower: String,
    trimmed: String,
    cap: usize,
    results: Vec<SearchResult>,
    library_names: HashMap<String, String>,
    seen_library_ids: HashSet<String>,
    seen_presentation_ids: HashSet<String>,
    seen_slide_ids: HashSet<String>,
    matched_library_ids: HashSet<String>,
    matched_presentation_ids: HashSet<String>,
}

impl SearchContext {
    fn remaining(&self) -> usize {
        self.cap.saturating_sub(self.results.len())
    }

    fn is_full(&self) -> bool {
        self.results.len() >= self.cap
    }
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn search_presenter(
        &self,
        query: &str,
        limit: u64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let tokens = query_tokens(trimmed);
        let has_tokens = !tokens.is_empty();
        let folded = fold_query(trimmed);
        let has_folded = !folded.is_empty();
        let query_lower = trimmed.to_lowercase();
        let cap = limit.clamp(1, 100) as usize;

        let mut ctx = SearchContext {
            tokens,
            has_tokens,
            folded,
            has_folded,
            query_lower,
            trimmed: trimmed.to_string(),
            cap,
            results: Vec::with_capacity(cap),
            library_names: HashMap::with_capacity(cap),
            seen_library_ids: HashSet::with_capacity(cap),
            seen_presentation_ids: HashSet::with_capacity(cap),
            seen_slide_ids: HashSet::with_capacity(cap),
            matched_library_ids: HashSet::with_capacity(cap),
            matched_presentation_ids: HashSet::new(),
        };

        self.search_libraries(&mut ctx).await?;
        if !ctx.is_full() {
            self.search_presentations(&mut ctx).await?;
        }
        if !ctx.is_full() {
            self.search_slides(&mut ctx).await?;
        }

        Ok(ctx.results)
    }

    async fn search_libraries(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let mut library_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            library_condition = library_condition.add(library::Column::Name.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            let mut token_condition = Condition::any();
            for token in &ctx.tokens {
                token_condition =
                    token_condition.add(library::Column::SearchName.contains(token.clone()));
            }
            library_condition = library_condition.add(token_condition);
        }

        let library_models = library::Entity::find()
            .filter(library_condition)
            .order_by_asc(library::Column::Name)
            .limit(ctx.remaining() as u64)
            .all(&self.db)
            .await?;

        for model in library_models {
            if !ctx.seen_library_ids.insert(model.id.clone()) {
                continue;
            }
            let tokens_in_name = if ctx.has_tokens {
                let haystack = fold_query(&model.name);
                ctx.tokens
                    .iter()
                    .filter(|token| haystack.contains(*token))
                    .count()
            } else {
                0
            };
            if tokens_in_name > 0 {
                ctx.matched_library_ids.insert(model.id.clone());
            }
            if ctx.has_tokens && tokens_in_name < ctx.tokens.len() {
                continue;
            }
            let library_id = LibraryId::from_uuid(parse_uuid(&model.id)?);
            ctx.library_names
                .insert(model.id.clone(), model.name.clone());
            ctx.results.push(SearchResult {
                kind: SearchResultKind::Library,
                library_id,
                library_name: model.name.clone(),
                presentation_id: None,
                presentation_name: None,
                slide_id: None,
                match_field: SearchMatchField::LibraryName,
                snippet: None,
            });
            if ctx.is_full() {
                return Ok(());
            }
        }

        // Pre-fetch presentation IDs belonging to matched libraries for later use
        let matched_library_vec: Vec<String> = ctx.matched_library_ids.iter().cloned().collect();
        if !matched_library_vec.is_empty() {
            let matched_presentations = presentation_entity::Entity::find()
                .filter(presentation_entity::Column::LibraryId.is_in(matched_library_vec))
                .all(&self.db)
                .await?;
            for model in matched_presentations {
                ctx.matched_presentation_ids.insert(model.id.clone());
            }
        }

        Ok(())
    }

    async fn search_presentations(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let remaining = ctx.remaining();
        if remaining == 0 {
            return Ok(());
        }

        let matched_library_vec: Vec<String> = ctx.matched_library_ids.iter().cloned().collect();

        let mut presentation_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            presentation_condition = presentation_condition
                .add(presentation_entity::Column::Name.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            let mut token_branch = Condition::all();
            for token in &ctx.tokens {
                token_branch = token_branch
                    .add(presentation_entity::Column::SearchName.contains(token.clone()));
            }
            presentation_condition = presentation_condition.add(token_branch);
        }
        if !matched_library_vec.is_empty() {
            presentation_condition = presentation_condition
                .add(presentation_entity::Column::LibraryId.is_in(matched_library_vec));
        }

        let presentation_rows = presentation_entity::Entity::find()
            .filter(presentation_condition)
            .order_by_asc(presentation_entity::Column::Name)
            .limit(remaining as u64)
            .find_also_related(library::Entity)
            .all(&self.db)
            .await?;

        for (presentation_model, library_model_opt) in presentation_rows {
            let library_model = match library_model_opt {
                Some(model) => model,
                None => continue,
            };
            if !ctx
                .seen_presentation_ids
                .insert(presentation_model.id.clone())
            {
                continue;
            }
            if ctx.has_tokens {
                let combined = fold_query(&format!(
                    "{} {}",
                    presentation_model.name, library_model.name
                ));
                if !ctx.tokens.iter().all(|token| combined.contains(token)) {
                    continue;
                }
            }
            let presentation_id = PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
            let library_uuid = parse_uuid(&library_model.id)?;
            let library_id = LibraryId::from_uuid(library_uuid);
            ctx.library_names
                .entry(library_model.id.clone())
                .or_insert_with(|| library_model.name.clone());
            ctx.results.push(SearchResult {
                kind: SearchResultKind::Presentation,
                library_id,
                library_name: library_model.name.clone(),
                presentation_id: Some(presentation_id),
                presentation_name: Some(presentation_model.name.clone()),
                slide_id: None,
                match_field: SearchMatchField::PresentationName,
                snippet: None,
            });
            if ctx.is_full() {
                return Ok(());
            }
        }

        Ok(())
    }

    async fn search_slides(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let remaining = ctx.remaining();
        if remaining == 0 {
            return Ok(());
        }

        let matched_presentation_vec: Vec<String> =
            ctx.matched_presentation_ids.iter().cloned().collect();

        let mut slide_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            slide_condition = slide_condition
                .add(slide_entity::Column::WorshipMain.contains(&ctx.trimmed))
                .add(slide_entity::Column::WorshipTranslate.contains(&ctx.trimmed))
                .add(slide_entity::Column::WorshipStage.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            let mut token_condition = Condition::all();
            for token in &ctx.tokens {
                let per_token = Condition::any()
                    .add(slide_entity::Column::WorshipMainSearch.contains(token.clone()))
                    .add(slide_entity::Column::WorshipTranslateSearch.contains(token.clone()))
                    .add(slide_entity::Column::WorshipStageSearch.contains(token.clone()));
                token_condition = token_condition.add(per_token);
            }
            slide_condition = slide_condition.add(token_condition);
        }
        if !matched_presentation_vec.is_empty() {
            slide_condition = slide_condition
                .add(slide_entity::Column::PresentationId.is_in(matched_presentation_vec));
        }

        let slide_rows = slide_entity::Entity::find()
            .filter(slide_condition)
            .order_by_asc(slide_entity::Column::Position)
            .limit(remaining as u64)
            .find_also_related(presentation_entity::Entity)
            .all(&self.db)
            .await?;

        let mut pending = Vec::new();
        let mut missing_library_ids: HashSet<String> = HashSet::new();

        for (slide_model, presentation_model_opt) in slide_rows {
            let presentation_model = match presentation_model_opt {
                Some(model) => model,
                None => continue,
            };
            if !ctx.seen_slide_ids.insert(slide_model.id.clone()) {
                continue;
            }
            if !ctx
                .library_names
                .contains_key(&presentation_model.library_id)
            {
                missing_library_ids.insert(presentation_model.library_id.clone());
            }
            pending.push((slide_model, presentation_model));
        }

        if !missing_library_ids.is_empty() {
            let ids: Vec<String> = missing_library_ids.into_iter().collect();
            let missing = library::Entity::find()
                .filter(library::Column::Id.is_in(ids))
                .all(&self.db)
                .await?;
            for model in missing {
                ctx.library_names
                    .insert(model.id.clone(), model.name.clone());
            }
        }

        for (slide_model, presentation_model) in pending {
            if ctx.is_full() {
                break;
            }
            let library_name = ctx
                .library_names
                .get(&presentation_model.library_id)
                .cloned()
                .unwrap_or_default();
            let library_id = LibraryId::from_uuid(parse_uuid(&presentation_model.library_id)?);
            let presentation_id = PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
            let slide_id = SlideId::from_uuid(parse_uuid(&slide_model.id)?);

            // Worship slide text fields (bible slides live in a separate table).
            let eff_main = slide_model.worship_main.as_str();
            let eff_main_search = slide_model.worship_main_search.as_str();
            let eff_translation = slide_model.worship_translate.as_str();
            let eff_translation_search = slide_model.worship_translate_search.as_str();
            let eff_stage = slide_model.worship_stage.as_str();
            let eff_stage_search = slide_model.worship_stage_search.as_str();

            if ctx.has_tokens {
                let combined = fold_query(&format!(
                    "{} {} {} {} {}",
                    library_name, presentation_model.name, eff_main, eff_translation, eff_stage
                ));
                if !ctx.tokens.iter().all(|token| combined.contains(token)) {
                    continue;
                }
            }

            let main_lower = eff_main.to_lowercase();
            let translation_lower = eff_translation.to_lowercase();
            let stage_lower = eff_stage.to_lowercase();

            let folded_ref = ctx.folded.as_str();
            let main_matches = (!ctx.query_lower.is_empty()
                && main_lower.contains(&ctx.query_lower))
                || (ctx.has_folded && eff_main_search.contains(folded_ref))
                || (ctx.has_tokens
                    && ctx
                        .tokens
                        .iter()
                        .any(|token| eff_main_search.contains(token)));
            let translation_matches = (!ctx.query_lower.is_empty()
                && translation_lower.contains(&ctx.query_lower))
                || (ctx.has_folded && eff_translation_search.contains(folded_ref))
                || (ctx.has_tokens
                    && ctx
                        .tokens
                        .iter()
                        .any(|token| eff_translation_search.contains(token)));
            let stage_matches = !eff_stage.is_empty()
                && ((!ctx.query_lower.is_empty() && stage_lower.contains(&ctx.query_lower))
                    || (ctx.has_folded && eff_stage_search.contains(folded_ref))
                    || (ctx.has_tokens
                        && ctx
                            .tokens
                            .iter()
                            .any(|token| eff_stage_search.contains(token))));

            if !main_matches && !translation_matches && !stage_matches {
                continue;
            }

            let (match_field, source_text) = if main_matches {
                (SearchMatchField::MainText, eff_main)
            } else if translation_matches {
                (SearchMatchField::TranslationText, eff_translation)
            } else {
                (SearchMatchField::StageText, eff_stage)
            };

            let snippet = build_snippet(source_text, &ctx.trimmed);

            ctx.results.push(SearchResult {
                kind: SearchResultKind::Slide,
                library_id,
                library_name: library_name.clone(),
                presentation_id: Some(presentation_id),
                presentation_name: Some(presentation_model.name.clone()),
                slide_id: Some(slide_id),
                match_field,
                snippet,
            });
        }

        Ok(())
    }
}

/// Builds a context snippet around the matched query.
/// Optimized to avoid Vec<char> allocation using char_indices() for byte slicing.
fn build_snippet(text: &str, query: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Count chars and collect byte indices only if needed
    let char_count = trimmed.chars().count();
    if char_count <= 160 {
        return Some(trimmed.to_string());
    }

    // Find match position in lowercase text (byte index)
    let lower_text = trimmed.to_lowercase();
    let lower_query = query.to_lowercase();
    let match_char_index = lower_text
        .find(&lower_query)
        .map(|byte_index| trimmed[..byte_index].chars().count())
        .unwrap_or(0);

    // Calculate char range for snippet
    let start_char = match_char_index.saturating_sub(20);
    let end_char = (start_char + 160).min(char_count);

    // Convert char indices to byte indices using char_indices() - zero allocation
    let mut char_indices = trimmed.char_indices();
    let start_byte = char_indices
        .nth(start_char)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    // Reset iterator and skip to end position
    let end_byte = trimmed
        .char_indices()
        .nth(end_char)
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());

    // Build snippet from byte slice
    let mut snippet = String::with_capacity(164); // 160 chars + potential ellipses
    if start_char > 0 {
        snippet.push('…');
    }
    snippet.push_str(&trimmed[start_byte..end_byte]);
    if end_char < char_count {
        snippet.push('…');
    }
    Some(snippet)
}
