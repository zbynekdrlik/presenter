# Resolume Slovak Diacritics — NFC Normalization (Design)

> **Status:** Approved
> **Issue:** #305
> **Created:** 2026-05-06

## Problem

Slovak/Czech text imported from ProPresenter on Mac (e.g., `"401 Po Tebe Pane žíznim"`) renders in Resolume Arena with a `?` after each diacritic letter (e.g., `ž?í?znim`). The same text shows correctly in the operator UI and `/stage` HTML.

Diagnostic capture from prod (`/search?q=znim`):

```
'401 Po Tebe Pane žíznim'
codepoints: [..., 0x7a, 0x30c, 0x69, 0x301, ...]   # NFD form
NFC equal: False
NFD equal: True
```

The data is stored in **NFD form** (decomposed Unicode) — `ž` is `z` + combining caron (U+030C), `í` is `i` + combining acute (U+0301). This is macOS HFS+/APFS filesystem convention; the importer pipes those bytes from `.pro` files straight into the database.

Browsers and modern HTML renderers apply combining marks correctly. Resolume's text effect renderer does not — it shows the base char and renders the combining mark as a missing-glyph placeholder (`?`).

## Goal

Make stored text canonical NFC so every consumer (Resolume, Companion, future surfaces) sees correctly-composed strings without per-consumer normalization.

## Approach

Two pieces, no send-path safety net:

1. **One-time DB migration** — sea-orm migration that scans existing rows in `libraries`, `presentations`, `slides`, normalizes text columns to NFC in place, and is idempotent.
2. **Importer normalizes new imports** — `presenter-importer` calls `.nfc().collect()` on text fields before they reach the database, preventing NFD from being re-introduced by future Mac imports.

The Resolume send path is left untouched. The data layer is the source of truth; consumers send what they read.

## Components

### Migration: `m20260506_000001_normalize_text_to_nfc`

Crate: `crates/presenter-migration/src/m20260506_000001_normalize_text_to_nfc.rs`

Columns covered:

| Table | Column | Notes |
|---|---|---|
| `libraries` | `name` | Display name, source of `presentationName` headers |
| `presentations` | `name` | Visible in operator UI, sent to Resolume |
| `slides` | `worship_main` | Slide text sent to Resolume |
| `slides` | `worship_translate` | Translated slide text |
| `slides` | `worship_stage` | Stage-display text |

`*_search` columns are NOT touched — `fold_query` (used by the search index) does NFD-strip-and-lowercase, so its output is identical regardless of input form.

Algorithm:

```text
for each table in (libraries, presentations, slides):
    for each row:
        for each text column:
            nfc = normalize_to_nfc(value)
            if nfc != value:
                UPDATE row SET column = nfc WHERE id = row.id
```

Use `unicode_normalization::UnicodeNormalization::nfc` for the comparison and rewrite. `is_nfc()` from the same crate is a fast pre-check that avoids materializing the NFC form when the row is already canonical.

Down migration: no-op. NFC is the canonical form; reverting to NFD has no value.

### Importer changes

Crate: `crates/presenter-importer/`

1. `rtf.rs::clean_text` — append `.nfc().collect::<String>()` as the final transformation before returning the cleaned string. All slide text from RTF decoding lands NFC.
2. `lib.rs:285` — wrap `raw.name.clone()` in NFC normalization for the presentation name.
3. `lib.rs` library-name path — wrap the directory-derived library name in NFC.

A small helper `fn to_nfc(s: &str) -> String { s.nfc().collect() }` keeps the call sites readable.

### Cargo.toml updates

Add `unicode-normalization.workspace = true` to:
- `crates/presenter-migration/Cargo.toml`
- `crates/presenter-importer/Cargo.toml`

The workspace declaration `unicode-normalization = "0.1"` already exists in root `Cargo.toml`.

## Tests

### Migration tests (`presenter-migration`)

- **Idempotency:** seed test DB with mixed NFC and NFD rows; run migration; assert all rows NFC; run migration again; assert no row was rewritten.
- **NFD fixtures:** include rows with `"401 Po Tebe Pane z\u{30c}i\u{301}znim"`; after migration, name is `"401 Po Tebe Pane žíznim"` (NFC).
- **Empty/ASCII rows:** rows with `""` or `"abc"` are unchanged.

### Importer tests (`presenter-importer`)

- `rtf.rs`: `clean_text` on `"z\u{30c}i\u{301}"` → `"ží"` (NFC). Existing tests still pass (preserve current cleaning behavior).
- `lib.rs`: import a fixture `.pro` with NFD-form presentation name; assert the resulting `Presentation.name` is NFC.
- Library directory name with NFD characters → stored library name is NFC.

### No new tests in `presenter-server`

Send path is unchanged; existing Resolume tests continue to pass without modification.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Migration corrupts data | Pre-deploy DB backup is automatic (5 backups retained per CLAUDE.md). Migration is idempotent so re-running is safe. |
| Migration too slow on large library (1287 presentations on prod) | NFC normalization is microseconds per string; entire migration runs in <1s on prod. Wrap in a single transaction for atomicity. |
| `unicode-normalization` crate behavior differs across versions | Workspace pin at `0.1` is already exercised by `presenter-core::search`. No new version. |
| Importer changes break existing import workflow | Existing tests assert current behavior; new tests assert NFC. Both run in CI. |

## Out of scope

- Send-path safety net in `resolume::handlers::put_text_param_future`. The data layer is canonical; if any future bug re-introduces NFD, the user prefers to surface and fix it at the source rather than silently mask it.
- Bible text. Bibles import via a different code path (`ingest_bibles`); they have not exhibited the bug. A separate audit can confirm if needed but is not part of this work.
- Operator UI / `/stage` HTML. Browsers handle NFD correctly; once the DB is NFC the question is moot.

## Verification on dev

After deploy:

```bash
curl -s 'http://10.77.8.134:8080/search?q=znim' | python3 -c "
import sys, json, unicodedata
d = json.load(sys.stdin)
for r in d[:3]:
    n = r['presentationName']
    print(n, '— NFC:', unicodedata.normalize('NFC', n) == n)
"
```

Expect every row to print `NFC: True`. Then trigger a slide that pushes worship text to Resolume; confirm in Resolume the diacritic letters render without trailing `?`.
