use std::collections::{HashMap, HashSet};
use tracing::instrument;

use crate::entities::{
    library, playlist, playlist_entry, playlist_favorite, presentation as presentation_entity,
    slide as slide_entity,
};
use presenter_core::{
    playlist::{MidiBinding, PlaylistEntryKind},
    search::{fold_query, query_tokens},
    LibraryId, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId, PresentationId,
    SearchMatchField, SearchResult, SearchResultKind, SlideId,
};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::util::parse_uuid;
use super::Repository;

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
        let mut results = Vec::with_capacity(cap);
        let mut library_names: HashMap<String, String> = HashMap::new();
        let mut seen_library_ids: HashSet<String> = HashSet::new();
        let mut seen_presentation_ids: HashSet<String> = HashSet::new();
        let mut seen_slide_ids: HashSet<String> = HashSet::new();
        let mut matched_library_ids: HashSet<String> = HashSet::new();

        let mut remaining = cap;

        if remaining == 0 {
            return Ok(results);
        }

        let mut library_condition = Condition::any();
        if !trimmed.is_empty() {
            library_condition = library_condition.add(library::Column::Name.contains(trimmed));
        }
        if has_tokens {
            let mut token_condition = Condition::any();
            for token in &tokens {
                token_condition =
                    token_condition.add(library::Column::SearchName.contains(token.clone()));
            }
            library_condition = library_condition.add(token_condition);
        }

        let library_models = library::Entity::find()
            .filter(library_condition)
            .order_by_asc(library::Column::Name)
            .limit(remaining as u64)
            .all(&self.db)
            .await?;

        for model in library_models {
            if !seen_library_ids.insert(model.id.clone()) {
                continue;
            }
            let tokens_in_name = if has_tokens {
                let haystack = fold_query(&model.name);
                tokens
                    .iter()
                    .filter(|token| haystack.contains(*token))
                    .count()
            } else {
                0
            };
            if tokens_in_name > 0 {
                matched_library_ids.insert(model.id.clone());
            }
            if has_tokens && tokens_in_name < tokens.len() {
                continue;
            }
            let library_id = LibraryId::from_uuid(parse_uuid(&model.id)?);
            library_names.insert(model.id.clone(), model.name.clone());
            results.push(SearchResult {
                kind: SearchResultKind::Library,
                library_id,
                library_name: model.name.clone(),
                presentation_id: None,
                presentation_name: None,
                slide_id: None,
                match_field: SearchMatchField::LibraryName,
                snippet: None,
            });
            if results.len() >= cap {
                return Ok(results);
            }
        }

        let matched_library_vec: Vec<String> = matched_library_ids.iter().cloned().collect();
        let mut matched_presentation_ids: HashSet<String> = HashSet::new();
        if !matched_library_vec.is_empty() {
            let matched_presentations = presentation_entity::Entity::find()
                .filter(presentation_entity::Column::LibraryId.is_in(matched_library_vec.clone()))
                .all(&self.db)
                .await?;
            for model in matched_presentations {
                matched_presentation_ids.insert(model.id.clone());
            }
        }
        let matched_presentation_vec: Vec<String> =
            matched_presentation_ids.iter().cloned().collect();

        if results.len() < cap {
            remaining = cap - results.len();
            if remaining > 0 {
                let mut presentation_condition = Condition::any();
                if !trimmed.is_empty() {
                    presentation_condition = presentation_condition
                        .add(presentation_entity::Column::Name.contains(trimmed));
                }
                if has_tokens {
                    let mut token_branch = Condition::all();
                    for token in &tokens {
                        token_branch = token_branch
                            .add(presentation_entity::Column::SearchName.contains(token.clone()));
                    }
                    presentation_condition = presentation_condition.add(token_branch);
                }
                if !matched_library_vec.is_empty() {
                    presentation_condition = presentation_condition.add(
                        presentation_entity::Column::LibraryId.is_in(matched_library_vec.clone()),
                    );
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
                    if !seen_presentation_ids.insert(presentation_model.id.clone()) {
                        continue;
                    }
                    if has_tokens {
                        let combined = fold_query(&format!(
                            "{} {}",
                            presentation_model.name, library_model.name
                        ));
                        if !tokens.iter().all(|token| combined.contains(token)) {
                            continue;
                        }
                    }
                    let presentation_id =
                        PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
                    let library_uuid = parse_uuid(&library_model.id)?;
                    let library_id = LibraryId::from_uuid(library_uuid);
                    library_names
                        .entry(library_model.id.clone())
                        .or_insert_with(|| library_model.name.clone());
                    results.push(SearchResult {
                        kind: SearchResultKind::Presentation,
                        library_id,
                        library_name: library_model.name.clone(),
                        presentation_id: Some(presentation_id),
                        presentation_name: Some(presentation_model.name.clone()),
                        slide_id: None,
                        match_field: SearchMatchField::PresentationName,
                        snippet: None,
                    });
                    if results.len() >= cap {
                        return Ok(results);
                    }
                }
            }
        }

        if results.len() < cap {
            remaining = cap - results.len();
            if remaining > 0 {
                let mut slide_condition = Condition::any();
                if !trimmed.is_empty() {
                    slide_condition = slide_condition
                        .add(slide_entity::Column::MainText.contains(trimmed))
                        .add(slide_entity::Column::TranslationText.contains(trimmed))
                        .add(slide_entity::Column::StageText.contains(trimmed));
                }
                if has_tokens {
                    let mut token_condition = Condition::all();
                    for token in &tokens {
                        let per_token = Condition::any()
                            .add(slide_entity::Column::MainTextSearch.contains(token.clone()))
                            .add(
                                slide_entity::Column::TranslationTextSearch.contains(token.clone()),
                            )
                            .add(slide_entity::Column::StageTextSearch.contains(token.clone()));
                        token_condition = token_condition.add(per_token);
                    }
                    slide_condition = slide_condition.add(token_condition);
                }
                if !matched_presentation_vec.is_empty() {
                    slide_condition = slide_condition.add(
                        slide_entity::Column::PresentationId
                            .is_in(matched_presentation_vec.clone()),
                    );
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
                    if !seen_slide_ids.insert(slide_model.id.clone()) {
                        continue;
                    }
                    if !library_names.contains_key(&presentation_model.library_id) {
                        missing_library_ids.insert(presentation_model.library_id.clone());
                    }
                    pending.push((slide_model, presentation_model));
                }

                if !missing_library_ids.is_empty() {
                    let ids: Vec<String> = missing_library_ids.into_iter().collect();
                    let missing = library::Entity::find()
                        .filter(library::Column::Id.is_in(ids.clone()))
                        .all(&self.db)
                        .await?;
                    for model in missing {
                        library_names.insert(model.id.clone(), model.name.clone());
                    }
                }

                for (slide_model, presentation_model) in pending {
                    if results.len() >= cap {
                        break;
                    }
                    let library_name = library_names
                        .get(&presentation_model.library_id)
                        .cloned()
                        .unwrap_or_default();
                    let library_id =
                        LibraryId::from_uuid(parse_uuid(&presentation_model.library_id)?);
                    let presentation_id =
                        PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
                    let slide_id = SlideId::from_uuid(parse_uuid(&slide_model.id)?);

                    if has_tokens {
                        let combined = fold_query(&format!(
                            "{} {} {} {} {}",
                            library_name,
                            presentation_model.name,
                            slide_model.main_text,
                            slide_model.translation_text,
                            slide_model.stage_text
                        ));
                        if !tokens.iter().all(|token| combined.contains(token)) {
                            continue;
                        }
                    }

                    let main_lower = slide_model.main_text.to_lowercase();
                    let translation_lower = slide_model.translation_text.to_lowercase();
                    let stage_lower = slide_model.stage_text.to_lowercase();

                    let folded_ref = folded.as_str();
                    let main_matches = (!query_lower.is_empty()
                        && main_lower.contains(&query_lower))
                        || (has_folded && slide_model.main_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.main_text_search.contains(token)));
                    let translation_matches = (!query_lower.is_empty()
                        && translation_lower.contains(&query_lower))
                        || (has_folded && slide_model.translation_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.translation_text_search.contains(token)));
                    let stage_matches = (!query_lower.is_empty()
                        && stage_lower.contains(&query_lower))
                        || (has_folded && slide_model.stage_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.stage_text_search.contains(token)));

                    if !main_matches && !translation_matches && !stage_matches {
                        continue;
                    }

                    let (match_field, source_text) = if main_matches {
                        (SearchMatchField::MainText, slide_model.main_text.as_str())
                    } else if translation_matches {
                        (
                            SearchMatchField::TranslationText,
                            slide_model.translation_text.as_str(),
                        )
                    } else {
                        (SearchMatchField::StageText, slide_model.stage_text.as_str())
                    };

                    let snippet = build_snippet(source_text, trimmed);

                    results.push(SearchResult {
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
            }
        }

        Ok(results)
    }
}

fn build_snippet(text: &str, query: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 160 {
        return Some(trimmed.to_string());
    }

    let lower_text = trimmed.to_lowercase();
    let lower_query = query.to_lowercase();
    let match_char_index = lower_text
        .find(&lower_query)
        .map(|byte_index| trimmed[..byte_index].chars().count())
        .unwrap_or(0);

    let start = match_char_index.saturating_sub(20);
    let end = (start + 160).min(chars.len());

    let mut snippet: String = chars[start..end].iter().collect();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    Some(snippet)
}
