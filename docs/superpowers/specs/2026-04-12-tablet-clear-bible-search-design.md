# Tablet Clear Button + Bible Search Fix Design

> **Issues:** #222, #220
> **Date:** 2026-04-12

## Problem

1. **#222 — Tablet UI missing clear button:** The tablet UI has no way to clear bible content from stage output. Operators must switch to the desktop operator UI to clear bible display.

2. **#220 — Bible search too strict and slow:** Search uses SQL `LIKE '%term%'` which does full table scans and only matches exact substrings. Users report long "searching" times and inability to find phrases or words.

## Fix #222: Tablet Bible Clear Button

### Location

Add a "Clear" button in the `TabletHeader` component at `crates/presenter-ui/src/pages/tablet.rs` (around line 269, after the text scale slider).

### Behavior

- Button calls existing `bible::clear_broadcast()` API function (`POST /bible/clear`)
- On success: resets `active_slide_id` signal to `None`, shows toast "Bible cleared"
- On error: shows error toast
- Button styled as a small action button matching the tablet UI's existing style
- Always visible at the top of the tablet UI

### API

Already exists — `POST /bible/clear` returns 204 No Content. The API wrapper is at `crates/presenter-ui/src/api/bible.rs:127-128`.

## Fix #220: Bible Search with SQLite FTS5

### Current Implementation

- Endpoint: `GET /bible/search?query=&translation=&limit=`
- Server handler: `crates/presenter-server/src/router/bible.rs:146-166`
- Persistence: `crates/presenter-persistence/src/repository/bible.rs:111-192`
- Uses `LIKE '%term%'` pattern matching — full table scan, no word boundary awareness, no relevance ranking

### New Implementation: FTS5 Virtual Table

**Migration:** Add incremental migration to create FTS5 virtual table:
```sql
CREATE VIRTUAL TABLE IF NOT EXISTS bible_passage_fts USING fts5(
    book,
    content,
    content=bible_passages,
    content_rowid=id
);
```

Populate from existing data:
```sql
INSERT INTO bible_passage_fts(rowid, book, content)
SELECT id, book, content FROM bible_passages;
```

**Triggers:** Add triggers to keep FTS table in sync when passages are inserted/deleted (during bible ingestion):
```sql
CREATE TRIGGER IF NOT EXISTS bible_passage_fts_insert AFTER INSERT ON bible_passages BEGIN
    INSERT INTO bible_passage_fts(rowid, book, content) VALUES (new.id, new.book, new.content);
END;
CREATE TRIGGER IF NOT EXISTS bible_passage_fts_delete AFTER DELETE ON bible_passages BEGIN
    INSERT INTO bible_passage_fts(bible_passage_fts, rowid, book, content) VALUES('delete', old.id, old.book, old.content);
END;
```

**Search query:** Replace the LIKE query with FTS5 MATCH:
```sql
SELECT bp.* FROM bible_passages bp
JOIN bible_passage_fts fts ON bp.id = fts.rowid
WHERE bible_passage_fts MATCH ?
ORDER BY rank
LIMIT ?
```

**Query preprocessing:** Before passing to FTS5, transform user input:
- Split into words
- Append `*` to each word for prefix matching (e.g., "lov" matches "love", "loved", "loving")
- Join with spaces (implicit AND in FTS5)
- Example: user types "god love" → FTS5 query `god* love*`

### Fallback

If FTS5 is not available (shouldn't happen — it's built into SQLite since 3.9.0), fall back to the current LIKE query with a warning log.

## Testing

- **#222:** Playwright E2E — open tablet UI, trigger bible passage, click clear button, verify stage output cleared
- **#220:** Rust integration tests — search for partial words, phrases, verify FTS5 returns relevant results faster than LIKE

## Out of Scope

- Bible search result highlighting
- Search history or autocomplete
- Worship UI rework (#215) — separate project
