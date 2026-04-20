# Group Color System Design

## Goal

Replace the current 8-color hash-based group coloring with a persistent database-backed color system that preserves the ~70 legacy colors singers already know from ProPresenter, auto-generates colors for new groups, and uses solid backgrounds with contrast-aware black/white text for readability.

## Architecture

A new `group_colors` table stores the canonical mapping of group name to hex color. Colors are resolved server-side and sent to the stage display alongside group names. The WASM frontend renders full solid-color pill backgrounds with text color computed via WCAG luminance.

## Database

### Table: `group_colors`

| Column | Type | Constraint |
|--------|------|------------|
| name | TEXT | PRIMARY KEY |
| color | TEXT NOT NULL | Hex color, e.g. `#E08A3C` |

- Seeded on first migration with the ~70 legacy colors listed below.
- When a group name is encountered that has no entry, a color is generated deterministically and inserted.

### Legacy Seed Data

```
Vsetci = #E08A3C
Zeny = #C73E9E
Muzi = #2E2E8F
Zeny/Muzi = #4A6CE0
Peta = #8E44C4
Stevo = #D62828
Miro = #3CB371
Zuzka = #E8631C
Patrika = #D4621E
Tina = #E89B7A
Miro, Peta, Zuzka = #C8304A
Stevo, Peta = #8B1A1A
Stevo, Peta, Zuzka = #D81BC0
Stevo, Zuzka = #2B9D9D
Peta, Miro = #6A2C9E
Miro, Stevo, Zuzka = #5A7A7A
Stevo, zeny = #E8A020
Tina, Zuzka = #B83CA4
Miro, Zuzka = #8A9A3A
Peta // Vsetci = #1A1A1A
Miro, Tina = #A05A2C
Vsetci // Zuzka = #A61E1E
Muzi, Peta = #2E8B57
Muzi, Zuzka = #C83030
Vsetci okrem zuzky = #D47A2C
Stevo, Miro, Tina = #1E2B8F
Vsetci okrem Peti = #9E8AC4
Stevo, Tina = #4A7AD4
Miro, Tina, Zuzka = #8B1E3F
Miro // vsetci = #3CA4E0
Muzi // Zeny = #B8A82C
Vsetci // Peta = #2B5AA6
Peta, Zuzka = #3A5AD4
Patrika, Miro = #8B1E3F
Vsetci // Patrika, Miro = #A89A2C
Peta, Miro // vsetci = #1A1A1A
Patrika, Miro, Zuzka = #C41E5A
Miro, Tina, Patrika = #7A2CA6
Patrika, Stevo = #2B5A6A
Pomaly = #1A1A1A
Rychlejsie = #3A3A3A
Rychlo = #9A9A9A
Bridge 2 = #A0521E
Bridge 1 = #E8831C
Chorus 4 = #8B1E2C
Chorus 3 = #9A9A9A
Inter Chorus = #1E5A8B
Chorus 2 = #C43CB4
PreChorus = #5A1E9E
Intro = #F0E020
Chorus 1 = #E02020
Verse 1 = #2040E0
Verse 2 = #6FE020
Verse 3 = #D41E6A
Verse 4 = #7A2CA6
1. sloha = #2CA64A
2. sloha = #2B6AA6
Peta, Stevo, Zuzka // vsetci = #1A1A1A
Postchorus = #3CB371
Miro, Zuzka // vsetci = #8B1E3F
Patrika, Tina, Stevo = #D62828
Vsetci // Stevo, Peta = #2B7AC4
Patrika, muzi = #D41EA6
Tina, Patrika = #3CB371
```

## Auto-Generation for Unknown Groups

When a group name has no entry in `group_colors`:

1. Compute FNV-1a hash of the group name (case-sensitive, exact match).
2. Map to one of ~20 curated colors that are visually distinct from each other and from the legacy set.
3. Insert the new `(name, color)` row into the database.
4. The color is now permanent for that group name.

### Auto-Generation Palette (avoids legacy colors)

```
#FF6B6B, #4ECDC4, #45B7D1, #96CEB4, #FFEAA7,
#DDA0DD, #98D8C8, #F7DC6F, #BB8FCE, #85C1E9,
#F8C471, #82E0AA, #F1948A, #AED6F1, #D7BDE2,
#A3E4D7, #FAD7A0, #A9CCE3, #D5F5E3, #FADBD8
```

## Text Contrast

Text color is computed client-side from the background hex:

```
R, G, B = parse hex to 0.0-1.0
L = 0.2126*R + 0.7152*G + 0.0722*B
text_color = if L > 0.4 then #000000 else #ffffff
```

Threshold 0.4 (slightly below midpoint) ensures good contrast on medium-brightness backgrounds.

## Pill Rendering

- **Full solid color** — background fills the entire group box area (no opacity, no rounded badge)
- **Text**: bold, uppercase, letter-spacing 0.15-0.18em, centered
- **No border**, no shadow, no translucency
- Current layout positions preserved: top-left for current group, middle-left for next group

## Data Flow

```
Database (group_colors table)
    ↓
Server resolves group name → color
    ↓
StageDisplaySlide { group: Option<String>, group_color: Option<String> }
    ↓
WebSocket → WASM frontend
    ↓
Render: solid background = group_color, text = luminance-derived black/white
```

### StageDisplaySlide Changes

Add `group_color: Option<String>` field (hex string like `#E08A3C`). Sent alongside `group` name. If the server cannot resolve a color (should not happen after migration), the frontend falls back to white text on transparent background.

## Server-Side Resolution

The server maintains an in-memory cache of the `group_colors` table (loaded on startup, updated when new groups are auto-generated). When building `StageDisplaySlide`:

1. Look up `group_name` in the cache.
2. If found → use stored color.
3. If not found → generate via hash, insert into DB, update cache, use generated color.

Cache is a simple `HashMap<String, String>` behind an `RwLock`. Invalidation is not needed because entries are never updated or deleted (append-only table).

## Migration Strategy

Incremental migration: `CREATE TABLE IF NOT EXISTS group_colors (name TEXT PRIMARY KEY, color TEXT NOT NULL)` followed by INSERT statements for all ~70 legacy colors (using `INSERT OR IGNORE` to be idempotent).

## What Does NOT Change

- Group names still come from ProPresenter import (no extraction of ProPresenter's internal color field)
- Group propagation logic (effective_group tracking through slides) stays the same
- Stage layout positions unchanged
- Autofit text scaling for group name text stays the same
- The `color.rs` utility in presenter-ui is replaced, not extended

## Testing

- Unit test: luminance function returns correct black/white for known colors
- Unit test: auto-generation produces consistent color for same name
- Unit test: lookup returns legacy color for known group
- E2E test: stage display shows colored pill for a group, verify computed style has correct background-color
