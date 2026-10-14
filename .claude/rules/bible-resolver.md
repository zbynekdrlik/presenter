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

- **Marks = every Unicode mark** (General_Category `M*`, Rust `is_combining_mark`), never Python's
  `unicodedata.combining()` (nonzero class only: it keeps U+034F, so `Li`+U+034F+`st Jánov` folded differently).
- **A spelled ordinal consumes only itself**; `kniha` / `list` / `epistola` after it go through the
  filler words (`Prvý listina` → `listina`). Keep the two ordinal lists in step.
- **NFD test inputs are written as `\uXXXX` escapes** in `fold-cases.json`, so they stay visibly
  different from NFC. Every form, source or `curated.yaml`, is stripped and NFC'd (`normal_form`)
  before it is matched, so an NFD `remove` / `no_fold` / `add` entry still applies.

## Notices ship with the release

`THIRD_PARTY_NOTICES.md` (repo root) and `data/bible-book-names/LICENSE-BPRP.md` are copied into
the public release tarball by release.yml's "Create release archive" step, at the same relative
paths. A new third-party source gets its notice there in the same PR.

## Data decisions (2026-10-13)

`Ab` = Habakkuk (removed from canonical.rs AND listed under `curated.yaml: remove`); `P.Š.` / `PŠ`
= Song of Songs, matched before folding (`no_fold`); `Mr` = Mark at tier 1 (`add`); SEB's stored
misspelling `Filimonovi` stays a key. BPRP entries that list several books (eng `Jo` = John,
Jonah, Job …) are skipped by the generator.

## Resolution rules the corpus pins (A1b)

- `parse` is pure text (no structure); `resolve` binds to the MAIN translation's `Versification`
  and drops candidates whose chapter/verse does not exist — the best disambiguator (`pt 5:7` →
  1 Pt, `ab 2:4` → Hab, `jud 2:5` → Sudcov). EVERY segment is checked (`mt 5:3.99.7`,
  `gn 50:1-51:2`), not only the last verse as the prototype did.
- Order: (tier, kind, penalty), then the family's citation rank, then the code. Keys of at most
  two letters are reordered by the curated `SHORT_DEFAULTS`, which is why `Ambiguous` uses `<=`:
  an alternative whose (tier, kind) is as good as OR BETTER than the default makes it a guess
  (`jd 1`).
- Corpus `alternatives` = what a picker offers. The corpus test requires the FITTING ones to open
  `Resolution::alternatives` in corpus order and the non-fitting ones (2 Ján has no chapter 3) to
  be parse candidates that never fit; `Resolution::alternatives` holds fitting books only.
- `parse` ranks WITHOUT stored names; `resolve` re-ranks with the main translation's stored
  names (a new translation's own names work without regenerating the table). So a corpus
  `alternatives` code must already be a table candidate, and `match: fuzzy` is checked on the
  table-only `parse` candidates.
- Typing in progress is normal input: separators after the parsed numbers with no number behind
  them — `:` `,` `.` `-` and a dangling `až`/`az` (`zid 11:1 až` = `zid 11:1-`) — are swallowed,
  never `leftover`; words after the numbers go to `leftover` (`zid 11:1 až viera` → `viera`); a
  failed tail that still holds a number stays (`zid 11,1:3` → `3`). At most 64 chapter/verse
  tokens are parsed. `split_input` NFC-normalises first: NFD `az` + U+030C is a combining mark,
  not a letter, so without it the `až` range broke.
- An ordinal alone (`1`–`5`, `I.`–`V.`, `prvá` … `piata`, exactly as the fold reads them) is
  `Incomplete`; its `books` are the main translation's books whose table forms or stored names
  carry that ORDINAL (`row.3 & 0x0F`), in canonical order — never the USFM code's digit:
  `1` → 1. Mojžišova (GEN) … 1 Ján, `4.` → 4. Mojžišova + Roháček's `Štvrtá kniha Kráľov` (2KI).
- Single-chapter books (OBA PHM 2JN 3JN JUD): a lone `n > 1` and `n-m` are verses, `1` is the book.
- `counted()` lives here (`hint.rs`); presenter-ui's `state/bible_range.rs` imports it, so the
  Bible page hint and the resolver hints share one wording.
- Rust tests run in CI only (Tier-0). When a rule or the corpus changes, check it first in a
  throwaway Python twin of the Rust code that imports `gen_bible_book_names.py` for the table and
  the fold (the A1b lane's twin scored 100/100 and matched every unit-test value), then port to
  Rust. The structure fixtures are `GET /bible/books` snapshots of dev (= prod = PP).
