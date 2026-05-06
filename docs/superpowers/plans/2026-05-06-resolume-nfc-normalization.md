# Resolume NFC Normalization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Normalize stored text to NFC so Resolume Arena renders Slovak diacritics correctly without the trailing `?` artifact (#305).

**Architecture:** Two-piece fix. (1) One-time idempotent sea-orm migration that scans `libraries.name`, `presentations.name`, `slides.worship_main`, `slides.worship_translate`, `slides.worship_stage` and rewrites NFD rows to NFC. (2) `presenter-importer` calls `.nfc().collect()` at the boundaries where ProPresenter text enters the domain (clean_text, presentation name, library name). No send-path change, no schema change, auto-runs on next deploy via existing `Repository::connect()` startup migrations.

**Tech Stack:** Rust, sea-orm, sea-orm-migration, `unicode-normalization` crate (already at workspace `0.1`).

**Spec:** `docs/superpowers/specs/2026-05-06-resolume-nfc-normalization-design.md` (commit 30c3f7e)

---

## Context

Issue #305: Slovak text like `"401 Po Tebe Pane žíznim"` renders in Resolume with `?` after each diacritic. Diagnostic capture from prod confirmed the data is stored in NFD form — `ž` = `z` + combining caron (U+030C), `í` = `i` + combining acute (U+0301). Browsers' text shapers compose combining marks; Resolume's text effect renderer does not. Fix the storage; consumers send what they read.

**Key existing code:**
- `crates/presenter-migration/src/lib.rs` — registers all migrations in order. Pattern: `m<timestamp>_<seq>_<name>.rs` with `DeriveMigrationName` + `MigrationTrait`.
- `crates/presenter-migration/src/m20260408_000001_add_preach_limit.rs` — reference for existing migration style (raw SQL via `sea_orm::Statement::from_string` with `Sqlite` backend).
- `crates/presenter-importer/src/rtf.rs:234` — `clean_text(text: String) -> String` returns `strip_header_lines(cleaned)` at line 277. NFC must be appended AFTER `strip_header_lines`.
- `crates/presenter-importer/src/lib.rs:285` — `Ok(Presentation::new(raw.name.clone(), slides)?)`. `raw.name` is the ProPresenter presentation name; comes from the `.pro` protobuf payload as-is.
- `crates/presenter-importer/src/lib.rs:207-216` — library name derived from `dir.file_name()`. NFD-form macOS directory names land here verbatim.
- `crates/presenter-persistence/src/entities.rs` — confirms columns: `libraries.name`, `presentations.name`, `slides.worship_main`, `slides.worship_translate`, `slides.worship_stage`.
- `crates/presenter-core/src/search.rs:42` — `normalise_for_search` does `nfd().filter(!is_combining_mark)`. Already form-agnostic; `*_search` columns NOT touched by the migration.
- `unicode-normalization = "0.1"` declared in root `Cargo.toml`. Need to add `unicode-normalization.workspace = true` to `crates/presenter-migration/Cargo.toml` and `crates/presenter-importer/Cargo.toml`.

---

## File Structure

### Created files
| File | Responsibility |
|------|----------------|
| `crates/presenter-importer/src/nfc.rs` | Tiny `to_nfc(s: &str) -> String` helper used at three boundaries |
| `crates/presenter-migration/src/m20260506_000001_normalize_text_to_nfc.rs` | One-time NFC rewrite of existing rows |

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | bump `[workspace.package].version` to `0.4.72` |
| `crates/presenter-importer/Cargo.toml` | add `unicode-normalization.workspace = true` |
| `crates/presenter-migration/Cargo.toml` | add `unicode-normalization.workspace = true` |
| `crates/presenter-importer/src/lib.rs` | add `mod nfc;`, wrap presentation name (line 285) and library name (line 216) with `nfc::to_nfc`. Add 2 unit tests. |
| `crates/presenter-importer/src/rtf.rs` | append `.nfc().collect::<String>()` to the final return at line 277. Add 1 unit test. |
| `crates/presenter-migration/src/lib.rs` | register the new migration |

---

## Task 1: Version bump 0.4.71 → 0.4.72

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`)

- [ ] **Step 1: Edit version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, change:

```toml
version = "0.4.71"
```

to:

```toml
version = "0.4.72"
```

- [ ] **Step 2: Refresh Cargo.lock**

Run: `cargo update -w`

Expected output: lines for each workspace member updating from `v0.4.71` → `v0.4.72`. No errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.72"
```

---

## Task 2: Wire up `unicode-normalization` dependency

**Files:**
- Modify: `crates/presenter-importer/Cargo.toml`
- Modify: `crates/presenter-migration/Cargo.toml`

- [ ] **Step 1: Add dep to presenter-importer**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/Cargo.toml`, find the `[dependencies]` table and add the line:

```toml
unicode-normalization.workspace = true
```

(Keep alphabetical order if the file uses it; otherwise append.)

- [ ] **Step 2: Add dep to presenter-migration**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-migration/Cargo.toml`, find the `[dependencies]` table and add:

```toml
unicode-normalization.workspace = true
```

- [ ] **Step 3: Verify the workspace builds with the new deps unused**

Run: `cargo build --workspace`

Expected: green build (the deps are declared but not yet used; that is fine — Rust does not warn about unused deps in `Cargo.toml`).

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-importer/Cargo.toml crates/presenter-migration/Cargo.toml
git commit -m "deps: wire unicode-normalization into importer + migration crates (#305)"
```

---

## Task 3: Importer NFC at the boundaries (TDD)

**Files:**
- Create: `crates/presenter-importer/src/nfc.rs`
- Modify: `crates/presenter-importer/src/lib.rs:21-23` (mod declarations), `:216` (library name), `:285` (presentation name)
- Modify: `crates/presenter-importer/src/rtf.rs:277` (clean_text return)

- [ ] **Step 1: Write the failing helper test**

Create `crates/presenter-importer/src/nfc.rs` with:

```rust
//! NFC normalization at ProPresenter import boundaries.
//!
//! ProPresenter on macOS stores text in NFD form (decomposed Unicode).
//! presenter stores it in NFC so consumers like Resolume's text effect
//! render combining marks correctly.

use unicode_normalization::UnicodeNormalization;

/// Return `s` in Unicode Normalization Form C (canonical composed form).
///
/// Idempotent: calling on already-NFC input yields a string equal to the input.
pub(crate) fn to_nfc(s: &str) -> String {
    s.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::to_nfc;

    #[test]
    fn nfd_input_is_recomposed() {
        // 'ž' as NFD: 'z' + combining caron (U+030C)
        let nfd = "z\u{30c}";
        assert_eq!(to_nfc(nfd), "\u{17e}"); // single 'ž' codepoint
    }

    #[test]
    fn already_nfc_is_unchanged() {
        let nfc = "\u{17e}\u{ed}znim"; // 'žíznim' precomposed
        assert_eq!(to_nfc(nfc), nfc);
    }

    #[test]
    fn ascii_passthrough() {
        assert_eq!(to_nfc("hello"), "hello");
        assert_eq!(to_nfc(""), "");
    }

    #[test]
    fn slovak_phrase_recomposed() {
        // "Po Tebe Pane žíznim" with NFD ž and í
        let nfd = "Po Tebe Pane z\u{30c}i\u{301}znim";
        let nfc = "Po Tebe Pane \u{17e}\u{ed}znim";
        assert_eq!(to_nfc(nfd), nfc);
    }
}
```

- [ ] **Step 2: Wire `mod nfc` into the importer crate**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/src/lib.rs`, find the existing module declarations near line 21-23:

```rust
pub mod bible;
mod bible_digest;
mod rtf;
```

and add `mod nfc;` after `mod rtf;`:

```rust
pub mod bible;
mod bible_digest;
mod nfc;
mod rtf;
```

- [ ] **Step 3: Run helper tests — confirm they pass**

Run: `cargo test -p presenter-importer --lib nfc::tests -- --nocapture`

Expected: 4 passing tests (`nfd_input_is_recomposed`, `already_nfc_is_unchanged`, `ascii_passthrough`, `slovak_phrase_recomposed`).

- [ ] **Step 4: Commit the helper**

```bash
git add crates/presenter-importer/src/nfc.rs crates/presenter-importer/src/lib.rs
git commit -m "feat(importer): add to_nfc helper for boundary normalization (#305)"
```

- [ ] **Step 5: Write the failing rtf.rs test**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/src/rtf.rs`, find the existing `#[cfg(test)] mod tests` block (around line 365). Add this test inside that block:

```rust
    #[test]
    fn clean_text_normalizes_nfd_to_nfc() {
        // ProPresenter Mac source produces NFD: 'z' + U+030C, 'i' + U+0301.
        let input = String::from("Po Tebe Pane z\u{30c}i\u{301}znim");
        let cleaned = clean_text(input);
        // Expect single-codepoint 'ž' and 'í' in the output.
        assert_eq!(cleaned, "Po Tebe Pane \u{17e}\u{ed}znim");
    }
```

- [ ] **Step 6: Run the rtf test — confirm it fails**

Run: `cargo test -p presenter-importer --lib rtf::tests::clean_text_normalizes_nfd_to_nfc -- --nocapture`

Expected: FAIL — current `clean_text` returns NFD as-is. Output diff shows `'z' + caron` vs precomposed `'ž'`.

- [ ] **Step 7: Implement NFC normalization in clean_text**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/src/rtf.rs`, change the final line of `clean_text` (around line 277). Find:

```rust
    strip_header_lines(cleaned)
}
```

and change to:

```rust
    use unicode_normalization::UnicodeNormalization;
    strip_header_lines(cleaned).nfc().collect()
}
```

(The `use` statement is local to the function so we don't change the file's top imports.)

- [ ] **Step 8: Run all rtf tests — confirm everything passes**

Run: `cargo test -p presenter-importer --lib rtf -- --nocapture`

Expected: every existing test still passes (NFC is idempotent on NFC inputs, and the existing fixtures are ASCII-only or already NFC) plus the new test.

- [ ] **Step 9: Commit rtf change**

```bash
git add crates/presenter-importer/src/rtf.rs
git commit -m "fix(importer): NFC-normalize cleaned slide text (#305)"
```

- [ ] **Step 10: Write the failing presentation-name test**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/src/lib.rs`, find the existing `#[cfg(test)] mod tests` block. Add this test inside that block:

```rust
    #[test]
    fn presentation_from_proto_normalizes_name_to_nfc() {
        let mut raw = proto::Presentation::default();
        raw.name = String::from("Po Tebe Pane z\u{30c}i\u{301}znim");
        // Add a single placeholder slide so presentation_from_proto succeeds.
        let cue = proto::Cue {
            uuid: Some(proto::Uuid {
                string: "cue-1".into(),
                ..Default::default()
            }),
            actions: vec![],
            ..Default::default()
        };
        let cue_id = proto::Identifier {
            string: "cue-1".into(),
            ..Default::default()
        };
        raw.cues = vec![cue];
        raw.cue_identifiers = vec![cue_id];

        // Build a no-op slide so the function does not error on empty actions.
        let slide_action = proto::Action {
            slide: Some(proto::action::Slide::default()),
            ..Default::default()
        };
        raw.cues[0].actions = vec![slide_action];

        let presentation = presentation_from_proto(&raw).expect("import succeeds");
        assert_eq!(presentation.name, "Po Tebe Pane \u{17e}\u{ed}znim");
    }
```

(If the existing tests already build a fixture proto, prefer reusing that helper. Inspect the `tests` module before writing this test and adjust if a `make_proto_with_name` helper exists.)

- [ ] **Step 11: Run the presentation-name test — confirm it fails**

Run: `cargo test -p presenter-importer --lib tests::presentation_from_proto_normalizes_name_to_nfc -- --nocapture`

Expected: FAIL — `presentation.name` still contains the NFD bytes.

- [ ] **Step 12: Implement NFC for presentation name**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-importer/src/lib.rs`, line 285. Find:

```rust
    Ok(Presentation::new(raw.name.clone(), slides)?)
```

and change to:

```rust
    Ok(Presentation::new(nfc::to_nfc(&raw.name), slides)?)
```

- [ ] **Step 13: Run the presentation-name test — confirm it passes**

Run: `cargo test -p presenter-importer --lib tests::presentation_from_proto_normalizes_name_to_nfc -- --nocapture`

Expected: PASS.

- [ ] **Step 14: Extract `library_name_from_dir` helper**

Refactor `load_library_from_directory` (lines 203-238) to extract the name derivation into a small testable helper. This keeps the test simple (a `Path::new`, no fixture file). In `crates/presenter-importer/src/lib.rs`, add this helper just before `load_library_from_directory`:

```rust
fn library_name_from_dir(dir: &Path) -> Result<String> {
    let raw = dir
        .file_name()
        .and_then(|os| os.to_str())
        .ok_or_else(|| {
            anyhow!(
                "library directory must have a valid UTF-8 name: {}",
                dir.display()
            )
        })?;
    Ok(nfc::to_nfc(raw))
}
```

Then in `load_library_from_directory`, replace lines 207-216 (the existing `let name = dir.file_name()...to_string();` block) with:

```rust
    let name = library_name_from_dir(dir)?;
```

- [ ] **Step 15: Add a unit test for `library_name_from_dir`**

In the same `lib.rs` test module, add:

```rust
    #[test]
    fn library_name_from_dir_normalizes_nfd_to_nfc() {
        use std::path::Path;
        // NFD-form 'ž' in directory name: 'TYz\u{30c}MY'
        let path = Path::new("TYz\u{30c}MY");
        let name = super::library_name_from_dir(path).expect("name");
        assert_eq!(name, "TY\u{17e}MY");
    }
```

- [ ] **Step 16: Run the library-name test — confirm it passes**

Run: `cargo test -p presenter-importer --lib tests::library_name_from_dir_normalizes_nfd_to_nfc -- --nocapture`

Expected: PASS (the helper from Step 14 already calls `nfc::to_nfc`).

- [ ] **Step 17: Run all importer tests — confirm everything passes**

Run: `cargo test -p presenter-importer -- --nocapture`

Expected: every existing test passes; new tests pass; total count up by at least 6 (4 nfc tests + 1 rtf + 1 presentation + 1 library, possibly more if you split helper tests).

- [ ] **Step 18: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`

Expected: no warnings.

- [ ] **Step 19: Commit importer boundary changes**

```bash
git add crates/presenter-importer/src/lib.rs
git commit -m "fix(importer): NFC-normalize presentation and library names (#305)"
```

---

## Task 4: One-time NFC migration for existing rows (TDD)

**Files:**
- Create: `crates/presenter-migration/src/m20260506_000001_normalize_text_to_nfc.rs`
- Modify: `crates/presenter-migration/src/lib.rs` (register migration)

- [ ] **Step 1: Write the failing migration test**

Create `crates/presenter-migration/src/m20260506_000001_normalize_text_to_nfc.rs` with the test module first (TDD; implementation follows). Start the file with:

```rust
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;
use unicode_normalization::{is_nfc, UnicodeNormalization};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (table, columns) in TARGETS {
            normalize_table(db, table, columns).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // NFC is canonical; reverting to NFD has no value. No-op.
        Ok(())
    }
}

const TARGETS: &[(&str, &[&str])] = &[
    ("libraries", &["name"]),
    ("presentations", &["name"]),
    (
        "slides",
        &[
            "worship_main",
            "worship_translate",
            "worship_stage",
        ],
    ),
];

async fn normalize_table<C: ConnectionTrait>(
    db: &C,
    table: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    let projection = std::iter::once("id".to_string())
        .chain(columns.iter().map(|c| (*c).to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT {projection} FROM {table}");
    let rows = db
        .query_all(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            select_sql,
        ))
        .await?;

    for row in rows {
        let id: String = row.try_get("", "id")?;
        let mut updates: Vec<(String, String)> = Vec::new();
        for column in columns {
            let value: String = row.try_get("", column)?;
            if !is_nfc(&value) {
                let nfc: String = value.nfc().collect();
                updates.push(((*column).to_string(), nfc));
            }
        }
        if updates.is_empty() {
            continue;
        }
        let set_clause = updates
            .iter()
            .map(|(col, _)| format!("{col} = ?"))
            .collect::<Vec<_>>()
            .join(", ");
        let update_sql = format!("UPDATE {table} SET {set_clause} WHERE id = ?");
        let mut values: Vec<sea_orm::Value> =
            updates.into_iter().map(|(_, v)| v.into()).collect();
        values.push(id.into());
        db.execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            update_sql,
            values,
        ))
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend, Statement};

    async fn setup_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect");
        // Minimal schema for the targeted tables. Use TEXT for ids; ignore FKs.
        for ddl in &[
            "CREATE TABLE libraries (id TEXT PRIMARY KEY, name TEXT NOT NULL, search_name TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE presentations (id TEXT PRIMARY KEY, library_id TEXT NOT NULL, name TEXT NOT NULL, search_name TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE slides (id TEXT PRIMARY KEY, presentation_id TEXT NOT NULL, worship_main TEXT NOT NULL DEFAULT '', worship_translate TEXT NOT NULL DEFAULT '', worship_stage TEXT NOT NULL DEFAULT '')",
        ] {
            db.execute(Statement::from_string(
                DbBackend::Sqlite,
                (*ddl).to_string(),
            ))
            .await
            .expect("ddl");
        }
        db
    }

    async fn insert_row(db: &sea_orm::DatabaseConnection, sql: &str) {
        db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("insert");
    }

    async fn fetch_one_string(db: &sea_orm::DatabaseConnection, sql: &str) -> String {
        let row = db
            .query_one(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("query")
            .expect("row");
        row.try_get_by_index::<String>(0).expect("string col")
    }

    #[tokio::test]
    async fn migration_normalizes_nfd_rows_and_is_idempotent() {
        let db = setup_db().await;

        // Seed: one NFD library, one NFC library, one NFD slide field.
        insert_row(
            &db,
            "INSERT INTO libraries (id, name) VALUES ('lib-nfd', 'TYz\u{30c}MY')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO libraries (id, name) VALUES ('lib-nfc', 'CLEAN')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO presentations (id, library_id, name) VALUES \
             ('p-nfd', 'lib-nfd', 'Po Tebe Pane z\u{30c}i\u{301}znim')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO slides (id, presentation_id, worship_main, worship_translate, worship_stage) VALUES \
             ('s-nfd', 'p-nfd', 'z\u{30c}', 'CLEAN', 'i\u{301}')",
        )
        .await;

        // Run the up function via a minimal SchemaManager substitute:
        // call normalize_table directly for each target.
        for (table, columns) in TARGETS {
            super::normalize_table(&db, table, columns).await.expect("up");
        }

        // After: every row's text fields are NFC.
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfd'").await,
            "TY\u{17e}MY"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfc'").await,
            "CLEAN"
        );
        assert_eq!(
            fetch_one_string(
                &db,
                "SELECT name FROM presentations WHERE id='p-nfd'"
            )
            .await,
            "Po Tebe Pane \u{17e}\u{ed}znim"
        );
        assert_eq!(
            fetch_one_string(
                &db,
                "SELECT worship_main FROM slides WHERE id='s-nfd'"
            )
            .await,
            "\u{17e}"
        );
        assert_eq!(
            fetch_one_string(
                &db,
                "SELECT worship_stage FROM slides WHERE id='s-nfd'"
            )
            .await,
            "\u{ed}"
        );

        // Idempotency: running again is a no-op (row contents identical).
        for (table, columns) in TARGETS {
            super::normalize_table(&db, table, columns).await.expect("rerun");
        }
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfd'").await,
            "TY\u{17e}MY"
        );
    }
}
```

- [ ] **Step 2: Register the migration**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-migration/src/lib.rs`, find the migration list. Add the new module declaration in the right place (after `m20260420_000001_create_group_colors`):

```rust
mod m20260420_000001_create_group_colors;
mod m20260506_000001_normalize_text_to_nfc;
```

And inside `Migrator::migrations()`, append the new migration to the `vec![...]` (preserving existing order):

```rust
            Box::new(m20260420_000001_create_group_colors::Migration),
            Box::new(m20260506_000001_normalize_text_to_nfc::Migration),
        ]
    }
}
```

- [ ] **Step 3: Run the migration test — confirm it passes**

Run: `cargo test -p presenter-migration --lib m20260506_000001_normalize_text_to_nfc::tests::migration_normalizes_nfd_rows_and_is_idempotent -- --nocapture`

Expected: PASS. The implementation and test live in the same new file (Step 1), so they ship together. If the test fails, inspect the diff against expected NFC values and fix the implementation before moving on.

- [ ] **Step 4: Run all migration tests**

Run: `cargo test -p presenter-migration -- --nocapture`

Expected: all pass.

- [ ] **Step 5: Run workspace build**

Run: `cargo build --workspace`

Expected: green.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`

Expected: no warnings.

- [ ] **Step 7: Commit the migration**

```bash
git add crates/presenter-migration/src/m20260506_000001_normalize_text_to_nfc.rs crates/presenter-migration/src/lib.rs
git commit -m "feat(migration): NFC-normalize stored text on existing rows (#305)"
```

---

## Task 5: Push, monitor CI, manual verify, open PR (controller-handled)

This task is performed by the orchestrator, not a subagent. The orchestrator owns push timing, CI monitoring, and the manual verification on dev.

- [ ] **Step 1: Final local checks**

Run, in this order:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace -- --nocapture
```
From `crates/presenter-ui`: `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all`

All must pass.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI**

```bash
gh run list --branch dev --limit 1 --json databaseId
# Capture the run id, then in a single background bash:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs
```

Wait for ALL jobs (including Mutation Testing) to be `success`. If any fail, investigate via `gh run view <run-id> --log-failed`, fix in ONE commit, repeat.

- [ ] **Step 4: Verify on dev**

After Deploy to Dev is green:

```bash
curl -s 'http://10.77.8.134:8080/healthz'
# Expect: {"channel":"dev","status":"ok","version":"0.4.72"}

curl -s 'http://10.77.8.134:8080/search?q=znim' | python3 -c "
import sys, json, unicodedata
d = json.load(sys.stdin)
for r in d[:5]:
    n = r['presentationName']
    print(n, '— NFC:', unicodedata.normalize('NFC', n) == n)
"
# Expect every line to print 'NFC: True'
```

If any row prints `NFC: False`, the migration didn't normalize that row — investigate before proceeding.

Optional (real Resolume on dev's LAN, if available): trigger a slide push that contains NFD-source text and visually confirm Resolume renders without trailing `?`. Otherwise the codepoint check above is the canonical pre-merge proof.

- [ ] **Step 5: Open PR**

```bash
gh pr create --base main --head dev --title "fix(importer+migration): NFC-normalize text for Resolume diacritic rendering (#305)" --body "$(cat <<'EOF'
## Summary

Closes #305: Resolume Arena was rendering Slovak diacritics with a trailing \`?\` because text was stored in NFD form (decomposed Unicode). Resolume's text effect renderer doesn't apply combining marks; browsers do, which is why the operator UI and \`/stage\` looked correct.

## What changed

- **One-time DB migration** \`m20260506_000001_normalize_text_to_nfc\`. Idempotent. Scans \`libraries.name\`, \`presentations.name\`, and \`slides.worship_{main,translate,stage}\`; rewrites any non-NFC row to NFC. Runs once on startup via \`Repository::connect()\`. Pre-deploy DB backup is automatic.
- **Importer NFC at boundaries**. \`presenter-importer::nfc::to_nfc\` helper. Applied in \`clean_text\` (slide RTF), at presentation-name conversion, and at library-name derivation. Future Mac imports land NFC.
- No send-path change; no schema change; \`*_search\` columns untouched (\`fold_query\` is form-agnostic).

## Verification

- All unit tests pass (4 nfc helper, 1 rtf, 1 presentation, 1 library, 1 migration with idempotency).
- Dev \`/search?q=znim\` returns rows where every \`presentationName\` is NFC.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Wait for the PR to reach \`mergeStateStatus: CLEAN\` and \`mergeable: MERGEABLE\` before reporting. Do NOT merge without explicit "merge it" from the user.

---

## Verification summary

| Check | How to verify |
|-------|---------------|
| Importer NFC applied | `cargo test -p presenter-importer` — 4 nfc + 1 rtf + 1 presentation + 1 library tests pass |
| Migration NFC applied | `cargo test -p presenter-migration` — `migration_normalizes_nfd_rows_and_is_idempotent` passes |
| Migration idempotent | Same test runs migration twice; final state matches first-run state |
| Workspace builds | `cargo build --workspace` green |
| WASM clippy | `cargo clippy --target wasm32-unknown-unknown -p presenter-ui` green |
| Dev migration ran | After deploy, `/search?q=znim` rows are all NFC |
| No send-path regressions | Existing `presenter-server` and `presenter-server::resolume::tests` pass unchanged |
| Version bump | `Cargo.toml` workspace version is `0.4.72` |
