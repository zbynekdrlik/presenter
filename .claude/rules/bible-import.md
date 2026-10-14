---
paths:
  - "crates/presenter-bible/src/**"
  - "crates/presenter-importer/src/bible_digest.rs"
  - "crates/presenter-importer/src/bin/ingest_bibles.rs"
  - "crates/presenter-persistence/src/repository/bible/import.rs"
---

# Bible import (#851 D1)

## What a verse may carry (`presenter-bible/src/cleanup.rs`)

- **MySword (SEB, ROH, MIL).** `<FI>[ … ]<Fi>` is a translator's note: it goes WITH the
  whitespace before it (else "Tigris ." on the wall). A BARE `[ … ]` is translation text
  (ROH Mt 17:21 "[a pôstom]", SEB 1 Thes 3:13 "[Amen.]", SEB Mk 16:20's bracketed shorter
  ending, which itself contains an `<FI>` note): it stays, the preacher strikes it per sermon
  item. `<TS>…<Ts>` is a section heading (ROH, 2,785 verses): it goes, case-insensitive. After
  the tags are stripped, whitespace before `. , ; : ! ?` goes (the modules' `<FR>` quote
  spacing). Order matters: notes and headings BEFORE the tag stripping, spacing AFTER it.
- **USFM (KJV).** `\f … \f*` and `\x … \x*` go WITH their text (the `\ft` text used to reach
  the wall: GEN 2:14 "…Euphrates.+ 2.14 toward…"). A leading `¶` goes AFTER the markers are
  stripped (`\v 16 \wj ¶ \+w For…`). A heading line is never verse text — matched by the
  marker NAME (`s`, `ms`, `mr`, `sr`, `r`, `d`, `sp`, `mt`, `mte`, `cl`, `cd`, `qa`, `h` +
  optional digits + whitespace), not by prefix, so `\sig` is not one. PSA 119's acrostic
  letters ("ב BETH.", 21 verses) and the 14 epistle subscriptions ("Written to the Hebrews
  from Italy by Timothy.", Romans to Hebrews) were glued to the verse before them (35 verses
  in all, counted on the bundled archive by a Python mirror of the parser) — spec §5.1 did
  not list it.
- **Obohu (SEVP)** is unchanged: its 3 rows with a space before punctuation stay (the rule is
  MySword-only, spec §5.1).
- D2a (`markup/`) replaces these helpers, keeps notes and headings as study markup, and must
  reproduce D1's KJV `content` byte for byte (`testdata/kjv-content.sha256`, computed from
  this parser). The 35 glued heading lines become `Heading` segments there, not text.

## Every parse change bumps `PARSER_VERSION`

`presenter-importer/src/bible_digest.rs`: digest = SHA-256(file bytes ‖ version). A parser
change that alters passage text WITHOUT a bump never reaches a deployed database — the stored
digest still matches and `ingest_bibles` prints "Skipping … (up-to-date)". D1 = 2, D2b = 3.
A bump re-imports all five translations on the next deploy (the server is stopped; saved
presentations and sermons survive — no FK to passages or translations, spec §5.7). Pin a bump
with a digest test (`the_d1_import_fixes_change_every_stored_digest`), never `assert_eq!` on
the constant.

## Tests

- The SEB/ROH/MIL/SEVP archives are private after #859 (read through `PRESENTER_BIBLE_*` env
  vars wherever they live): pin their markup shapes with synthetic archives
  (`import_fixes_tests.rs` `mysword_archive`) — made-up sentences in the real markup, never
  committed module text beyond the spec's own quotes.
- The KJV is public domain and stays in `data/bibles/`: `the_bundled_kjv_has_no_import_defects`
  parses the real zip (31,102 verses, 0 `¶`, 0 footnote residue `+ N.N`, 0 backslashes).
- A fixture that pins the old wrong behaviour is corrected IN THE RED COMMIT with the reason in
  the message (D1: two fixtures wrote a note without its `<FI>` wrapper and asserted it vanishes).
- New patterns are `LazyLock<Option<Regex>>` with `every_pattern_compiles` (no `expect` in new
  crate code; the old `BRACKETED_COMMENT_RE` with `.expect` is gone).

## Post-deploy check (T1), SNV and PP, read-only

`ssh -o BatchMode=yes newlevel@presenter.lan "sqlite3 -readonly -header /opt/presenter/presenter.db" < t1.sql`
(PP: `companion-pp.lan`) with the query of Task 1 in
`docs/superpowers/plans/2026-10-14-bible-study-sermon-prep-d1-p4.md`: ROH GEN 1:1 = "Na počiatku
stvoril Bôh nebesia a zem.", 0 KJV rows with `¶` or `+ N.N`, KJV GEN 2:14 ends "Euphrates.",
0 SEB rows `GLOB '* [.,;:!?]*'`, ROH MAT 17:21 ends "[a pôstom].", KJV HEB 13:25 = "Grace be
with you all. Amen.". The deploy log shows five `Imported … (digest …)` lines after a bump.
