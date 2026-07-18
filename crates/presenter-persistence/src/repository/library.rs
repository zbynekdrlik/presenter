use crate::entities::{
    library, library_favorite, presentation as presentation_entity, slide as slide_entity,
};
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{
    search::fold_query, Library, LibraryId, LibrarySummary, Presentation, PresentationId,
    PresentationSummary,
};
use sea_orm::{
    sea_query::OnConflict, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

use super::util::{
    build_slide_active_model, parse_uuid, sanitize_like_input, to_domain_slide, RepositoryError,
};
use super::Repository;

/// A trashed-or-live row's `(deleted_at, updated_at)` timestamps.
type RowState = (
    Option<chrono::DateTime<chrono::FixedOffset>>,
    chrono::DateTime<chrono::FixedOffset>,
);

/// One narrow `(sync_id, updated_at, deleted_at)` projection row from
/// `presentations` — #558 R7 (never fetch the full row just for these three
/// columns).
type OldPresentationRow = (
    String,
    chrono::DateTime<chrono::FixedOffset>,
    Option<chrono::DateTime<chrono::FixedOffset>>,
);

/// Trash/edit state captured from the OLD library being replaced BEFORE it
/// is deleted (#558 S2), keyed by `sync_id` ONLY.
///
/// #558 round-3 DESIGN SIMPLIFICATION (Decision A): an earlier revision
/// (R1) also kept a `by_name` fallback map so a trashed song would still be
/// found even when a re-import shifted its `sync_id` (a same-name twin
/// joining the scan makes BOTH occurrences derive fresh ids under S3's
/// cardinality-sensitive rule). Round-3 review found that fallback
/// unfixable by patching — T2/T4/T5/T6 were four INDEPENDENT failures of
/// the same mechanism (sibling-key leakage, name recycling, scan-order
/// dependence, old-map name collisions) — so it was deleted wholesale
/// instead of patched again.
///
/// The simplified rule: trash carryover on re-import is keyed by `sync_id`
/// ONLY. If a trashed song's `sync_id` SHIFTS on re-import — a corner of a
/// corner: it requires a same-name twin to join the import while the song
/// sits in trash — the song comes back LIVE. That outcome is
/// understandable ("re-import restored a song because the library file
/// still contains it") and composes safely with sync: the peer still holds
/// the OLD id as a fresh tombstone, which Decision B (`sync_apply.rs`)
/// applies as its OWN new trashed row rather than reaching for any
/// existing local row by name. Both sites converge to
/// new-id-live + old-id-trashed, with no scan-order dependence and no
/// wrongly-trashed live song, ever.
///
/// #558 round-4 U8: a plain alias — the old wrapper struct (with its own
/// `empty()`/`take()`) was a pass-through with no behavior beyond what
/// `HashMap` already provides; callers use `.remove(&sync_id)` directly.
type OldTrashState = HashMap<String, RowState>;

impl Repository {
    #[instrument(skip_all)]
    pub async fn upsert_library(&self, library: &Library) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        // Replace any existing library that collides with this one — matched by
        // id OR by name. The importer assigns a fresh random LibraryId on every
        // run (`Library::new`), so a library that already exists under the same
        // NAME but a different id MUST be removed before the insert below, or it
        // fails on the `idx_libraries_name_unique` UNIQUE(name) constraint
        // (#463). Deleting the colliding library row cascades to its
        // presentations and slides (ON DELETE CASCADE in the schema; foreign_keys
        // is on by default on our sqlx connections), scoped to that one library
        // only — never a global purge, so other libraries and playlists are
        // untouched.
        let stale_library_ids: Vec<String> = library::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(library::Column::Id.eq(library.id.to_string()))
                    .add(library::Column::Name.eq(library.name.clone())),
            )
            .all(&txn)
            .await?
            .into_iter()
            .map(|model| model.id)
            .collect();

        // #558 S2: capture the OLD library's trash/edit state BEFORE deleting
        // it, keyed by sync_id — a re-import restores CONTENT but must never
        // resurrect a tombstone or manufacture a newer edit-time for an
        // already-trashed song.
        let mut old_trash_state = fetch_old_trash_state(&txn, &stale_library_ids).await?;

        if !stale_library_ids.is_empty() {
            library::Entity::delete_many()
                .filter(library::Column::Id.is_in(stale_library_ids))
                .exec(&txn)
                .await?;
        }

        let lib_model = library::ActiveModel {
            id: Set(library.id.to_string()),
            name: Set(library.name.clone()),
            search_name: Set(fold_query(&library.name)),
            created_at: Set(Utc::now().into()),
        };
        library::Entity::insert(lib_model).exec(&txn).await?;

        let sync_ids = resolve_content_pure_sync_ids(&txn, library).await?;
        for (presentation, sync_id) in library.presentations.iter().zip(sync_ids) {
            insert_presentation_with_slides(
                &txn,
                library,
                presentation,
                sync_id,
                &mut old_trash_state,
            )
            .await?;
        }

        txn.commit().await?;

        // #515: a name-colliding re-import cascade-deleted the old library's
        // slides and recreated them under fresh UUIDs — sweep the marker rows
        // that now point at dead slide ids so they never accumulate.
        self.prune_orphan_slide_stage_layouts().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn create_library(&self, name: &str) -> anyhow::Result<Library> {
        let txn = self.db.begin().await?;
        let id = LibraryId::new();

        let model = library::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
        };

        library::Entity::insert(model).exec(&txn).await?;
        txn.commit().await?;

        let library = Library::new(name.to_string(), Vec::new())?.with_id(id);
        Ok(library)
    }

    #[instrument(skip_all)]
    pub async fn rename_library(&self, library_id: LibraryId, name: &str) -> anyhow::Result<()> {
        let id = library_id.to_string();
        let mut model = library::Entity::find_by_id(id.clone())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("library not found"))?
            .into_active_model();
        model.name = Set(name.to_string());
        model.search_name = Set(fold_query(name));
        model.update(&self.db).await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_library(&self, library_id: LibraryId) -> anyhow::Result<()> {
        let id = library_id.to_string();
        let result = library::Entity::delete_by_id(id).exec(&self.db).await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("library not found"));
        }
        // #515: the FK cascade removed the library's slides — sweep their
        // stage-layout marker rows (no FK on slide_stage_layouts).
        self.prune_orphan_slide_stage_layouts().await?;
        Ok(())
    }

    /// Fetches all libraries with presentations and slides using batch queries.
    /// Optimized to use 3 queries total instead of 1 + n + (n*m) queries.
    #[instrument(skip_all)]
    pub async fn fetch_libraries(&self) -> anyhow::Result<Vec<Library>> {
        // Query 1: Fetch all libraries
        let libraries = library::Entity::find()
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        if libraries.is_empty() {
            return Ok(Vec::new());
        }

        let library_ids: Vec<String> = libraries.iter().map(|lib| lib.id.clone()).collect();

        // Query 2: Batch fetch all presentations for these libraries
        let all_presentations = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.is_in(library_ids))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .order_by_asc(presentation_entity::Column::Name)
            .all(&self.db)
            .await?;

        let presentation_ids: Vec<String> =
            all_presentations.iter().map(|p| p.id.clone()).collect();

        // Query 3: Batch fetch all slides for these presentations
        let all_slides = if presentation_ids.is_empty() {
            Vec::new()
        } else {
            slide_entity::Entity::find()
                .filter(slide_entity::Column::PresentationId.is_in(presentation_ids))
                .order_by_asc(slide_entity::Column::Position)
                .all(&self.db)
                .await?
        };

        // Group slides by presentation_id in memory
        let mut slides_by_presentation: HashMap<String, Vec<slide_entity::Model>> =
            HashMap::with_capacity(all_presentations.len());
        for slide in all_slides {
            slides_by_presentation
                .entry(slide.presentation_id.clone())
                .or_default()
                .push(slide);
        }

        // Group presentations by library_id in memory
        let mut presentations_by_library: HashMap<String, Vec<presentation_entity::Model>> =
            HashMap::with_capacity(libraries.len());
        for pres in all_presentations {
            presentations_by_library
                .entry(pres.library_id.clone())
                .or_default()
                .push(pres);
        }

        // Build domain models
        let mut results = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentations = presentations_by_library.remove(&lib.id).unwrap_or_default();

            let mut presentation_models = Vec::with_capacity(presentations.len());
            for pres in presentations {
                let slides = slides_by_presentation.remove(&pres.id).unwrap_or_default();

                let slide_models = slides
                    .into_iter()
                    .map(to_domain_slide)
                    .collect::<Result<Vec<_>, RepositoryError>>()?;

                let presentation = Presentation::new(pres.name.clone(), slide_models)?
                    .with_id(PresentationId::from_uuid(parse_uuid(&pres.id)?));
                presentation_models.push(presentation);
            }

            let library_domain = Library::new(lib.name.clone(), presentation_models)?
                .with_id(LibraryId::from_uuid(parse_uuid(&lib.id)?));
            results.push(library_domain);
        }

        Ok(results)
    }

    #[instrument(skip_all)]
    pub async fn list_library_favorites(&self) -> anyhow::Result<Vec<LibraryId>> {
        let favorites = library_favorite::Entity::find()
            .order_by_asc(library_favorite::Column::LibraryId)
            .all(&self.db)
            .await?;
        let mut ids = Vec::with_capacity(favorites.len());
        for fav in favorites {
            let uuid = parse_uuid(&fav.library_id)?;
            ids.push(LibraryId::from_uuid(uuid));
        }
        Ok(ids)
    }

    #[instrument(skip_all)]
    pub async fn set_library_favorite(
        &self,
        library_id: LibraryId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let id_string = library_id.to_string();

        if favorite {
            library_favorite::Entity::insert(library_favorite::ActiveModel {
                library_id: Set(id_string.clone()),
            })
            .on_conflict(
                OnConflict::column(library_favorite::Column::LibraryId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        } else {
            library_favorite::Entity::delete_by_id(id_string.clone())
                .exec(&txn)
                .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    /// Lists library summaries with presentation counts using batch queries.
    /// Optimized to use 2 queries total instead of 1 + n queries.
    #[instrument(skip_all)]
    pub async fn list_library_summaries(
        &self,
        filter: Option<&str>,
    ) -> anyhow::Result<Vec<LibrarySummary>> {
        // Query 1: Fetch libraries (with optional filter)
        let mut query = library::Entity::find();
        if let Some(filter) = filter {
            let pattern = format!("%{}%", sanitize_like_input(filter));
            query = query.filter(library::Column::Name.like(pattern));
        }

        let libraries = query
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        if libraries.is_empty() {
            return Ok(Vec::new());
        }

        let library_ids: Vec<String> = libraries.iter().map(|lib| lib.id.clone()).collect();

        // Query 2: Batch fetch all presentations for these libraries
        let all_presentations = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.is_in(library_ids))
            .filter(presentation_entity::Column::DeletedAt.is_null())
            .order_by_asc(presentation_entity::Column::Name)
            .all(&self.db)
            .await?;

        // Group presentations by library_id in memory
        let mut presentations_by_library: HashMap<String, Vec<presentation_entity::Model>> =
            HashMap::with_capacity(libraries.len());
        for pres in all_presentations {
            presentations_by_library
                .entry(pres.library_id.clone())
                .or_default()
                .push(pres);
        }

        // Build summaries
        let mut summaries = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentations = presentations_by_library.remove(&lib.id).unwrap_or_default();

            let mut presentation_summaries = Vec::with_capacity(presentations.len());
            for pres in &presentations {
                let pres_id = PresentationId::from_uuid(parse_uuid(&pres.id)?);
                presentation_summaries.push(PresentationSummary::new(pres_id, pres.name.clone()));
            }

            let library_id = LibraryId::from_uuid(parse_uuid(&lib.id)?);
            let summary = LibrarySummary::new(
                library_id,
                lib.name.clone(),
                presentation_summaries.len(),
                presentation_summaries,
            );
            summaries.push(summary);
        }

        Ok(summaries)
    }
}

/// #558 S2 (round-3 Decision A): read the OLD library rows (about to be
/// replaced) BEFORE they are deleted, keyed by `sync_id` ONLY, so the
/// caller can carry over `deleted_at` / `updated_at` for any song that
/// re-imports under the SAME identity. A re-import restores CONTENT but
/// must never resurrect a tombstone nor manufacture a newer edit-time for
/// an already-trashed song — unless the re-import itself shifted the
/// song's `sync_id` (see `OldTrashState`'s doc), in which case the song is,
/// by design, a fresh identity.
async fn fetch_old_trash_state(
    txn: &DatabaseTransaction,
    stale_library_ids: &[String],
) -> anyhow::Result<OldTrashState> {
    if stale_library_ids.is_empty() {
        return Ok(OldTrashState::new());
    }
    // #558 R7 (round-3): project only sync_id/updated_at/deleted_at — the
    // by-name map (and the library-name lookup it needed) is gone.
    let old_rows: Vec<OldPresentationRow> = presentation_entity::Entity::find()
        .select_only()
        .column(presentation_entity::Column::SyncId)
        .column(presentation_entity::Column::UpdatedAt)
        .column(presentation_entity::Column::DeletedAt)
        .filter(presentation_entity::Column::LibraryId.is_in(stale_library_ids.to_vec()))
        .into_tuple()
        .all(txn)
        .await?;

    let mut state = OldTrashState::new();
    for (sync_id, updated_at, deleted_at) in old_rows {
        state.insert(sync_id, (deleted_at, updated_at));
    }
    Ok(state)
}

/// #558 S3: resolve a content-pure `sync_id` for every presentation in
/// `library`, in order. A raw id (the `.pro` UUID, or the deterministic
/// name-based fallback) is kept ONLY when it is unique across THIS library's
/// own import scan AND does not already belong to a DIFFERENT, still-existing
/// library in the DB. Every other occurrence — an in-scan duplicate, or a
/// foreign-DB conflict — derives `UUIDv5(raw/library/name)`, with NO
/// exception for "the first one" — the old dedupe's first-occurrence-wins
/// rule was exactly the import-order/history dependence #558 flags. This is
/// a pure function of content: neither the order presentations appear in the
/// list, nor which library was imported into this DB first, changes the
/// outcome for a raw id that only collides WITHIN this call.
async fn resolve_content_pure_sync_ids(
    txn: &DatabaseTransaction,
    library: &Library,
) -> anyhow::Result<Vec<String>> {
    let desired_raw_ids: Vec<String> = library
        .presentations
        .iter()
        .map(|presentation| {
            presentation
                .sync_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    presenter_core::sync_id_for_name(&library.name, &presentation.name)
                })
        })
        .collect();

    let mut raw_occurrences: HashMap<&str, u32> = HashMap::new();
    for raw in &desired_raw_ids {
        *raw_occurrences.entry(raw.as_str()).or_insert(0) += 1;
    }

    // Read-only snapshot: sync_ids already used by any OTHER library still in
    // the DB (this library's own stale rows were already deleted by the
    // caller). Consulted only to AVOID a unique-index violation — never to
    // implement "first come" claiming.
    let foreign_sync_ids: HashSet<String> = presentation_entity::Entity::find()
        .select_only()
        .column(presentation_entity::Column::SyncId)
        .into_tuple::<String>()
        .all(txn)
        .await?
        .into_iter()
        .collect();

    let mut assigned_this_scan: HashSet<String> = HashSet::new();
    let mut sync_ids = Vec::with_capacity(library.presentations.len());
    for (presentation, raw) in library.presentations.iter().zip(desired_raw_ids.iter()) {
        let unique_in_scan = raw_occurrences.get(raw.as_str()).copied() == Some(1);
        let sync_id = if unique_in_scan && !foreign_sync_ids.contains(raw) {
            raw.clone()
        } else {
            derive_sync_id(
                raw,
                &library.name,
                &presentation.name,
                &assigned_this_scan,
                &foreign_sync_ids,
            )
        };
        assigned_this_scan.insert(sync_id.clone());
        sync_ids.push(sync_id);
    }
    Ok(sync_ids)
}

/// Deterministically derive a non-raw `sync_id` for a presentation whose raw
/// id is not content-pure-unique (#558 S3): `UUIDv5(ns, raw/library/name)`,
/// with a deterministic occurrence counter (`.../2`, `.../3`, …) appended only
/// in the astronomically rare case that the derived id ITSELF collides.
/// `assigned`/`foreign` are consulted purely to avoid that collision — never
/// to grant "first come" priority.
fn derive_sync_id(
    raw: &str,
    library_name: &str,
    presentation_name: &str,
    assigned: &HashSet<String>,
    foreign: &HashSet<String>,
) -> String {
    let mut k = 1u32;
    loop {
        let key = if k == 1 {
            format!("{raw}/{library_name}/{presentation_name}")
        } else {
            format!("{raw}/{library_name}/{presentation_name}/{k}")
        };
        let candidate =
            uuid::Uuid::new_v5(&presenter_core::SYNC_ID_NAMESPACE, key.as_bytes()).to_string();
        if !assigned.contains(&candidate) && !foreign.contains(&candidate) {
            return candidate;
        }
        k += 1;
    }
}

/// Insert one presentation row + its slides, carrying over the OLD
/// trash/edit state for its (already resolved, content-pure) `sync_id` if a
/// prior row under that identity existed (#558 S2) — otherwise it is a brand
/// new song (`deleted_at: None`, `updated_at: now()`).
async fn insert_presentation_with_slides(
    txn: &DatabaseTransaction,
    library: &Library,
    presentation: &Presentation,
    sync_id: String,
    old_trash_state: &mut OldTrashState,
) -> anyhow::Result<()> {
    let (deleted_at, updated_at) = old_trash_state
        .remove(&sync_id)
        .unwrap_or((None, Utc::now().into()));

    let pres_model = presentation_entity::ActiveModel {
        id: Set(presentation.id.to_string()),
        library_id: Set(library.id.to_string()),
        name: Set(presentation.name.clone()),
        search_name: Set(fold_query(&presentation.name)),
        created_at: Set(Utc::now().into()),
        updated_at: Set(updated_at),
        sync_id: Set(sync_id),
        deleted_at: Set(deleted_at),
    };
    presentation_entity::Entity::insert(pres_model)
        .exec(txn)
        .await?;

    for slide in &presentation.slides {
        let pres_id_str = presentation.id.to_string();
        let slide_model = build_slide_active_model(slide, &pres_id_str, slide.order as i32);
        slide_entity::Entity::insert(slide_model).exec(txn).await?;
    }
    Ok(())
}
