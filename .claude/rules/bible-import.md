---
paths:
  - "crates/presenter-bible/src/**"
  - "crates/presenter-importer/src/bible_digest.rs"
  - "crates/presenter-importer/src/bin/ingest_bibles.rs"
  - "crates/presenter-persistence/src/repository/bible/import.rs"
  - "data/bibles/**"
  - "scripts/ci/check_public_bibles.py"
  - "scripts/ci/test_check_public_bibles.py"
---

# Bible import (#851 D1)

## Where the modules live (#859)

- **Only public-domain modules are in this public repo:** `data/bibles/kjv.usfm.zip`. SEB,
  Roháček, SEVP and Milosť are licence-restricted (SEB: "Nešíriť bez povolenia") and live in
  the PRIVATE repo `zbynekdrlik/presenter-bibles`, at its root. Git history before #859 still
  holds them (never rewritten, owner's call).
- **CI:** every job that needs them adds, right AFTER its main checkout (a self-hosted main
  checkout `git clean`s the workspace), the step "Check out the private Bible modules (#859)":
  `actions/checkout@v4` with `repository: zbynekdrlik/presenter-bibles`,
  `ref: <pinned 40-hex commit>`, `ssh-key: ${{ secrets.BIBLES_DEPLOY_KEY }}` (a read-only deploy key),
  `path: data/bibles-private`, `persist-credentials: false`. Today: pipeline.yml `e2e`,
  `e2e-ndi`, `deploy-dev`; deploy.yml `deploy`; release.yml `deploy-pp`; ndi-latency.yml.
  `PRESENTER_BIBLE_SEB/ROHACEK/SEVP/MILOST` and the "Deploy Bible files" scp read
  `data/bibles-private/<file>`; the host paths `/opt/presenter*/bibles/<file>` are unchanged
  (import-data.yml reads only those).
- **The checkout is PINNED, one pin for every workflow.** Every private checkout carries the
  same `ref:` (a full commit SHA of presenter-bibles; its README lists the modules' sha256).
  Without the pin any push to presenter-bibles would reach SNV on the next deploy, and
  redeploying an old release would ship today's modules. **Updating a module = push it to
  presenter-bibles, then bump the pinned SHA in EVERY workflow in ONE commit here** (reviewed
  like code; the new bytes reach SNV/PP with that release). The checker fails on a missing,
  short or branch `ref:` and on two different pins.
- **Local dev:** `git clone git@github.com:zbynekdrlik/presenter-bibles data/bibles-private`
  (gitignored). `tests/e2e/support.ts` `refreshDevData` and the ai_eval recipe point there.
- **A missing module fails loudly, never silently:** an unset `PRESENTER_BIBLE_*` skips that
  translation ("Skipping … (local file not configured)"), but a SET var pointing at a missing
  file makes `ingest_bibles` exit non-zero. CI sets all five, so a failed private checkout
  breaks E2E instead of dropping translations. Never add an "if the file exists" fallback.
- **The public release tarball** (release.yml `build-release`) ships `data/bibles/*.zip`, which
  is the allow-list only; that job never checks out the private repo. PP gets the four from
  `deploy-pp`'s "Deploy Bible files".
- **`scripts/ci/check_public_bibles.py`** (+ its self-test, Quality job) fails when a tracked
  file under `data/bibles/` is off the allow-list (`PUBLIC_MODULES`), when anything under
  `data/bibles-private/` is tracked, when a release-asset job or an `upload-artifact` step
  touches the private modules (public downloads), when a checkout is unpinned or the pins
  differ, when `PRESENTER_BIBLE_SEB/ROHACEK/SEVP/MILOST` points at the old `data/bibles/<module>`,
  or when a job needs the modules before (or without) the private checkout: it names
  `data/bibles-private`, runs the Playwright suite (`playwright test`, `npm test`,
  `npm run test:playwright*`, `tests/e2e`, `refresh-dev-data.sh` — `support.ts` reads the
  private dir) or runs `ingest_bibles` with a repo-relative path. It scans
  `.github/workflows/*.yml` and `*.yaml`. A new public-domain module goes on the allow-list;
  a new restricted one goes to the private repo plus a checkout in each consuming job.

## What a verse may carry (`presenter-bible/src/cleanup.rs`)

- **MySword (SEB, ROH, MIL).** `<FI>[ … ]<Fi>` is a translator's note: it goes WITH the
  whitespace around it, leaving ONE space (so "slovo <FI>[…]<Fi>ďalšie" never fuses into
  "slovoďalšie"; the whitespace pass and the punctuation rule below drop the space where it is
  not needed, else "Tigris ." would reach the wall). A BARE `[ … ]` is translation text
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
A bump re-imports all five translations on the next deploy (saved presentations and sermons
survive — no FK to passages or translations, spec §5.7). Dev and PP import with the service
stopped and the new binaries copied in; SNV's staged deploy imports AFTER the swap and the
`/healthz` wait, with the new service running (one transaction per translation, ~1.5 s each on
dev2). Never before the swap: `/opt/presenter/ingest_bibles` is then the previous release's
binary, which skips everything as up-to-date (D1 nearly shipped that way; pinned by invariants
3–4 of `scripts/ci/check_deploy_workflows.py`). Pin a bump
with a digest test (`the_d1_import_fixes_change_every_stored_digest`), never `assert_eq!` on
the constant.

## Tests

- The SEB/ROH/MIL/SEVP archives are private since #859 (read through `PRESENTER_BIBLE_*` env
  vars; see "Where the modules live"): pin their markup shapes with synthetic archives
  (`import_fixes_tests.rs` `mysword_archive`) — made-up sentences in the real markup, never
  committed module text beyond the spec's own quotes.
- The KJV is public domain and stays in `data/bibles/`: `the_bundled_kjv_has_no_import_defects`
  parses the real zip (31,102 verses, 0 `¶`, 0 footnote residue `+ N.N`, no empty verse, and a
  survival floor of 4,100,000 characters in total: pre-D1 4,377,552, cleaned 4,107,378).
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
