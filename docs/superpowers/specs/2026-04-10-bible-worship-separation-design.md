# Bible/Worship Separation — Design Spec

**Issue:** #231 — Fully separate bible and worship at the database, domain, repository, and broadcasting layers
**Date:** 2026-04-10
**Approach:** Single big-bang PR (per user direction)

## Problem

Bible and worship currently share storage, types, and broadcasting code. The coupling shows up in four ways:

1. **Bible library leaks into worship UI list** (issue #227) — the operator sees a "Bible" library row in the worship library list
2. **Field overloading** — the worship `stage` field is reused for bible reference labels, and any worship slide with non-empty `stage` text incorrectly triggers a `BibleUpdate` to Resolume
3. **Per-type rules** — bible needs its own settings (character limits, default translation, etc.) but they live alongside worship
4. **Per-type outputs** — bible should write to different Resolume parameters / different stage layouts than worship

The root cause is that bible content is stored as a regular `Library` row named "Bible" with bible-specific columns (`bible_main`, `bible_main_reference`, etc.) on the same `slides` table as worship slides. There is no type discriminator anywhere — type is inferred by content inspection (`is_bible = !model.bible_main.is_empty()` at `repository/util.rs:66, 371`) and by string matching the library name in 4 places.

A dead `category` column on `libraries` exists from an aborted earlier separation attempt but is never read.

## Goal

Fully separate bible from worship at every layer:
- **Storage:** dedicated `bible_presentations` and `bible_slides` tables; the bible library wrapper is removed entirely (one bible per system)
- **Domain:** separate `BiblePresentation` / `BibleSlide` types with their own newtype IDs
- **Repository:** separate methods, no string matching
- **Broadcasting:** worship stage updates never produce `BibleUpdate`
- **UI:** worship library list never sees a bible row (because no such row exists)

## Non-goals

- Migrating existing bible content. The 2 bible presentations on production are dropped (user explicit decision).
- Per-type stage layouts and per-type Resolume parameters beyond what already exists. Pain points 3 and 4 are addressed structurally — bible already has its own broadcast type (`BibleUpdate`) and its own preferences object (`BiblePreferences`) — by removing the cross-contamination, those existing structures actually start working as intended.
- Multiple bible "libraries" or collections. There is exactly one bible per system; `bible_presentations` has no library_id.
- Splitting into a separate sqlite file. Same database, separate tables.

## Architecture

### Schema (SQLite)

New tables created by migration `m20260410_000001_separate_bible`:

```sql
CREATE TABLE IF NOT EXISTS bible_presentations (
    id          varchar(36) NOT NULL PRIMARY KEY,
    name        varchar     NOT NULL,
    created_at  timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS bible_slides (
    id                   varchar(36) NOT NULL PRIMARY KEY,
    presentation_id      varchar(36) NOT NULL,
    slide_order          integer     NOT NULL,
    main_text            text        NOT NULL,
    main_search          text        NOT NULL DEFAULT '',
    main_reference       text        NOT NULL,
    secondary_text       text        NOT NULL DEFAULT '',
    secondary_search     text        NOT NULL DEFAULT '',
    secondary_reference  text        NOT NULL DEFAULT '',
    metadata_json        text,
    FOREIGN KEY (presentation_id) REFERENCES bible_presentations(id) ON DELETE CASCADE
);

CREATE INDEX idx_bible_slides_presentation_id ON bible_slides(presentation_id);
```

The bible-specific metadata (book, chapter, verses, translation codes) is stored as JSON in `metadata_json` rather than normalized columns. This is pragmatic for an MVP — SQLite JSON functions handle the queries we currently need. Normalize later if/when we want to filter by individual metadata fields.

Removed in the same migration:
- `libraries.category` column (dead code from a previous separation attempt)
- `slides.bible_main`, `slides.bible_main_search`, `slides.bible_main_reference`, `slides.bible_translation`, `slides.bible_translation_search`, `slides.bible_translation_reference`, `slides.metadata_json`
- Any row in `libraries` where `LOWER(name) = 'bible'` (cascade-deletes its presentations and slides)

### Domain types (`crates/presenter-core/src/bible.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiblePresentationId(Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibleSlideId(Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiblePresentation {
    pub id: BiblePresentationId,
    pub name: String,
    pub slides: Vec<BibleSlide>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibleSlide {
    pub id: BibleSlideId,
    pub order: u32,
    pub main: SlideText,
    pub main_reference: String,
    pub secondary: SlideText,
    pub secondary_reference: String,
    pub metadata: Option<BibleSlideMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiblePresentationSummary {
    pub id: BiblePresentationId,
    pub name: String,
    pub slide_count: usize,
}
```

`BiblePresentationId` and `BibleSlideId` are new newtypes — they are NOT compatible with the existing `PresentationId` / `SlideId`, so the type system prevents accidentally mixing worship and bible IDs at API boundaries.

The existing `Library`, `Presentation`, `Slide`, `SlideContent` types now mean **worship only**. They keep their unprefixed names; new convention documented in `crates/presenter-core/src/lib.rs`: "unprefixed = worship; `Bible*` = bible." `SlideContent` loses bible field accessors because the columns no longer exist on the `slides` table.

### Repository (`crates/presenter-persistence/src/repository/bible.rs`)

```rust
impl Repository {
    pub async fn list_bible_presentation_summaries(&self) -> Result<Vec<BiblePresentationSummary>>;
    pub async fn fetch_bible_presentation(&self, id: BiblePresentationId) -> Result<Option<BiblePresentation>>;
    pub async fn create_bible_presentation(&self, name: &str) -> Result<BiblePresentation>;
    pub async fn rename_bible_presentation(&self, id: BiblePresentationId, name: &str) -> Result<()>;
    pub async fn delete_bible_presentation(&self, id: BiblePresentationId) -> Result<()>;
    pub async fn replace_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BibleSlide],
    ) -> Result<()>;
    pub async fn append_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BibleSlide],
    ) -> Result<BiblePresentation>;
}
```

The existing `Repository::list_libraries`, `fetch_libraries`, etc. continue to mean worship-only — no Bible filtering needed because the bible library row no longer exists.

`repository/util.rs` loses the `is_bible = !model.bible_main.is_empty()` content inspection at lines 66 and 371. Worship `to_domain_slide` and `build_slide_active_model` simplify because they no longer have to branch on type.

### State layer

`crates/presenter-server/src/state/bible.rs:198-244`: replace string-based library lookup with direct calls to the new `bible_presentations` repository methods.

`crates/presenter-server/src/state/broadcasting.rs:83-96`: **delete the entire block** that emits `BibleUpdate` based on the worship slide's `stage` field. Worship stage broadcasts only emit `StageUpdate`. Bible broadcasts only happen via the dedicated `state/bible.rs` trigger code path.

`crates/presenter-server/src/ai/tools.rs:375, 685`: replace `lib.name.eq_ignore_ascii_case("bible")` magic strings with proper API calls (call `list_bible_presentation_summaries` if you need to know about bible presentations; never reference the worship library list when looking for bible content).

### API / router

`crates/presenter-server/src/router/bible.rs` already exists with the full `/bible/*` route surface. Handlers get updated to use `BiblePresentation` / `BibleSlide` and the new repository methods. **No new routes, no removed routes** — the API surface is unchanged.

### UI

**No code changes needed.** After the migration, the bible library row is gone from the `libraries` table, so:
- Worship `LibraryList` (`crates/presenter-ui/src/components/library_list.rs`) never sees Bible — fixes #227 implicitly
- Bible UI continues to work via `/bible/*` endpoints
- The two operator tabs (`<section data-view-panel="worship">` / `<section data-view-panel="bible">`) keep their existing wiring

## Migration

Single file: `crates/presenter-migration/src/m20260410_000001_separate_bible.rs`. Registered in `lib.rs` after the existing `m20260408_000001_add_preach_limit`.

```rust
async fn up() {
    // 1. Create new tables (idempotent)
    create_table_if_not_exists("bible_presentations");
    create_table_if_not_exists("bible_slides");
    create_index_if_not_exists("idx_bible_slides_presentation_id");

    // 2. Drop the bible library row + cascade-delete its presentations/slides
    DELETE FROM libraries WHERE LOWER(name) = 'bible';

    // 3. Drop the now-unused bible_* columns from slides
    ALTER TABLE slides DROP COLUMN bible_main;
    ALTER TABLE slides DROP COLUMN bible_main_search;
    ALTER TABLE slides DROP COLUMN bible_main_reference;
    ALTER TABLE slides DROP COLUMN bible_translation;
    ALTER TABLE slides DROP COLUMN bible_translation_search;
    ALTER TABLE slides DROP COLUMN bible_translation_reference;
    ALTER TABLE slides DROP COLUMN metadata_json;

    // 4. Drop the dead category column from libraries
    ALTER TABLE libraries DROP COLUMN category;
}
```

**Idempotency:** `IF NOT EXISTS` for table creation. The `DELETE FROM libraries` is naturally idempotent (no rows to delete on re-run). The column drops are guarded by checking column existence first (sea-orm-migration patterns from the existing `m20260408_000001_add_preach_limit` migration).

**SQLite version:** `ALTER TABLE DROP COLUMN` requires SQLite 3.35+ (released March 2021). All current sea-orm sqlite drivers are well above this.

**Down migration:** Recreates the dropped columns and the empty bible library wrapper. **Lossy** — does not restore data. The down() exists for testing and hypothetical rollback, not for round-tripping production data.

**Production safety:**
- The dev pipeline takes a backup before deploy (`pipeline.yml:761-776`)
- The dev pipeline tests migrations against a copy of the prod DB (`pipeline.yml:778-804`) — this catches schema-incompatibility errors before they touch dev, let alone prod
- The user has explicitly confirmed dropping the 2 existing bible presentations on production

## Tests

### Unit (Rust)

- **Repository:** create/read/update/delete/append for `bible_presentations` and `bible_slides` (covers all 7 new methods)
- **Migration:** test that runs the new migration against a fresh database, verifies tables exist, verifies old columns are gone
- **Migration on real schema:** test that runs against a copy of the existing schema (with bible library row + bible_* columns populated) and verifies the cleanup completes

### Regression tests (the heart of this PR)

- **Bible-out-of-worship-list:** integration test that creates a bible presentation via `POST /bible/presentations`, then calls `GET /libraries/summary`, asserts the response does NOT contain any library named "Bible"
- **Broadcasting leak:** unit test that creates a worship slide with non-empty `stage` text, calls the broadcasting code path, asserts the resolume registry's `bible_update` was NOT invoked. This is the regression guard for the field-overloading bug.

### E2E (Playwright)

- Existing bible E2E tests (`bible-presentation-append.spec.ts`, `bible-trigger-slide.spec.ts`) should continue to pass — they create their own bible presentations via `/bible/presentations`, which now hits the new tables transparently.
- New E2E test: navigate to worship operator, create a bible presentation in another tab, refresh worship operator, assert no Bible library appears in the library list.

## Risks & Rollback

| Risk | Severity | Mitigation |
|------|----------|------------|
| Migration crashes mid-run | LOW | All steps idempotent. IF NOT EXISTS for tables, DELETE is naturally re-runnable, ALTER DROP is guarded by column existence check. Crash + restart = picks up where it left off. |
| Old binary deployed against new schema | MEDIUM | Old binary's `slide::Model` references `bible_main` etc., would fail at startup. **Rollback path:** restore from the auto-backup taken in `Backup database` step before deploy. The dev DB also gets replaced from prod every deploy, so dev is never in a worse state than prod was. |
| Production data loss | ACCEPTED | User explicitly confirmed dropping the 2 existing bible presentations. Backup is still taken. |
| Companion / Bitfocus integration breaks | LOW | Companion API for bible already uses `/bible/*` routes. AI tool magic string checks get replaced with proper API calls in the same PR. |
| ai/tools.rs magic string regression | LOW | Search-and-replace scope is small (2 references). Linter / `cargo check` catches it. |

## Concrete file impact (~12 files)

| File | Change |
|------|--------|
| `crates/presenter-migration/src/m20260410_000001_separate_bible.rs` | NEW: migration |
| `crates/presenter-migration/src/lib.rs` | Register new migration |
| `crates/presenter-persistence/src/entities.rs` | Add bible_presentation/bible_slide entities, drop bible_* columns from slide entity |
| `crates/presenter-persistence/src/repository/bible.rs` | NEW: bible-specific repository methods |
| `crates/presenter-persistence/src/repository/mod.rs` | Re-export bible methods |
| `crates/presenter-persistence/src/repository/util.rs` | Drop is_bible content inspection (lines 66, 371) |
| `crates/presenter-core/src/bible.rs` | Add BiblePresentation, BibleSlide, BiblePresentationId, BibleSlideId types |
| `crates/presenter-core/src/lib.rs` | Export new types, document the unprefixed=worship convention |
| `crates/presenter-server/src/state/bible.rs` | Replace string lookups (lines 198-244) with new repo methods |
| `crates/presenter-server/src/state/broadcasting.rs` | Delete the BibleUpdate-from-stage-field leak (lines 83-96) |
| `crates/presenter-server/src/router/bible.rs` | Update handlers to use new types |
| `crates/presenter-server/src/ai/tools.rs` | Replace magic string checks (lines 375, 685) |
| `tests/e2e/bible-presentation-append.spec.ts` etc. | Verify still passing; add new regression test for #227 |
| `Cargo.toml` | Version bump 0.4.14 → 0.4.15 |

## Verification checklist

After deploy to production:
- [ ] `/healthz` returns version 0.4.15
- [ ] Worship library list does NOT contain "Bible"
- [ ] Bible UI loads, can create/edit/trigger bible presentations via `/bible/presentations`
- [ ] Trigger a worship slide with non-empty stage text → Resolume bible clips do NOT change
- [ ] Trigger a bible presentation → Resolume bible clips DO change
- [ ] Existing bible E2E tests pass on the new schema
