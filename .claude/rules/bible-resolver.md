---
paths:
  - "crates/presenter-core/src/bible/resolver/**"
  - "crates/presenter-core/src/bible/canonical.rs"
  - "scripts/dev/gen_bible_book_names.py"
  - "scripts/dev/test_gen_bible_book_names.py"
  - "data/bible-book-names/**"
---

# Bible reference resolver (#850 part A)

## The name table is generated — never hand-edit `names_table.rs`

- Edit a source under `data/bible-book-names/` (the curated rules live in `curated.yaml`) or a
  Slovak array in `canonical.rs`, then run `python3 scripts/dev/gen_bible_book_names.py` and commit
  both. CI's `quality` job runs the generator's unittest and `--check`; a stale table fails it.
- `resolver/mod.rs` declares `#[rustfmt::skip] pub mod names_table;` — the generator owns the
  file's layout, so `cargo fmt --check` and `--check` never fight. Keep that attribute.
- Rows are `(folded key, form as written, book index, tier << 4 | ordinal)`, sorted by key, then
  ordinal, then index. They are unique per **(key, ordinal, book)**; the same (key, book) repeats
  across ordinals on purpose (`kings` → 1KI with and without `1`; `kralov` → 1KI as `1 Kráľov`
  and as Roháček's `Tretia kniha Kráľov`).
- `canonical_book_by_name` (the importer `repository/bible/import.rs`, the AI `load_bible_verses`
  tool, `state/bible_source.rs`) stays exact. The fuzzy resolver must never assign codes at ingest.

## Two folds, one behaviour

`fold()` (Python, builds the keys) and `fold_book_text` (Rust, folds what the operator types)
must agree on every form. Two guards: `resolver/testdata/fold-cases.json` runs through BOTH
(`test_shared_fold_cases` and `fold_matches_the_cases_shared_with_the_generator`), and
`every_generated_row_folds_back_to_its_key` re-folds each row's display form in Rust. Change both
folds in the same commit and add the new case to `fold-cases.json`.

## Data decisions (2026-10-13)

`Ab` = Habakkuk (removed from canonical.rs AND listed under `curated.yaml: remove`); `P.Š.` / `PŠ`
= Song of Songs, matched before folding (`no_fold`); `Mr` = Mark at tier 1 (`add`); SEB's stored
misspelling `Filimonovi` stays a key. BPRP entries that list several books (eng `Jo` = John,
Jonah, Job …) are skipped by the generator.
