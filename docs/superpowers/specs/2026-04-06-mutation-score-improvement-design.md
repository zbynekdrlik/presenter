# Mutation Score Improvement & Summary Parsing Fix

**Date:** 2026-04-06
**Issues:** #207 (improve mutation score), #208 (fix summary parsing)
**Status:** Approved

## Context

Mutation testing was added to CI (PR #206) as an advisory job. The baseline run shows 0% mutation score — no test catches any code mutation. The summary parsing also has bugs: grep-based stdout parsing produces garbled output because cargo-mutants doesn't print CAUGHT lines by default.

## Goals

1. Fix mutation testing summary output to reliably report CAUGHT/MISSED/TIMEOUT counts
2. Write comprehensive unit tests across presenter-core, presenter-bible, and presenter-importer to raise mutation score from 0% toward 30-50%
3. Keep mutation testing advisory (not blocking build) until score stabilizes

## Section 1: Fix Summary Parsing (#208)

**Problem:** The CI job greps stdout for `CAUGHT`/`MISSED`/`TIMEOUT` patterns, but cargo-mutants v27 only prints MISSED to stdout by default. Shell arithmetic breaks on empty values.

**Fix:** Switch to `--output mutants.out` and parse `mutants.out/outcomes.json` with `jq`.

**File:** `.github/workflows/pipeline.yml` (mutation job, lines 322-345)

**Changes:**
- Add `--output mutants.out` to the cargo-mutants command
- Replace grep-based parsing with `jq` queries on `mutants.out/outcomes.json`
- Upload `mutants.out/` directory as artifact (includes outcomes.json, logs)
- Keep `|| true` for advisory mode

**outcomes.json format:**
```json
[
  {"scenario": {"Mutant": {...}}, "log_path": "...", "summary": "CaughtMutant"},
  {"scenario": {"Mutant": {...}}, "log_path": "...", "summary": "MissedMutant"},
  {"scenario": {"Mutant": {...}}, "log_path": "...", "summary": "Timeout"}
]
```

Count by: `jq '[.[] | .summary] | group_by(.) | map({(.[0]): length}) | add'`

## Section 2: Test Hardening — presenter-core

**Target functions with NO tests (CRITICAL):**

| Function | File | Lines | Logic |
|----------|------|-------|-------|
| `extract_song_prefix()` | `ableset.rs` | 153-163 | Numeric extraction from string |
| `ResolumeHostDraft::validate()` | `resolume.rs` | 74-85 | 3 error path validation |
| `OscSettingsDraft::validate()` | `osc.rs` | 63-71 | Port + address validation |
| `normalise_for_search()` | `search.rs` | 42-45 | Unicode normalization + diacritics |
| `query_tokens()` | `search.rs` | 61-63 | Tokenization by non-alphanumeric |

**Tests to write:**
- `extract_song_prefix`: empty string, no digits, "123 Song", "Song 456", boundary lengths
- `ResolumeHostDraft::validate`: empty host, port=0, valid draft
- `OscSettingsDraft::validate`: port=0, empty address, valid settings
- `normalise_for_search`: ASCII passthrough, diacritics removal ("ěščř" → "escr"), case folding
- `query_tokens`: single word, multi-word, special chars, empty input

**Existing weak tests to strengthen (HIGH):**

| Test | File | Issue |
|------|------|-------|
| `CountdownTimer::remaining()` | `timer.rs` | Missing boundary: `now == target` |
| `BibleReference::validate()` | `bible/reference.rs` | Missing: chapter=0, verse=0 |
| `MidiBinding::new(127)` | `playlist.rs` | Only checks `is_ok()`, not value |
| `resolve_sequence` tests | `slide.rs` | Doesn't verify text preservation |

## Section 3: Test Hardening — presenter-bible

**Target functions:**

| Function | File | Lines | Priority |
|----------|------|-------|----------|
| `sanitise_url()` | `provider.rs` | 47-54 | HIGH |
| `mysword_book_code()` | `parsers.rs` | 558-628 | CRITICAL (66 branches) |
| `default_book_name()` | `parsers.rs` | 630-700 | CRITICAL (66 branches) |
| `sanitize_text()` | `parsers.rs` | 417-465 | HIGH |

**Tests to write:**
- `sanitise_url`: alphanumeric passthrough, spaces/special chars replaced with underscore
- `mysword_book_code`: spot-check 10 mappings (1=Gen, 19=Psa, 40=Mat, 66=Rev, plus edge cases)
- `default_book_name`: same spot-check approach
- `sanitize_text`: RTF markers stripped, plain text preserved, nested markers

## Section 4: Test Hardening — presenter-importer

**Target functions:**

| Function | File | Lines | Priority |
|----------|------|-------|----------|
| `resolve_cue_sequence()` | `lib.rs` | 427-506 | CRITICAL (3 paths) |
| `build_group_lookup()` | `lib.rs` | 362-381 | HIGH |
| `classify_text_role()` existing tests | `lib.rs` | 527-534 | MEDIUM (strengthen) |
| `slide_content_from_proto()` existing tests | `lib.rs` | 537-614 | MEDIUM (strengthen) |

**Tests to write:**
- `resolve_cue_sequence`: arrangement path, cue-group fallback, direct ordering
- `build_group_lookup`: anchor assignment (first=true, rest=false), empty input
- Strengthen `classify_text_role`: case variations, all known aliases
- Strengthen `slide_content_from_proto`: verify actual text values not just emptiness

## Section 5: Verification

1. `cargo test --workspace` — all tests pass
2. `cargo fmt --all --check` — formatted
3. Run `cargo mutants` on targeted crates to measure improvement
4. Push to dev, monitor CI
5. Create PR dev → main

## Out of Scope

- `resolve_cache_path()` in provider.rs — depends on environment variables, not worth unit testing
- `is_placeholder_text()` — trivial single-line function
- Macro-generated `id_type!` code — trivial serialization
- Making mutation testing a build blocker — stays advisory until score stabilizes
