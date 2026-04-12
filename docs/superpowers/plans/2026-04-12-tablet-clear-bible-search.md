# Tablet Clear Button + Bible Search Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a bible clear button to the tablet UI and replace the slow LIKE-based bible search with SQLite FTS5 full-text search.

**Architecture:** Two independent fixes: (1) Add a clear button to `TabletHeader` that calls the existing `POST /bible/clear` API, (2) Add an FTS5 virtual table via migration, add sync triggers, replace the `search_bible_passages` repository method with FTS5 MATCH query.

**Tech Stack:** Rust/Leptos WASM (presenter-ui), CSS, SeaORM migrations, SQLite FTS5

**Spec:** `docs/superpowers/specs/2026-04-12-tablet-clear-bible-search-design.md`

---

## Context

- **#222:** Tablet UI at `crates/presenter-ui/src/pages/tablet.rs` has no clear button. `TabletHeader` (line 238-272) only has title + text scale slider. The API `bible::clear_broadcast()` already exists.
- **#220:** Bible search at `crates/presenter-persistence/src/repository/bible.rs:111-192` uses `LIKE '%term%'` — slow full-table scan, no word boundary matching. Need FTS5 for fast word-based search.

**Key files:**
- `crates/presenter-ui/src/pages/tablet.rs:238-272` — TabletHeader component
- `crates/presenter-ui/src/state/tablet.rs` — TabletContext with `active_broadcast`, `active_slide_id`, `show_toast`
- `crates/presenter-ui/styles/tablet.css:39-74` — tablet header CSS
- `crates/presenter-persistence/src/repository/bible.rs:111-192` — search methods
- `crates/presenter-migration/src/lib.rs` — migration registry
- `crates/presenter-server/src/router/bible.rs:145-166` — search handler

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-ui/src/pages/tablet.rs` | Add clear button to TabletHeader |
| `crates/presenter-ui/styles/tablet.css` | Add clear button CSS |
| `crates/presenter-persistence/src/repository/bible.rs` | Replace LIKE with FTS5 search |
| `crates/presenter-migration/src/lib.rs` | Register FTS5 migration |

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-migration/src/m20260412_000001_bible_fts.rs` | FTS5 virtual table + sync triggers |

---

## Task 1: Tablet Bible Clear Button (#222)

**Files:**
- Modify: `crates/presenter-ui/src/pages/tablet.rs:238-272`
- Modify: `crates/presenter-ui/styles/tablet.css`

- [ ] **Step 1: Add clear button to TabletHeader**

In `crates/presenter-ui/src/pages/tablet.rs`, replace the `TabletHeader` component (lines 238-272) with:

```rust
#[component]
fn TabletHeader() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let text_scale = ctx.text_scale;

    let on_scale_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(el) = target {
            if let Ok(val) = el.value().parse::<u32>() {
                text_scale.set(val);
                apply_scale(val);
                ctx.persist_scale();
            }
        }
    };

    let on_clear = {
        let ctx = ctx.clone();
        move |_| {
            let ctx = ctx.clone();
            leptos::task::spawn_local(async move {
                match bible::clear_broadcast().await {
                    Ok(()) => {
                        ctx.active_broadcast.set(None);
                        ctx.active_slide_id.set(None);
                        ctx.show_toast("Bible cleared", "success");
                    }
                    Err(e) => {
                        ctx.show_toast(&format!("Clear failed: {e}"), "error");
                    }
                }
            });
        }
    };

    view! {
        <header class="tablet-header">
            <h1>"Bible Tablet"</h1>
            <div class="tablet-header__actions">
                <button type="button" class="tablet-clear-btn"
                    data-role="clear-bible"
                    on:click=on_clear
                >"Clear"</button>
                <div class="tablet-scale">
                    <label for="scale-slider">"Text size"</label>
                    <input type="range" id="scale-slider" data-role="scale-slider"
                        min="50" max="200" step="10"
                        prop:value=move || text_scale.get().to_string()
                        on:input=on_scale_input
                    />
                    <span data-role="scale-value">
                        {move || format!("{}%", text_scale.get())}
                    </span>
                </div>
            </div>
        </header>
    }
}
```

- [ ] **Step 2: Add CSS for clear button**

In `crates/presenter-ui/styles/tablet.css`, after the `.tablet-header h1` block (after line 51), add:

```css
.tablet-header__actions {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.tablet-clear-btn {
  padding: 0.4rem 1rem;
  background: #ef4444;
  color: white;
  border: none;
  border-radius: 0.4rem;
  font-size: 0.9rem;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.tablet-clear-btn:active {
  background: #dc2626;
}
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/pages/tablet.rs crates/presenter-ui/styles/tablet.css
git commit -m "feat(tablet): add bible clear button (#222)

Red 'Clear' button in tablet header calls POST /bible/clear to
stop sending bible content to stage output. Resets active slide
highlight and shows confirmation toast. Closes #222."
```

---

## Task 2: FTS5 Migration (#220)

**Files:**
- Create: `crates/presenter-migration/src/m20260412_000001_bible_fts.rs`
- Modify: `crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Create the FTS5 migration file**

Create `crates/presenter-migration/src/m20260412_000001_bible_fts.rs`:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const FTS_TABLE: &str = "bible_passage_fts";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Create FTS5 virtual table (content-sync mode)
        db.execute_unprepared(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {FTS_TABLE} USING fts5(\
                book, \
                content, \
                content=bible_passages, \
                content_rowid=id\
            )"
        ))
        .await?;

        // Populate from existing data
        db.execute_unprepared(&format!(
            "INSERT OR IGNORE INTO {FTS_TABLE}(rowid, book, content) \
             SELECT id, book, content FROM bible_passages"
        ))
        .await?;

        // Keep FTS in sync on INSERT
        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_insert \
             AFTER INSERT ON bible_passages BEGIN \
                INSERT INTO {FTS_TABLE}(rowid, book, content) \
                VALUES (new.id, new.book, new.content); \
             END"
        ))
        .await?;

        // Keep FTS in sync on DELETE
        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_delete \
             AFTER DELETE ON bible_passages BEGIN \
                INSERT INTO {FTS_TABLE}({FTS_TABLE}, rowid, book, content) \
                VALUES ('delete', old.id, old.book, old.content); \
             END"
        ))
        .await?;

        // Keep FTS in sync on UPDATE
        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_update \
             AFTER UPDATE ON bible_passages BEGIN \
                INSERT INTO {FTS_TABLE}({FTS_TABLE}, rowid, book, content) \
                VALUES ('delete', old.id, old.book, old.content); \
                INSERT INTO {FTS_TABLE}(rowid, book, content) \
                VALUES (new.id, new.book, new.content); \
             END"
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_update")
            .await?;
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {FTS_TABLE}"))
            .await?;
        Ok(())
    }
}
```

- [ ] **Step 2: Register migration**

In `crates/presenter-migration/src/lib.rs`, add the module and register it:

```rust
mod m20260412_000001_bible_fts;
```

And add to the `migrations()` vec:

```rust
Box::new(m20260412_000001_bible_fts::Migration),
```

- [ ] **Step 3: Verify migration compiles**

```bash
cargo check -p presenter-migration
```

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-migration/src/m20260412_000001_bible_fts.rs crates/presenter-migration/src/lib.rs
git commit -m "feat(bible): add FTS5 full-text search migration (#220)

Creates bible_passage_fts virtual table with content-sync triggers
for INSERT, DELETE, and UPDATE. Populates from existing passages
on first run."
```

---

## Task 3: Replace LIKE Search with FTS5 (#220)

**Files:**
- Modify: `crates/presenter-persistence/src/repository/bible.rs:111-192`

- [ ] **Step 1: Add FTS5 search helper method**

In `crates/presenter-persistence/src/repository/bible.rs`, replace the `search_bible_passages` method (lines 111-144) with:

```rust
    #[instrument(skip_all)]
    pub async fn search_bible_passages(
        &self,
        translation_code: &str,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(Vec::new());
        };

        let fts_query = build_fts_query(query);
        let rows = self.fts_search(&fts_query, Some(translation_code), limit).await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(to_domain_passage(row, translation.clone())?);
        }
        Ok(results)
    }
```

- [ ] **Step 2: Replace cross-translation search**

Replace the `search_bible_passages_cross` method (lines 147-192) with:

```rust
    #[instrument(skip_all)]
    pub async fn search_bible_passages_cross(
        &self,
        translation_code: Option<&str>,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if let Some(code) = translation_code {
            return self.search_bible_passages(code, query, limit).await;
        }

        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let all_translations: std::collections::HashMap<String, bible_translation::Model> =
            bible_translation::Entity::find()
                .all(&self.db)
                .await?
                .into_iter()
                .map(|m| (m.code.clone(), m))
                .collect();

        if all_translations.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = build_fts_query(query);
        let rows = self.fts_search(&fts_query, None, limit).await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let translation_code = row.translation_code.clone();
            if let Some(translation) = all_translations.get(&translation_code) {
                results.push(to_domain_passage(row, translation.clone())?);
            }
        }
        Ok(results)
    }
```

- [ ] **Step 3: Add the fts_search and build_fts_query helpers**

Add these private methods/functions at the end of the impl block or as free functions in the same file:

```rust
    /// Execute an FTS5 search query, falling back to LIKE if FTS table doesn't exist.
    async fn fts_search(
        &self,
        fts_query: &str,
        translation_code: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        use sea_orm::{ConnectionTrait, Statement};

        let (sql, params) = if let Some(code) = translation_code {
            (
                "SELECT bp.* FROM bible_passages bp \
                 JOIN bible_passage_fts fts ON bp.id = fts.rowid \
                 WHERE bible_passage_fts MATCH ?1 \
                 AND bp.translation_code = ?2 \
                 ORDER BY fts.rank \
                 LIMIT ?3"
                    .to_string(),
                vec![
                    sea_orm::Value::from(fts_query.to_string()),
                    sea_orm::Value::from(code.to_string()),
                    sea_orm::Value::from(limit),
                ],
            )
        } else {
            (
                "SELECT bp.* FROM bible_passages bp \
                 JOIN bible_passage_fts fts ON bp.id = fts.rowid \
                 WHERE bible_passage_fts MATCH ?1 \
                 ORDER BY fts.rank \
                 LIMIT ?2"
                    .to_string(),
                vec![
                    sea_orm::Value::from(fts_query.to_string()),
                    sea_orm::Value::from(limit),
                ],
            )
        };

        let stmt = Statement::from_sql_and_values(sea_orm::DatabaseBackend::Sqlite, &sql, params);

        match bible_passage::Entity::find()
            .from_raw_sql(stmt)
            .all(&self.db)
            .await
        {
            Ok(rows) => Ok(rows),
            Err(e) => {
                tracing::warn!(error = ?e, "FTS5 search failed, falling back to LIKE");
                self.fts_fallback_like(fts_query, translation_code, limit).await
            }
        }
    }

    /// Fallback LIKE search when FTS5 is unavailable.
    async fn fts_fallback_like(
        &self,
        _fts_query: &str,
        translation_code: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        // Re-derive the original query from the FTS query (strip * suffixes)
        let original = _fts_query.replace('*', "");
        let pattern = format!("%{}%", sanitize_like_input(&original));
        let mut query = bible_passage::Entity::find()
            .filter(bible_passage::Column::Content.like(pattern));
        if let Some(code) = translation_code {
            query = query.filter(bible_passage::Column::TranslationCode.eq(code.to_string()));
        }
        Ok(query
            .order_by_asc(bible_passage::Column::Book)
            .order_by_asc(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::VerseStart)
            .limit(limit as u64)
            .all(&self.db)
            .await?)
    }
```

Add this free function (outside the impl block):

```rust
/// Build an FTS5 query from user input: split into words, append * for prefix matching.
fn build_fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .map(|w| {
            // Remove characters that are special in FTS5 syntax
            let cleaned: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if cleaned.is_empty() {
                String::new()
            } else {
                format!("{cleaned}*")
            }
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p presenter-persistence -- bible --nocapture
cargo test -p presenter-server -- bible --nocapture
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-persistence/src/repository/bible.rs
git commit -m "feat(bible): replace LIKE search with FTS5 full-text search (#220)

Search now uses SQLite FTS5 with prefix matching (word* syntax).
Multi-word queries use implicit AND. Falls back to LIKE if FTS
table is missing. Closes #220."
```

---

## Task 4: Version Bump, Push, CI, PR

- [ ] **Step 1: Bump version if needed**

Check `Cargo.toml` version vs main. Bump if equal.

- [ ] **Step 2: Local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-persistence -- bible --nocapture
npm run test:companion
```

- [ ] **Step 3: Push and monitor CI**

```bash
git push origin dev
```

Monitor until all jobs pass.

- [ ] **Step 4: Create PR**

```bash
gh pr create --title "feat: tablet clear button + FTS5 bible search (#222, #220)" --body "$(cat <<'EOF'
## Summary
- Add bible clear button to tablet UI header
- Replace LIKE-based bible search with SQLite FTS5 full-text search
- FTS5 supports word prefix matching, implicit AND, relevance ranking
- Falls back to LIKE if FTS table is missing

Closes #222, #220

## Test plan
- [ ] Tablet: trigger bible → click Clear → stage output cleared
- [ ] Search: type partial word → results appear quickly
- [ ] Search: type multiple words → results match all words
- [ ] Search: verify FTS5 fallback works on fresh DB before migration

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Clear button visible | Open tablet UI, see red "Clear" button in header |
| Clear works | Trigger bible, click Clear, stage shows no bible |
| Search speed | Search for "love" — results appear in <1s instead of 5-10s |
| Word matching | Search for "lov" — finds "love", "loved", "loving" |
| Multi-word | Search for "god love" — finds verses containing both words |
| Fallback | If FTS table missing, search still works via LIKE |
