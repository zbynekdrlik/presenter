# Group Color System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hash-based 8-color group palette with persistent database-backed colors using legacy ProPresenter mappings, auto-generation for unknown groups, and WCAG luminance-based text contrast.

**Architecture:** A new `group_colors` table stores name→color mappings (seeded with ~70 legacy values). The server resolves colors and sends them alongside group names in `StageDisplaySlide`. The WASM frontend renders solid-color pill backgrounds with black/white text computed via luminance.

**Tech Stack:** Rust (SeaORM, sea-orm-migration, Axum), Leptos WASM, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-20-group-color-system-design.md`

---

## Context

- `StageDisplaySlide` (in `presenter-core`) currently has `group: Option<String>` — just the name, no color.
- Color is computed client-side in `crates/presenter-ui/src/utils/color.rs` using FNV-1a hash → 8 colors.
- Both `worship_snv.rs` and `worship_pp.rs` call `group_color(&name)` and `hex_to_rgba(color, 0.25)` to produce inline styles.
- The server builds `StageDisplaySlide` in `crates/presenter-server/src/state/stage.rs` (`SlideCtx::to_stage_display()`).
- Migrations live in `crates/presenter-migration/src/` and are registered in `lib.rs`.
- Entity models are in `crates/presenter-persistence/src/entities.rs`.
- Repository methods are in `crates/presenter-persistence/src/repository/mod.rs`.

---

## File Structure

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-migration/src/m20260420_000001_create_group_colors.rs` | Migration: create table + seed legacy data |
| `crates/presenter-persistence/src/repository/group_color.rs` | Repository methods for group_colors table |
| `tests/e2e/stage-group-colors.spec.ts` | E2E test for colored group pills |

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-migration/src/lib.rs` | Register new migration |
| `crates/presenter-persistence/src/entities.rs` | Add `group_color` entity module |
| `crates/presenter-persistence/src/repository/mod.rs` | Add `mod group_color;` and re-export |
| `crates/presenter-core/src/stage_display.rs` | Add `group_color: Option<String>` to `StageDisplaySlide` |
| `crates/presenter-server/src/state/stage.rs` | Resolve group colors when building slides |
| `crates/presenter-server/src/state/mod.rs` | Add group color cache to AppState |
| `crates/presenter-ui/src/utils/color.rs` | Replace `group_color()` with `text_color_for_bg()` luminance function |
| `crates/presenter-ui/src/components/stage/worship_snv.rs` | Use `group_color` from snapshot, solid bg, luminance text |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Same changes as worship_snv |
| `crates/presenter-ui/styles/stage.css` | Update `.stage__group-pill` to solid background style |

---

## Task 1: Database Migration — Create group_colors Table with Legacy Seed

**Files:**
- Create: `crates/presenter-migration/src/m20260420_000001_create_group_colors.rs`
- Modify: `crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Create migration file**

```rust
// crates/presenter-migration/src/m20260420_000001_create_group_colors.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const LEGACY_COLORS: &[(&str, &str)] = &[
    ("Vsetci", "#E08A3C"),
    ("Zeny", "#C73E9E"),
    ("Muzi", "#2E2E8F"),
    ("Zeny/Muzi", "#4A6CE0"),
    ("Peta", "#8E44C4"),
    ("Stevo", "#D62828"),
    ("Miro", "#3CB371"),
    ("Zuzka", "#E8631C"),
    ("Patrika", "#D4621E"),
    ("Tina", "#E89B7A"),
    ("Miro, Peta, Zuzka", "#C8304A"),
    ("Stevo, Peta", "#8B1A1A"),
    ("Stevo, Peta, Zuzka", "#D81BC0"),
    ("Stevo, Zuzka", "#2B9D9D"),
    ("Peta, Miro", "#6A2C9E"),
    ("Miro, Stevo, Zuzka", "#5A7A7A"),
    ("Stevo, zeny", "#E8A020"),
    ("Tina, Zuzka", "#B83CA4"),
    ("Miro, Zuzka", "#8A9A3A"),
    ("Peta // Vsetci", "#1A1A1A"),
    ("Miro, Tina", "#A05A2C"),
    ("Vsetci // Zuzka", "#A61E1E"),
    ("Muzi, Peta", "#2E8B57"),
    ("Muzi, Zuzka", "#C83030"),
    ("Vsetci okrem zuzky", "#D47A2C"),
    ("Stevo, Miro, Tina", "#1E2B8F"),
    ("Vsetci okrem Peti", "#9E8AC4"),
    ("Stevo, Tina", "#4A7AD4"),
    ("Miro, Tina, Zuzka", "#8B1E3F"),
    ("Miro // vsetci", "#3CA4E0"),
    ("Muzi // Zeny", "#B8A82C"),
    ("Vsetci // Peta", "#2B5AA6"),
    ("Peta, Zuzka", "#3A5AD4"),
    ("Patrika, Miro", "#8B1E3F"),
    ("Vsetci // Patrika, Miro", "#A89A2C"),
    ("Peta, Miro // vsetci", "#1A1A1A"),
    ("Patrika, Miro, Zuzka", "#C41E5A"),
    ("Miro, Tina, Patrika", "#7A2CA6"),
    ("Patrika, Stevo", "#2B5A6A"),
    ("Pomaly", "#1A1A1A"),
    ("Rychlejsie", "#3A3A3A"),
    ("Rychlo", "#9A9A9A"),
    ("Bridge 2", "#A0521E"),
    ("Bridge 1", "#E8831C"),
    ("Chorus 4", "#8B1E2C"),
    ("Chorus 3", "#9A9A9A"),
    ("Inter Chorus", "#1E5A8B"),
    ("Chorus 2", "#C43CB4"),
    ("PreChorus", "#5A1E9E"),
    ("Intro", "#F0E020"),
    ("Chorus 1", "#E02020"),
    ("Verse 1", "#2040E0"),
    ("Verse 2", "#6FE020"),
    ("Verse 3", "#D41E6A"),
    ("Verse 4", "#7A2CA6"),
    ("1. sloha", "#2CA64A"),
    ("2. sloha", "#2B6AA6"),
    ("Peta, Stevo, Zuzka // vsetci", "#1A1A1A"),
    ("Postchorus", "#3CB371"),
    ("Miro, Zuzka // vsetci", "#8B1E3F"),
    ("Patrika, Tina, Stevo", "#D62828"),
    ("Vsetci // Stevo, Peta", "#2B7AC4"),
    ("Patrika, muzi", "#D41EA6"),
    ("Tina, Patrika", "#3CB371"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE TABLE IF NOT EXISTS group_colors (
                name TEXT PRIMARY KEY NOT NULL,
                color TEXT NOT NULL
            )",
        ))
        .await?;

        for (name, color) in LEGACY_COLORS {
            db.execute(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT OR IGNORE INTO group_colors (name, color) VALUES (?, ?)",
                [(*name).into(), (*color).into()],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "DROP TABLE IF EXISTS group_colors",
        ))
        .await?;
        Ok(())
    }
}
```

- [ ] **Step 2: Register migration in lib.rs**

In `crates/presenter-migration/src/lib.rs`, add:

```rust
mod m20260420_000001_create_group_colors;
```

And append to the migrations vec:

```rust
Box::new(m20260420_000001_create_group_colors::Migration),
```

- [ ] **Step 3: Verify migration compiles**

Run: `cargo build -p presenter-migration`
Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-migration/src/m20260420_000001_create_group_colors.rs crates/presenter-migration/src/lib.rs
git commit -m "feat(migration): create group_colors table with legacy seed data

Adds group_colors table (name TEXT PK, color TEXT) seeded with ~63
legacy ProPresenter group color mappings that singers already know."
```

---

## Task 2: Entity Model and Repository Methods

**Files:**
- Create: `crates/presenter-persistence/src/repository/group_color.rs`
- Modify: `crates/presenter-persistence/src/entities.rs`
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Add entity module to entities.rs**

Append to `crates/presenter-persistence/src/entities.rs`:

```rust
pub mod group_color {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "group_colors")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub color: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
```

- [ ] **Step 2: Create repository/group_color.rs**

```rust
// crates/presenter-persistence/src/repository/group_color.rs
use crate::entities::group_color;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use std::collections::HashMap;

use super::Repository;

/// Curated palette for auto-generated colors (avoids legacy color collisions).
const AUTO_PALETTE: [&str; 20] = [
    "#FF6B6B", "#4ECDC4", "#45B7D1", "#96CEB4", "#FFEAA7",
    "#DDA0DD", "#98D8C8", "#F7DC6F", "#BB8FCE", "#85C1E9",
    "#F8C471", "#82E0AA", "#F1948A", "#AED6F1", "#D7BDE2",
    "#A3E4D7", "#FAD7A0", "#A9CCE3", "#D5F5E3", "#FADBD8",
];

fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

fn generate_color(name: &str) -> &'static str {
    let hash = fnv1a(name);
    AUTO_PALETTE[(hash as usize) % AUTO_PALETTE.len()]
}

impl Repository {
    /// Load all group colors into a HashMap for caching.
    pub async fn load_all_group_colors(&self) -> anyhow::Result<HashMap<String, String>> {
        let rows = group_color::Entity::find()
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(|r| (r.name, r.color)).collect())
    }

    /// Get color for a group name. If not found, generate and insert.
    pub async fn resolve_group_color(&self, name: &str) -> anyhow::Result<String> {
        let existing = group_color::Entity::find()
            .filter(group_color::Column::Name.eq(name))
            .one(&self.db)
            .await?;

        if let Some(row) = existing {
            return Ok(row.color);
        }

        let color = generate_color(name).to_string();
        let backend = self.db.get_database_backend();
        self.db
            .execute(Statement::from_sql_and_values(
                backend,
                "INSERT OR IGNORE INTO group_colors (name, color) VALUES (?, ?)",
                [name.into(), color.clone().into()],
            ))
            .await?;

        Ok(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_color_is_deterministic() {
        assert_eq!(generate_color("Verse 5"), generate_color("Verse 5"));
    }

    #[test]
    fn generate_color_returns_palette_entry() {
        let color = generate_color("Some New Group");
        assert!(AUTO_PALETTE.contains(&color));
    }

    #[test]
    fn generate_color_different_names_can_differ() {
        // Not guaranteed for all pairs, but these two should differ
        let a = generate_color("Alpha");
        let b = generate_color("Beta");
        // At minimum they're valid palette entries
        assert!(AUTO_PALETTE.contains(&a));
        assert!(AUTO_PALETTE.contains(&b));
    }

    #[tokio::test]
    async fn load_all_group_colors_includes_seeded_data() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let colors = repo.load_all_group_colors().await.unwrap();
        assert_eq!(colors.get("Vsetci"), Some(&"#E08A3C".to_string()));
        assert_eq!(colors.get("Muzi"), Some(&"#2E2E8F".to_string()));
        assert!(colors.len() >= 63);
    }

    #[tokio::test]
    async fn resolve_group_color_returns_legacy_for_known() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let color = repo.resolve_group_color("Stevo").await.unwrap();
        assert_eq!(color, "#D62828");
    }

    #[tokio::test]
    async fn resolve_group_color_generates_for_unknown() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let color = repo.resolve_group_color("Brand New Group").await.unwrap();
        assert!(AUTO_PALETTE.contains(&color.as_str()));
        // Second call returns same color (persisted)
        let color2 = repo.resolve_group_color("Brand New Group").await.unwrap();
        assert_eq!(color, color2);
    }
}
```

- [ ] **Step 3: Register module in repository/mod.rs**

Add `mod group_color;` after the existing module declarations at the top of `crates/presenter-persistence/src/repository/mod.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p presenter-persistence -- group_color --nocapture`
Expected: All 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-persistence/src/entities.rs crates/presenter-persistence/src/repository/group_color.rs crates/presenter-persistence/src/repository/mod.rs
git commit -m "feat(persistence): add group_color entity and repository methods

Provides load_all_group_colors() for cache init and
resolve_group_color() for on-demand lookup with auto-generation
via FNV-1a hash to a 20-color curated palette."
```

---

## Task 3: Add group_color to StageDisplaySlide (Core Crate)

**Files:**
- Modify: `crates/presenter-core/src/stage_display.rs:74-79, 119-129, 131-143`

- [ ] **Step 1: Add field to StageDisplaySlide**

In `crates/presenter-core/src/stage_display.rs`, add `group_color` after `group`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplaySlide {
    pub main: String,
    pub translation: String,
    pub stage: String,
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_color: Option<String>,
}
```

- [ ] **Step 2: Update From<&DomainSlide> impl**

```rust
impl From<&DomainSlide> for StageDisplaySlide {
    fn from(slide: &DomainSlide) -> Self {
        let content = &slide.content;
        Self {
            main: content.main.value().to_string(),
            translation: content.translation.value().to_string(),
            stage: content.stage.value().to_string(),
            group: content.group.as_ref().map(|g| g.name().to_string()),
            group_color: None, // Resolved by server later
        }
    }
}
```

- [ ] **Step 3: Update From<&ResolvedSlide> impl**

```rust
impl From<&ResolvedSlide> for StageDisplaySlide {
    fn from(slide: &ResolvedSlide) -> Self {
        Self {
            main: slide.main.value().to_string(),
            translation: slide.translation.value().to_string(),
            stage: slide.stage.value().to_string(),
            group: slide
                .effective_group
                .as_ref()
                .map(|group| group.name().to_string()),
            group_color: None, // Resolved by server later
        }
    }
}
```

- [ ] **Step 4: Fix all other StageDisplaySlide constructions**

Search for `StageDisplaySlide {` in the codebase and add `group_color: None` to each. The key location is `crates/presenter-server/src/state/stage.rs` line 72-77 (`SlideCtx::to_stage_display()`):

```rust
    fn to_stage_display(&self) -> StageDisplaySlide {
        StageDisplaySlide {
            main: self.slide.content.main.value().to_string(),
            translation: self.slide.content.translation.value().to_string(),
            stage: self.slide.content.stage.value().to_string(),
            group: self.effective_group.clone(),
            group_color: None,
        }
    }
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build --workspace`
Expected: Compiles. Fix any remaining struct literal errors.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-core/src/stage_display.rs crates/presenter-server/src/state/stage.rs
git commit -m "feat(core): add group_color field to StageDisplaySlide

Optional hex color string resolved server-side. Initially None,
populated by the server's color cache before sending to clients."
```

---

## Task 4: Server-Side Color Resolution (AppState Cache)

**Files:**
- Modify: `crates/presenter-server/src/state/mod.rs`
- Modify: `crates/presenter-server/src/state/stage.rs`
- Modify: `crates/presenter-server/src/state/broadcasting.rs`

- [ ] **Step 1: Add color cache to AppState**

In `crates/presenter-server/src/state/mod.rs`, add a group color cache field to the `AppState` struct (find the struct definition):

```rust
use std::collections::HashMap;
use tokio::sync::RwLock;

// Add to AppState struct:
pub(crate) group_color_cache: Arc<RwLock<HashMap<String, String>>>,
```

Initialize it in the constructor (e.g., `from_config` or `new`):

```rust
let group_colors = repository.load_all_group_colors().await.unwrap_or_default();
// ...
group_color_cache: Arc::new(RwLock::new(group_colors)),
```

- [ ] **Step 2: Add resolve method to AppState**

Add a method (in `state/mod.rs` or a new small file) to resolve a color:

```rust
impl AppState {
    pub(crate) async fn resolve_group_color(&self, name: &str) -> Option<String> {
        // Check cache first
        {
            let cache = self.group_color_cache.read().await;
            if let Some(color) = cache.get(name) {
                return Some(color.clone());
            }
        }
        // Not in cache — resolve from DB (auto-generates if new)
        match self.repository.resolve_group_color(name).await {
            Ok(color) => {
                let mut cache = self.group_color_cache.write().await;
                cache.insert(name.to_string(), color.clone());
                Some(color)
            }
            Err(_) => None,
        }
    }
}
```

- [ ] **Step 3: Enrich slides with color in broadcasting.rs**

In `crates/presenter-server/src/state/broadcasting.rs`, update `enrich_stage_context()` to also resolve group colors:

```rust
    pub(super) async fn enrich_stage_context(&self, context: &StageContext) -> StageContext {
        let mut context = context.clone();
        if context.resolution.override_song_name.is_none() {
            context.resolution.override_song_name = self.resolve_current_song_name().await;
        }
        if context.resolution.next_song_name.is_none() {
            context.resolution.next_song_name =
                self.resolve_next_song_name(&context.resolution).await;
        }
        // Resolve group colors
        if let Some(ref mut slide) = context.resolution.current {
            if let Some(ref name) = slide.group {
                if slide.group_color.is_none() {
                    slide.group_color = self.resolve_group_color(name).await;
                }
            }
        }
        if let Some(ref mut slide) = context.resolution.next {
            if let Some(ref name) = slide.group {
                if slide.group_color.is_none() {
                    slide.group_color = self.resolve_group_color(name).await;
                }
            }
        }
        context
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build -p presenter-server`
Expected: Compiles without errors.

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p presenter-server`
Expected: All existing tests pass (group_color is None by default, so nothing breaks).

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-server/src/state/
git commit -m "feat(server): resolve group colors from cache in stage context

Loads group_colors table into memory on startup. Enriches
StageDisplaySlide with the resolved hex color before sending
to WebSocket clients. Unknown groups get auto-generated colors."
```

---

## Task 5: Update WASM Frontend — Solid Color Pills with Luminance Text

**Files:**
- Modify: `crates/presenter-ui/src/utils/color.rs`
- Modify: `crates/presenter-ui/src/components/stage/worship_snv.rs`
- Modify: `crates/presenter-ui/src/components/stage/worship_pp.rs`
- Modify: `crates/presenter-ui/styles/stage.css`

- [ ] **Step 1: Replace color.rs with luminance utility**

Replace the contents of `crates/presenter-ui/src/utils/color.rs`:

```rust
/// Compute WCAG relative luminance from a hex color string.
/// Returns a value between 0.0 (black) and 1.0 (white).
fn luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return 0.0;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f64 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Returns "#000000" for light backgrounds, "#ffffff" for dark backgrounds.
pub fn text_color_for_bg(hex: &str) -> &'static str {
    if luminance(hex) > 0.4 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// Build inline style for a group pill: solid background + contrast text.
pub fn group_pill_style(bg_color: &str) -> String {
    let text = text_color_for_bg(bg_color);
    format!("background:{bg_color};color:{text};")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_background_gets_white_text() {
        assert_eq!(text_color_for_bg("#2E2E8F"), "#ffffff"); // Muzi - dark blue
        assert_eq!(text_color_for_bg("#1A1A1A"), "#ffffff"); // Pomaly - near black
        assert_eq!(text_color_for_bg("#8B1A1A"), "#ffffff"); // Stevo, Peta - dark red
    }

    #[test]
    fn light_background_gets_black_text() {
        assert_eq!(text_color_for_bg("#F0E020"), "#000000"); // Intro - yellow
        assert_eq!(text_color_for_bg("#E89B7A"), "#000000"); // Tina - salmon
        assert_eq!(text_color_for_bg("#6FE020"), "#000000"); // Verse 2 - green
    }

    #[test]
    fn medium_backgrounds() {
        assert_eq!(text_color_for_bg("#E08A3C"), "#000000"); // Vsetci - orange
        assert_eq!(text_color_for_bg("#3CB371"), "#000000"); // Miro - medium green
        assert_eq!(text_color_for_bg("#9A9A9A"), "#000000"); // Rychlo - gray
    }

    #[test]
    fn group_pill_style_format() {
        let style = group_pill_style("#E02020");
        assert_eq!(style, "background:#E02020;color:#ffffff;");
    }

    #[test]
    fn invalid_hex_defaults_dark() {
        assert_eq!(text_color_for_bg(""), "#ffffff");
        assert_eq!(text_color_for_bg("xyz"), "#ffffff");
    }
}
```

- [ ] **Step 2: Update worship_snv.rs to use group_color from snapshot**

In `crates/presenter-ui/src/components/stage/worship_snv.rs`, replace the import and style closures:

Change import from:
```rust
use crate::utils::color::{group_color, hex_to_rgba};
```
To:
```rust
use crate::utils::color::group_pill_style;
```

Replace `current_group_style` closure:
```rust
    let current_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };
```

Replace `next_group_style` closure:
```rust
    let next_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };
```

- [ ] **Step 3: Update worship_pp.rs identically**

Same changes as worship_snv.rs — replace import and both style closures.

- [ ] **Step 4: Update stage.css for solid pill background**

In `crates/presenter-ui/styles/stage.css`, update `.stage__group-pill`:

```css
.stage__group-pill {
    width: 100%;
    height: 100%;
    overflow: hidden;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    font-weight: 700;
    text-align: center;
    line-height: 0.95;
    padding: 0;
    margin: 0;
}
```

This stays the same — the `background` and `color` now come from inline `style` attributes set by `group_pill_style()`.

- [ ] **Step 5: Run WASM build**

Run: `cargo build -p presenter-ui --target wasm32-unknown-unknown`
Expected: Compiles without errors.

- [ ] **Step 6: Run unit tests**

Run: `cargo test -p presenter-ui -- color --nocapture`
Expected: All luminance tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/presenter-ui/src/utils/color.rs crates/presenter-ui/src/components/stage/worship_snv.rs crates/presenter-ui/src/components/stage/worship_pp.rs crates/presenter-ui/styles/stage.css
git commit -m "feat(ui): solid group pills with WCAG luminance text contrast

Replace hash-based 8-color system with server-provided colors.
Pills render solid background with black/white text chosen by
luminance threshold (0.4). Removes hex_to_rgba opacity approach."
```

---

## Task 6: E2E Test — Verify Group Color on Stage Display

**Files:**
- Create: `tests/e2e/stage-group-colors.spec.ts`

- [ ] **Step 1: Write E2E test**

```typescript
// tests/e2e/stage-group-colors.spec.ts
import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

test("group pill renders with legacy color and correct text contrast", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create library + presentation with a known group "Vsetci" (legacy color #E08A3C)
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `ColorTest Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Test Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presentation: { id: string } = await presResp.json();

  // Add slide with group "Vsetci"
  const slideResp = await request.post(
    new URL(`/presentations/${presentation.id}/slides`, baseURL).toString(),
    {
      data: {
        main: "Test lyrics",
        translation: "",
        stage: "",
        group: "Vsetci",
      },
    },
  );
  expect(slideResp.ok()).toBeTruthy();
  const slide: { id: string } = await slideResp.json();

  // Trigger the slide
  await request.post(new URL("/stage/trigger", baseURL).toString(), {
    data: {
      presentationId: presentation.id,
      slideId: slide.id,
    },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for group pill to appear
  const pill = stagePage.locator(".stage__group-pill").first();
  await expect(pill).toBeVisible({ timeout: 15_000 });
  await expect(pill).toContainText("Vsetci", { timeout: 10_000 });

  // Verify background color is the legacy #E08A3C (rgb(224,138,60))
  const bgColor = await pill.evaluate(
    (el) => getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).toBe("rgb(224, 138, 60)");

  // Verify text color is black (luminance of #E08A3C > 0.4)
  const textColor = await pill.evaluate((el) => getComputedStyle(el).color);
  expect(textColor).toBe("rgb(0, 0, 0)");

  // Clean console
  expect(consoleMessages).toEqual([]);
});

test("unknown group gets auto-generated color", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `AutoColor Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Auto Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presentation: { id: string } = await presResp.json();

  // Add slide with a group NOT in legacy list
  const uniqueGroup = `UniqueGroup${Date.now()}`;
  const slideResp = await request.post(
    new URL(`/presentations/${presentation.id}/slides`, baseURL).toString(),
    {
      data: {
        main: "Auto lyrics",
        translation: "",
        stage: "",
        group: uniqueGroup,
      },
    },
  );
  expect(slideResp.ok()).toBeTruthy();
  const slide: { id: string } = await slideResp.json();

  await request.post(new URL("/stage/trigger", baseURL).toString(), {
    data: {
      presentationId: presentation.id,
      slideId: slide.id,
    },
  });

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const pill = stagePage.locator(".stage__group-pill").first();
  await expect(pill).toBeVisible({ timeout: 15_000 });
  await expect(pill).toContainText(uniqueGroup, {
    timeout: 10_000,
  });

  // Verify it has SOME background color (not transparent/default)
  const bgColor = await pill.evaluate(
    (el) => getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).not.toBe("rgba(0, 0, 0, 0)");
  expect(bgColor).not.toBe("transparent");

  // Verify text is either black or white
  const textColor = await pill.evaluate((el) => getComputedStyle(el).color);
  expect(["rgb(0, 0, 0)", "rgb(255, 255, 255)"]).toContain(textColor);

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run E2E test locally**

Run: `npm run test:playwright -- stage-group-colors`
Expected: Both tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-group-colors.spec.ts
git commit -m "test(e2e): verify group color pills on stage display

Tests legacy color resolution (Vsetci → #E08A3C with black text)
and auto-generation for unknown groups (non-transparent background,
black or white text)."
```

---

## Task 7: Version Bump, Local Checks, Push

- [ ] **Step 1: Bump version**

Check current version and bump patch:
```bash
grep '^version' Cargo.toml | head -1
```

Edit `Cargo.toml` workspace version to the next patch (e.g., `0.4.28`).

- [ ] **Step 2: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.28"
```

- [ ] **Step 3: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace
```

Fix any issues in ONE commit.

- [ ] **Step 4: Build and deploy locally**

```bash
cargo build --release -p presenter-server
sudo systemctl stop presenter-dev
sudo cp target/release/presenter-server /opt/presenter-dev/presenter-server
sudo systemctl start presenter-dev
```

- [ ] **Step 5: Verify on dev**

Open http://10.77.8.134:8080/stage in Playwright, trigger a slide with a known group, verify the pill shows the correct legacy color with readable text.

- [ ] **Step 6: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until all jobs pass.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Legacy colors preserved | Trigger slide with group "Vsetci" → pill background is #E08A3C |
| Text readability | Dark bg (Muzi #2E2E8F) → white text; Light bg (Intro #F0E020) → black text |
| Auto-generation works | New group name → gets a palette color, persisted in DB |
| Solid pill (no opacity) | Pill background is opaque solid color, not rgba with 0.25 |
| Both layouts work | worship-snv and worship-pp both show colored pills |
| No regressions | All existing stage tests pass |
| Clean console | No browser errors or warnings |
