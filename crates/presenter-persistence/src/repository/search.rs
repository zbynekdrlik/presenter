use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use presenter_core::{
    search::{fold_query, query_tokens},
    LibraryId, PresentationId, SearchMatchField, SearchResult, SearchResultKind,
};
use sea_orm::{
    sea_query::{Query, SelectStatement},
    ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

use super::util::parse_uuid;
use super::Repository;

/// SQL subquery selecting the ids of LIVE (non-tombstoned) libraries (#646).
/// Filtering `LibraryId.in_subquery(...)` with this BEFORE `.limit()` keeps
/// an excluded (live-under-tombstoned-library) row from ever consuming a
/// result slot — the OLD code applied this exclusion AFTER the SQL `LIMIT`,
/// in Rust, so a page full of excluded rows sorting early could starve a
/// genuine match sorting later in the same page.
fn live_library_ids_subquery() -> SelectStatement {
    Query::select()
        .column(library::Column::Id)
        .from(library::Entity)
        .and_where(library::Column::DeletedAt.is_null())
        .to_owned()
}

struct SearchContext {
    tokens: Vec<String>,
    has_tokens: bool,
    trimmed: String,
    cap: usize,
    results: Vec<SearchResult>,
    library_names: HashMap<String, String>,
    seen_library_ids: HashSet<String>,
    seen_presentation_ids: HashSet<String>,
    seen_slide_ids: HashSet<String>,
    /// Per token (same index as `tokens`): the ids of LIVE libraries whose
    /// folded name contains it (#833). A token may be satisfied by the
    /// library name instead of the presentation/slide text, so the SQL
    /// prefilters need these to stay EQUAL to the Rust all-tokens check.
    token_libraries: Vec<Vec<String>>,
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
        let cap = limit.clamp(1, 100) as usize;

        let mut ctx = SearchContext {
            tokens,
            has_tokens,
            trimmed: trimmed.to_string(),
            cap,
            results: Vec::with_capacity(cap),
            library_names: HashMap::with_capacity(cap),
            seen_library_ids: HashSet::with_capacity(cap),
            seen_presentation_ids: HashSet::with_capacity(cap),
            seen_slide_ids: HashSet::with_capacity(cap),
            token_libraries: Vec::new(),
        };

        self.load_token_libraries(&mut ctx).await?;
        self.search_libraries(&mut ctx).await?;
        if !ctx.is_full() {
            self.search_presentations(&mut ctx).await?;
        }
        if !ctx.is_full() {
            self.search_slides(&mut ctx).await?;
        }

        Ok(ctx.results)
    }

    /// Fill `ctx.token_libraries` (#833). Deliberately unlimited: libraries
    /// are few, and a per-token set cut by a LIMIT would make the later SQL
    /// prefilters reject real matches.
    async fn load_token_libraries(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let mut per_token = Vec::with_capacity(ctx.tokens.len());
        for token in &ctx.tokens {
            let ids: Vec<String> = library::Entity::find()
                .filter(library::Column::SearchName.contains(token.clone()))
                .filter(library::Column::DeletedAt.is_null())
                .all(&self.db)
                .await?
                .into_iter()
                .map(|model| model.id)
                .collect();
            per_token.push(ids);
        }
        tracing::debug!(
            tokens = ?ctx.tokens,
            libraries_per_token = ?per_token.iter().map(Vec::len).collect::<Vec<_>>(),
            "search: per-token library matches"
        );
        ctx.token_libraries = per_token;
        Ok(())
    }

    /// `cond` OR "the presentation's library name holds token `idx`".
    fn or_token_library(cond: Condition, ctx: &SearchContext, idx: usize) -> Condition {
        match ctx.token_libraries.get(idx) {
            Some(ids) if !ids.is_empty() => {
                cond.add(presentation_entity::Column::LibraryId.is_in(ids.iter().cloned()))
            }
            _ => cond,
        }
    }

    /// Every token in the presentation name or its library name — the exact
    /// SQL twin of `search_presentations`' Rust check (#833).
    fn presentation_tokens_condition(ctx: &SearchContext) -> Condition {
        let mut all_tokens = Condition::all();
        for (idx, token) in ctx.tokens.iter().enumerate() {
            let per_token = Condition::any()
                .add(presentation_entity::Column::SearchName.contains(token.clone()));
            all_tokens = all_tokens.add(Self::or_token_library(per_token, ctx, idx));
        }
        all_tokens
    }

    async fn search_libraries(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let mut library_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            library_condition = library_condition.add(library::Column::Name.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            // #833: ALL tokens, not any — a library listed as a result must
            // hold every token, and an any-token query let "s"/"sa" fill
            // the LIMIT with libraries the Rust check then dropped.
            let mut token_condition = Condition::all();
            for token in &ctx.tokens {
                token_condition =
                    token_condition.add(library::Column::SearchName.contains(token.clone()));
            }
            library_condition = library_condition.add(token_condition);
        }

        let library_models = library::Entity::find()
            .filter(library_condition)
            // #578 review gap: a tombstoned library must never surface in
            // search, exactly like a trashed presentation/slide.
            .filter(library::Column::DeletedAt.is_null())
            .order_by_asc(library::Column::Name)
            .limit(ctx.remaining() as u64)
            .all(&self.db)
            .await?;

        for model in library_models {
            if !ctx.seen_library_ids.insert(model.id.clone()) {
                continue;
            }
            if ctx.has_tokens {
                let haystack = fold_query(&model.name);
                if !ctx.tokens.iter().all(|token| haystack.contains(token)) {
                    continue;
                }
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

        Ok(())
    }

    async fn search_presentations(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let remaining = ctx.remaining();
        if remaining == 0 {
            return Ok(());
        }

        let mut presentation_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            presentation_condition = presentation_condition
                .add(presentation_entity::Column::Name.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            presentation_condition =
                presentation_condition.add(Self::presentation_tokens_condition(ctx));
        }

        let presentation_rows = presentation_entity::Entity::find()
            .filter(presentation_condition)
            .filter(presentation_entity::Column::DeletedAt.is_null())
            // #646: exclude a tombstoned-library row's presentations IN
            // SQL, before `.limit()` — see `live_library_ids_subquery`'s
            // doc comment. The Rust-side check below stays as a cheap belt.
            .filter(presentation_entity::Column::LibraryId.in_subquery(live_library_ids_subquery()))
            .order_by_asc(presentation_entity::Column::Name)
            .limit(remaining as u64)
            .find_also_related(library::Entity)
            .all(&self.db)
            .await?;

        for (presentation_model, library_model_opt) in presentation_rows {
            // #635: a tombstoned (soft-deleted) parent library must hide its
            // presentations from search exactly like a trashed presentation
            // hides itself — `library_model_opt` being `None` (library row
            // gone entirely) was already excluded; a LIVE `Option` wrapping
            // a TOMBSTONED row was not.
            let library_model = match library_model_opt {
                Some(model) if model.deleted_at.is_none() => model,
                _ => continue,
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

    /// Build the WHERE condition for the slide-text phase of a search.
    /// #833: equal to `emit_slide_result`'s Rust check — every token must be
    /// in the slide text, the presentation name or the library name. The old
    /// condition OR'd in every presentation of any library holding ANY token,
    /// and those rows filled the LIMIT before the real lyric was read.
    fn slide_search_condition(ctx: &SearchContext) -> Condition {
        let mut slide_condition = Condition::any();
        if !ctx.trimmed.is_empty() {
            slide_condition = slide_condition
                .add(slide_entity::Column::WorshipMain.contains(&ctx.trimmed))
                .add(slide_entity::Column::WorshipTranslate.contains(&ctx.trimmed))
                .add(slide_entity::Column::WorshipStage.contains(&ctx.trimmed));
        }
        if ctx.has_tokens {
            let mut token_condition = Condition::all();
            for (idx, token) in ctx.tokens.iter().enumerate() {
                let per_token = Condition::any()
                    .add(slide_entity::Column::WorshipMainSearch.contains(token.clone()))
                    .add(slide_entity::Column::WorshipTranslateSearch.contains(token.clone()))
                    .add(slide_entity::Column::WorshipStageSearch.contains(token.clone()))
                    .add(presentation_entity::Column::SearchName.contains(token.clone()));
                token_condition = token_condition.add(Self::or_token_library(per_token, ctx, idx));
            }
            slide_condition = slide_condition.add(token_condition);
        }
        slide_condition
    }

    /// Determine WHICH field caused a slide match (preserves diagnostic info).
    /// Priority: Main, Translation, Stage. With tokens present the search-folded
    /// fields decide; otherwise literal contains on the raw fields.
    fn classify_slide_match(
        ctx: &SearchContext,
        eff_main: &str,
        eff_main_search: &str,
        eff_translation: &str,
        eff_translation_search: &str,
        eff_stage: &str,
        eff_stage_search: &str,
    ) -> SearchMatchField {
        if ctx.has_tokens {
            if ctx
                .tokens
                .iter()
                .all(|token| eff_main_search.contains(token))
            {
                SearchMatchField::MainText
            } else if ctx
                .tokens
                .iter()
                .all(|token| eff_translation_search.contains(token))
            {
                SearchMatchField::TranslationText
            } else if ctx
                .tokens
                .iter()
                .all(|token| eff_stage_search.contains(token))
            {
                SearchMatchField::StageText
            } else {
                // Combined name+library token-match (no per-field win).
                // Default to MainText for the diagnostic field.
                SearchMatchField::MainText
            }
        } else if !ctx.trimmed.is_empty() {
            if eff_main.contains(&ctx.trimmed) {
                SearchMatchField::MainText
            } else if eff_translation.contains(&ctx.trimmed) {
                SearchMatchField::TranslationText
            } else if eff_stage.contains(&ctx.trimmed) {
                SearchMatchField::StageText
            } else {
                SearchMatchField::MainText
            }
        } else {
            SearchMatchField::MainText
        }
    }

    async fn search_slides(&self, ctx: &mut SearchContext) -> anyhow::Result<()> {
        let remaining = ctx.remaining();
        if remaining == 0 {
            return Ok(());
        }

        let slide_condition = Self::slide_search_condition(ctx);

        // #558 S10: the two name-search phases (search_libraries' matched-
        // presentations prefetch, search_presentations itself) both filter
        // DeletedAt.is_null() — the slide-TEXT phase must too, or a trashed
        // song's lyrics still surface it in results.
        let slide_rows = slide_entity::Entity::find()
            .filter(slide_condition)
            .filter(presentation_entity::Column::DeletedAt.is_null())
            // #646: same SQL-side exclusion as `search_presentations` —
            // see `live_library_ids_subquery`'s doc comment.
            .filter(presentation_entity::Column::LibraryId.in_subquery(live_library_ids_subquery()))
            .order_by_asc(slide_entity::Column::Position)
            .limit(remaining as u64)
            .find_also_related(presentation_entity::Entity)
            .all(&self.db)
            .await?;

        let (pending, missing_library_ids) = Self::collect_pending_slides(ctx, slide_rows);
        if !missing_library_ids.is_empty() {
            self.backfill_live_library_names(ctx, missing_library_ids)
                .await?;
        }

        for (slide_model, presentation_model) in pending {
            if ctx.is_full() {
                break;
            }
            Self::emit_slide_result(ctx, slide_model, presentation_model)?;
        }

        Ok(())
    }

    /// First pass over the raw slide-join rows: dedupe by slide id, drop rows
    /// whose presentation vanished (`find_also_related` returned `None`), and
    /// collect which library ids `ctx.library_names` doesn't know about yet
    /// (extracted per the #558 round-3 function-length gate; #635 split).
    fn collect_pending_slides(
        ctx: &mut SearchContext,
        slide_rows: Vec<(slide_entity::Model, Option<presentation_entity::Model>)>,
    ) -> (
        Vec<(slide_entity::Model, presentation_entity::Model)>,
        HashSet<String>,
    ) {
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
        (pending, missing_library_ids)
    }

    /// Fetch and cache the names of libraries `ctx.library_names` doesn't
    /// know about yet. #635: a TOMBSTONED library is deliberately NOT
    /// inserted — `ctx.library_names` only ever holds LIVE libraries (every
    /// phase enforces that invariant), and `emit_slide_result` skips any
    /// presentation whose library has no entry here instead of falling back
    /// to a blank name.
    async fn backfill_live_library_names(
        &self,
        ctx: &mut SearchContext,
        missing_library_ids: HashSet<String>,
    ) -> anyhow::Result<()> {
        let ids: Vec<String> = missing_library_ids.into_iter().collect();
        let missing = library::Entity::find()
            .filter(library::Column::Id.is_in(ids))
            .all(&self.db)
            .await?;
        for model in missing {
            if model.deleted_at.is_none() {
                ctx.library_names
                    .insert(model.id.clone(), model.name.clone());
            }
        }
        Ok(())
    }

    /// Classify + push ONE slide-text match into `ctx.results`, or skip it
    /// silently (no library name known — tombstoned/missing library, #635;
    /// token mismatch; or already emitted by `search_presentations`).
    /// Extracted per the #558 round-3 function-length gate; #635 split.
    fn emit_slide_result(
        ctx: &mut SearchContext,
        slide_model: slide_entity::Model,
        presentation_model: presentation_entity::Model,
    ) -> anyhow::Result<()> {
        // #635: no entry means the library is tombstoned (or otherwise
        // missing) — never fall back to a blank name, skip the slide
        // entirely, exactly like a trashed presentation is skipped.
        let Some(library_name) = ctx
            .library_names
            .get(&presentation_model.library_id)
            .cloned()
        else {
            return Ok(());
        };
        let library_id = LibraryId::from_uuid(parse_uuid(&presentation_model.library_id)?);
        let presentation_id = PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);

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
                return Ok(());
            }
        }

        // Dedupe: a presentation already emitted by search_presentations
        // (matched by name) must not be re-emitted from the slide-text
        // phase. The insert returns false if the id was already present.
        if !ctx
            .seen_presentation_ids
            .insert(presentation_model.id.clone())
        {
            return Ok(());
        }

        let match_field = Self::classify_slide_match(
            ctx,
            eff_main,
            eff_main_search,
            eff_translation,
            eff_translation_search,
            eff_stage,
            eff_stage_search,
        );

        ctx.results.push(SearchResult {
            kind: SearchResultKind::Presentation,
            library_id,
            library_name,
            presentation_id: Some(presentation_id),
            presentation_name: Some(presentation_model.name.clone()),
            slide_id: None,
            match_field,
            snippet: None,
        });
        Ok(())
    }
}
