# Bible Import Fixes + Sermon Server Core (#851 D1, P1–P4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reach #851 milestone M1 ("walls fixed, sermon API on prod"): D1 removes the import defects that reach the walls (ROH headings, KJV footnotes / `¶` / glued heading lines, SEB stray spaces, lost bracketed words; `PARSER_VERSION` 2), and P1–P4 ship the sermon server stack — the pure editing core, persistence, slide generation through the live composer, and the `/bible/sermons*` API with its guards — on SNV and PP.

**Architecture:** D1 adds `presenter-bible/src/cleanup.rs` (MySword notes/headings, USFM notes/pilcrow/heading lines, punctuation spacing); `parsers.rs` only calls it. A sermon IS a Bible presentation: `presenter_core::bible::sermon` (tokens, edits, `render_item`, labels — shared with the later WASM editor) → `bible_sermons` + `bible_sermon_items` (deliberately no FK to translations, text snapshots) and `Repository::save_sermon_change` (ONE revision-guarded transaction for items + slides) → `state/sermons/` (generation through the composer's new `compose_bible_slides_labelled`, the commit path, the character-limit pass, views, writes, the slide guard) → `router/bible/sermons.rs` + `router/access_identity.rs`. Slides get stable v5 ids and play through the existing stored-slide trigger; no new `LiveEvent`.

**Tech Stack:** Rust 2021 (msrv 1.85), axum 0.8, SeaORM 1.1 over sqlx-sqlite 0.8 (foreign keys ON per connection by default), chrono 0.4.44, uuid 1 (`v5`), regex 1, rusqlite + zip (test archives), serde/serde_json, wiremock 0.6 (NLT mock), tracing + `src/tracing_capture.rs`, rstest-free plain `#[test]`s (the quality job's assertion check reads `#[test]` bodies).

**Spec:** `docs/superpowers/specs/2026-10-14-bible-study-sermon-prep-design.md` (approved 2026-10-14, #851): §2 T1/T5–T8, §4, §5.1, §5.7, §7.1–§7.5, §8.2, §9, §11–§15. Baseline `dev` @ b839a71e (v0.4.318, main v0.4.317, #850 O1 merged on dev, not released yet). Every code block below was written into a scratch tree mirroring the repo and passed `rustfmt --check` with the repo's `rustfmt.toml` (syntax-checked, not compiled — Tier-0); the render/label rules and every expected string were checked with a Python mirror, and the D1 rules were simulated on the five real archives.

## Global Constraints

- File cap ≤ 1000 production lines (warn > 800): `bash scripts/dev/count_prod_lines.sh <file>`; function cap ≤ 120 lines (warn > 80): `QC_TARGETS="<newline-separated files>" python3 scripts/dev/fn_length_check.py .` (it measures generic fns too since 2026-10-14). Near-cap files touched here get call sites only: `entities.rs` 927 → new `entities/sermon.rs`; `state/mod.rs` 943 (+1 line); `state/bible.rs` 763 (+11); `router.rs` 762 (+3); `parsers.rs` 701 (+10).
- No `unwrap()` / `expect()` / `panic!` in server, core, persistence or bible production code (tests exempt). New regexes are `LazyLock<Option<Regex>>` with an `every_pattern_compiles` test (the `bible_validator.rs` idiom).
- Tier-0: NO local compiling cargo. Allowed locally: `cargo fmt --all -- --check`, `cargo metadata --offline`, `rustfmt`, Python, `sqlite3 -readonly`. Every Rust test below is verified by CI after the unit's push; a RED commit's failure is argued from the code and the archive simulation, not run locally.
- Incremental migrations only — production data exists (`bible_*` on SNV and PP): a NEW file `m20261014_000003_bible_sermons.rs`, `CREATE … IF NOT EXISTS`, never an edit of an applied migration, appended LAST in `presenter-migration/src/lib.rs` `migrations()` (SeaORM applies pending migrations by set difference, so 000003 before 000001/000002 is fine).
- Public repo: no real e-mails (tests use `preacher@example.org`), RFC 5737 addresses (`192.0.2.x`, `203.0.113.x`), and no SEB/ROH/MIL/SEVP text beyond the spec's own quoted examples (SEB Heb 11:1) — synthetic sentences in the modules' real markup. KJV (public domain) may be quoted.
- Archives are read only through the `PRESENTER_BIBLE_*` env vars; #859 (open) moves SEB/ROH/SEVP/MIL to a private repo. D1 changes no archive path, env var or workflow, so it works wherever the archives live; the KJV corpus test reads `data/bibles/kjv.usfm.zip`, which stays public.
- Version bump = first commit of a unit whenever dev ≤ main (`[workspace.package].version` + `Cargo.lock` via the Task 5 recipe). Expected: D1 v0.4.318 (no bump; dev 0.4.318 > main 0.4.317), P1 0.4.319, P2a 0.4.320, P2b 0.4.321, P3 0.4.322, P4a 0.4.323, P4b 0.4.324, P4c 0.4.325, P4d 0.4.326 — if another release lands first, the recipe picks the next patch and every `vX.Y.Z` below follows.
- D1 is a bug fix: RED commit `test(#851): [red] …` BEFORE GREEN `fix(#851): [green] …`, same PR (regression-test-first; the hook checks the order). P units are features: tests in the same PR, order free.
- Commits: `git commit -F <file>`, subject `type(scope): … (#851 <unit>)`, `[no-test: <reason>]` on commits without tests, message ends with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- Never rewrite history (no amend / `reset --hard` / force push); stage explicit paths only. The checkout is shared (at planning time another lane had an unpushed local commit 15d8cebb): never stage, stash or revert files you did not change; open a dev→main release PR only when dev is quiet (`.claude/rules/quality-gates.md` "Shared dev under a multi-worker fleet").
- Two-branch flow: commit on `dev`, PR dev→main, merge commit only, merge yourself when every check is green and the PR is `mergeable: true` + `mergeable_state: clean` (never `--admin`); PP gets the release via `gh release create vX.Y.Z --target main --generate-notes`.
- CI clippy = `cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::all`, msrv 1.85: at every PR head no unused `pub(crate)` item, re-export or enum variant (each unit adds a `SermonOp` variant together with its first constructor), no unread struct field, inline format args where the argument is a plain identifier (presenter-core/persistence/bible do not allow `uninlined_format_args`), `///` list continuations indented, `ok_or(RepositoryError::NotFound("…"))` never `ok_or_else` for it.
- presenter-core consumers import `presenter_core::bible::sermon::…` (no root `pub use`, so the `lib.rs` re-export E0432 trap does not apply).
- Typed refusals: 404 `RepositoryError::NotFound`, 409 `RepositoryError::Conflict` (stale revision, stale item set, a sermon's slides on the presentation routes) — both through the central `From<anyhow::Error> for AppError`; 422 the state-layer `SermonRefusal` (its own type per the repository-error-pattern rule, mapped ONCE by `sermon_error` in `router/bible/sermons.rs`); 400 only for a malformed request decided in the router.
- Every verse read goes through the `AppState` dispatch in `state/bible_source.rs` (`bible_passage_range`, `list_bible_translations`) — never `self.repository.bible_passage_range` from a new call site (bible-remote rule). The NLT is never an item translation.
- Struct-field additions: grep every literal first (`grep -rn 'BiblePresentation {\|BiblePresentationSummary {' crates tests`) — E0063 surfaces only at CI's clippy job.
- Before a push: load `ci-push-discipline`; after it: monitor to a terminal state per `ci-monitoring` (never `gh run watch`); after a deploy: `post-deploy-verification` (liveness, version from the live DOM, functional).
- PR units ≤ ~600 code+test lines (non-blank, non-comment, rustfmt-formatted, measured on the scratch tree): the spec's §15 estimates were ≈ 2.1× low, so P2–P4 are re-cut into seven vertical units with no dead code at any head (unit map below). P3 (~720) is the one unit over the cap; see "Spec gaps" 2.

Shell variables used below:

```bash
REPO=/home/newlevel/devel/presenter/presenter-dev2
SCRATCH=/tmp/claude-1000/-home-newlevel-devel-presenter-presenter-dev2/d306e5ce-ff0c-4d69-8407-7cf7c1af755a/scratchpad
```

Commit recipe (every "Commit" step): write the message to `$SCRATCH/msg.txt` with the Write tool (subject, blank line, body, blank line, the `Co-Authored-By:` line), then `git -C "$REPO" add <the listed paths> && git -C "$REPO" commit -F "$SCRATCH/msg.txt"`.

Local gates (every unit's "Local gates" step; `<files>` = every `.rs` file the unit created or changed):

```bash
cd "$REPO"
cargo fmt --all -- --check
QC_TARGETS="$(printf '%s\n' <files>)" python3 scripts/dev/fn_length_check.py .
for f in <files>; do echo "$f $(bash scripts/dev/count_prod_lines.sh "$f")"; done
bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .
git status --short    # only your paths (and other lanes' untracked files) may show
```

Ship recipe (every unit's "Ship" step, `<V>` = the unit's version, `<title>` given in the unit):

1. `git -C "$REPO" fetch origin && git -C "$REPO" merge --ff-only origin/dev` (merge normally if dev moved), `git -C "$REPO" push origin dev`; monitor the `pipeline.yml` run to a terminal state — ALL jobs incl. e2e and deploy-dev. A red job: `gh run view <id> --log-failed`, fix the root cause in ONE commit.
2. When dev is quiet: `gh pr create --base main --head dev --title "Release v<V>: <title>" --body-file "$SCRATCH/pr-<unit>.md"` (body: the unit's tasks, its tests, the measured LoC, closing line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`). All checks green + `mergeable_state: clean` → `gh pr merge <N> --merge`.
3. SNV after `deploy.yml`: `curl -fsS http://10.77.9.205/healthz` → `"version":"<V>"`; the operator footer shows `v<V>` in the live DOM (post-deploy-verification skill); then the unit's functional check.
4. PP: `gh release create v<V> --target main --title v<V> --generate-notes`; monitor `release.yml`; `curl -fsS http://companion-pp.lan/healthz` → `<V>`, footer `v<V>`, the unit's PP check.
5. Run `playbook-review` (emit the `📔 Playbook:` line); comment on #851 with the shipped unit, its measured sizes and any FINDING.

## Review Focus

The five inputs the spec implies but its own test list does not exercise, most likely first; each line names the test that pins it in its owning task:

1. **USFM paragraph lines between verses** (`\s1 \tl ב BETH.\tl*` inside PSA 119, `\s1 Written to the Hebrews from Italy by Timothy.` after an epistle's last verse). §5.1 lists five defects; this sixth one is on the walls today in 35 KJV verses (`HEB 13:25` = "Grace be with you all. Amen. Written to the Hebrews…"). Tests: `heading_lines_never_join_a_verse` and `the_bundled_kjv_has_no_import_defects` (Task 2), `heading_lines_are_recognised_by_the_marker_name` (Task 3).
2. **Nested and adjacent MySword markup** (SEB Mk 16:20: a bare `[ … ]` shorter ending that itself contains an `<FI>[ … ]<Fi>` note; `<FR>` quote spacing leaving " ." / " ,"; lower-case `<ts>`). The old `\[[^\]]*\]` regex ate from the outer `[` to the inner `]`. Tests: `bare_brackets_belong_to_the_translation_and_stay`, `whitespace_before_punctuation_goes_in_mysword_modules`, `a_roh_heading_is_dropped_from_its_verse` (Task 2).
3. **Edits at awkward edges** — a dash token at an end cut, an inner verse cut at both edges, a cut verse ending in `.`, every token of a middle verse struck, emphasis on Slovak diacritics. Tests: `an_end_cut_drops_a_trailing_dash_or_comma_but_adds_no_period`, `inner_edges_get_ellipses_but_never_after_a_full_stop`, `a_struck_verse_splits_the_label` (`5-7a, 9`), `emphasis_is_unicode_upper_case_and_struck_beats_emphasized` (Task 8).
4. **Two devices** — a stale revision, a write landing between load and commit, a reorder with a different item set, an add right after another add. Tests: `a_stale_revision_is_a_conflict_and_changes_nothing` (in-transaction check, Task 16), `a_stale_revision_is_refused_before_anything_changes` (Task 31), `deleting_and_reordering_items` (stale set, Task 34), `item_refusals_are_422_409_and_404` (Task 32).
5. **The secondary or item translation is unavailable** — NLT down, a secondary that lacks some kept verses, an item whose translation was uninstalled after the snapshot, a new character limit. Tests: `an_unavailable_nlt_gives_slides_without_the_line_and_a_warning`, `the_secondary_line_has_the_full_kept_verses_and_its_own_book`, `an_item_of_a_removed_translation_still_renders_from_its_snapshot` (Task 19), `a_new_character_limit_regenerates_future_sermons_only` (Task 20).

## Unit map

| Unit | Tasks | Content | Code + tests (measured) | Version | Depends on |
|---|---|---|---|---|---|
| D1 | 1–4 | `cleanup.rs`, `parsers.rs` call sites, `PARSER_VERSION` 2, `bible-import.md` | ≈ 390 | 0.4.318 | — (after #859 preferred, not required) |
| P1 | 5–9 | `presenter_core::bible::sermon` (tokens, labels, dates, edits, render), `sermon-prep.md` | ≈ 636 | 0.4.319 | — |
| P2a | 10–14 | migration 000003, `entities/sermon.rs`, `repository/bible/sermons.rs` create/fetch/dates, `sermon_date` on presentations, `compose_bible_slides_labelled` (no behaviour change) | ≈ 557 | 0.4.320 | P1 |
| P2b | 15–17 | `save_sermon_change` (revision → 409), summaries, date lookups, re-import safety (T6) | ≈ 547 | 0.4.321 | P2a |
| P3 | 18–21 | `state/sermons/{mod,generate,commit}.rs`: generation, commit path, INFO line, character-limit pass | ≈ 720 | 0.4.322 | P2b; #850 O1 (on dev) |
| P4a | 22–25 | `view.rs`, `router/bible/sermons.rs` reads (`GET /bible/sermons`, `/today`, `/{id}`), `access_identity.rs`, `GET /bible/study/identity` | ≈ 656 | 0.4.323 | P3 |
| P4b | 26–29 | create + delete (state + HTTP), `guard.rs` on the presentation slide paths, `sermonDate` in the presentation DTOs | ≈ 499 | 0.4.324 | P4a |
| P4c | 30–32 | `snapshot.rs`, add item + edits (state + HTTP) | ≈ 469 | 0.4.325 | P4b |
| P4d | 33–36 | `maintain.rs`: translation, refresh, delete item, reorder, update, regenerate; architecture + functional docs; M1 check | ≈ 576 | 0.4.326 | P4c |

**Milestone M1** = D1 on SNV and PP (walls fixed) + P4d on SNV and PP (sermon API). Next plan: D2a (markup parsing; consumes D1's KJV output as its content digest).

---

## PR unit D1 — Bible import fixes (v0.4.318, bug fix: RED before GREEN)

Spec §5.1 + T1. Simulated 2026-10-14 on the five archives with a Python mirror of exactly these rules (current parser → D1 parser): KJV `¶` 2,970 → 0, KJV verses with footnote residue `+ N.N` 5,844 → 0, KJV verses with a glued `\s1` line 35 → 0, SEB verses with whitespace before `. , ; : ! ?` 674 → 0, ROH 5 → 0, bare `[…]` kept: SEB 2 / ROH 4 verses, ROH `<TS>` headings 2,785 dropped; KJV verse count 31,102 unchanged; MIL unchanged. (On dev's DB the T1 GLOB counts 548 SEB rows today, not the spec's 490; the target is 0 either way.)

### Task 1: Pre-flight — versions, #859, the "before" values

**Files:** none (read-only).

**Interfaces:** none.

- [ ] **Step 1: Versions.** `cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev`; `git show origin/main:Cargo.toml | grep -m1 '^version'` and `grep -m1 '^version' Cargo.toml`. Expected main 0.4.317 < dev 0.4.318 → D1 needs no bump. If main ≥ dev (a release landed), bump first with the Task 5 commands (Steps 2-3) and use that version for D1.

- [ ] **Step 2: #859.** `gh issue view 859 --json state -q .state` and `gh pr list --search 859 --state open`. D1 touches `crates/presenter-bible/src/{lib.rs,parsers.rs}` (tests + two call sites) and adds files; #859 moves archives and workflow env lines. If a #859 PR is mid-flight, let it merge first and `git merge origin/dev` before Task 2 (spec §10: "#859 → D1 avoids two PRs touching the same import code"). Not a blocker.

- [ ] **Step 3: Record the "before" values on SNV** (read-only, database skill). Write `$SCRATCH/t1.sql`:

```sql
SELECT
  (SELECT content FROM bible_passages WHERE translation_code = 'slk-roh' AND book_code = 'GEN' AND chapter = 1 AND verse_start = 1) AS roh_gen_1_1,
  (SELECT COUNT(*) FROM bible_passages WHERE translation_code = 'eng-kjv' AND content LIKE '%¶%') AS kjv_pilcrows,
  (SELECT substr(content, -45) FROM bible_passages WHERE translation_code = 'eng-kjv' AND book_code = 'GEN' AND chapter = 2 AND verse_start = 14) AS kjv_gen_2_14_end,
  (SELECT COUNT(*) FROM bible_passages WHERE translation_code = 'eng-kjv' AND content GLOB '*+ [0-9]*.[0-9]*') AS kjv_footnotes,
  (SELECT COUNT(*) FROM bible_passages WHERE translation_code = 'slk-seb' AND content GLOB '* [.,;:!?]*') AS seb_space_before_punct,
  (SELECT content FROM bible_passages WHERE translation_code = 'slk-roh' AND book_code = 'MAT' AND chapter = 17 AND verse_start = 21) AS roh_mat_17_21,
  (SELECT content FROM bible_passages WHERE translation_code = 'eng-kjv' AND book_code = 'HEB' AND chapter = 13 AND verse_start = 25) AS kjv_heb_13_25,
  (SELECT group_concat(code || ':' || substr(source_digest, 1, 12), ' ') FROM bible_translations) AS digests;
```

Run `ssh -o BatchMode=yes newlevel@presenter.lan "sqlite3 -readonly -header /opt/presenter/presenter.db" < "$SCRATCH/t1.sql"` (fallback per the project memory: `sshpass -p "$(cat ~/.secrets/presenter-prod-ssh)" ssh …`; if a host lacks the CLI, the database skill's `python3 -c "import sqlite3 …?mode=ro"` form). Expected today: `Stvorenie sveta Na počiatku…`, 2970, `…Euphrates.+ 2.14 toward…: or, eastward to Assyria`, 5844, ≈ 548, `…iba modlitbou .`, `Grace be with you all. Amen. Written to the Hebrews from Italy by Timothy.`, five digests. Keep the output for Task 4.

### Task 2: RED — the import defects as failing tests

**Files:**
- Create: `crates/presenter-bible/src/import_fixes_tests.rs`
- Modify: `crates/presenter-bible/src/lib.rs:1` (declare the test module) and `lib.rs:638-646` (the `make_mysword_archive` fixture's note gets its real `<FI>` wrapper)
- Modify: `crates/presenter-bible/src/parsers.rs:817-823` (the same correction in `sanitize_mysword_strips_bracketed_variants`, renamed)
- Modify: `crates/presenter-importer/src/bible_digest.rs` (test module: one new test)

**Interfaces:**
- Consumes: `parsers::build_ingestion_batch(bytes: &[u8], spec: &BibleTranslationSpec) -> Result<(BibleIngestionBatch, BibleImportSummary)>` (`pub(crate)`, `parsers.rs:22`); the private spec builders `slovak_ekumenicky_spec()` / `king_james_spec()` in `lib.rs` (visible to a child module); `compute_source_digest(file_bytes: &[u8], parser_version: u32) -> String` and `PARSER_VERSION: u32` (`bible_digest.rs`).
- Produces: nothing for later tasks (tests only).

- [ ] **Step 1: Write the failing tests** — `crates/presenter-bible/src/import_fixes_tests.rs`:

```rust
//! #851 D1: the import defects that reached the walls (spec §5.1, T1). The SEB
//! and ROH modules are private after #859, so their markup shapes are rebuilt
//! here as synthetic MySword archives with made-up sentences; the KJV is public
//! domain, so its own lines and the bundled `data/bibles/kjv.usfm.zip` are used.

use super::*;
use presenter_core::BiblePassage;
use regex::Regex;
use rusqlite::Connection;
use std::io::{Cursor, Write};
use tempfile::NamedTempFile;
use zip::write::FileOptions;

const BUNDLED_KJV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/bibles/kjv.usfm.zip"
);

fn zip_with(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
        writer
            .start_file(name, FileOptions::default())
            .expect("zip entry");
        writer.write_all(bytes).expect("zip write");
        writer.finish().expect("zip finish");
    }
    buffer
}

/// A MySword module zip whose `Bible` table holds `rows`
/// (book, chapter, verse, scripture).
fn mysword_archive(rows: &[(i64, i64, i64, &str)]) -> Vec<u8> {
    let temp = NamedTempFile::new().expect("temp file");
    let conn = Connection::open(temp.path()).expect("sqlite");
    conn.execute(
        "CREATE TABLE Bible (Book INTEGER, Chapter INTEGER, Verse INTEGER, Scripture TEXT)",
        [],
    )
    .expect("create table");
    for (book, chapter, verse, scripture) in rows {
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (book, chapter, verse, scripture),
        )
        .expect("insert row");
    }
    drop(conn);
    let bytes = std::fs::read(temp.path()).expect("read sqlite");
    zip_with("MODULE.bbl.mybible", &bytes)
}

/// The verse texts the SEB spec imports from `rows`.
fn mysword_texts(rows: &[(i64, i64, i64, &str)]) -> Vec<String> {
    let (batch, _) =
        parsers::build_ingestion_batch(&mysword_archive(rows), &slovak_ekumenicky_spec())
            .expect("parse MySword archive");
    batch.passages().iter().map(|p| p.text.clone()).collect()
}

/// The passages the KJV spec imports from one USFM document.
fn usfm_passages(usfm: &str) -> Vec<BiblePassage> {
    let archive = zip_with("BOOK.usfm", usfm.as_bytes());
    let (batch, _) =
        parsers::build_ingestion_batch(&archive, &king_james_spec()).expect("parse USFM");
    batch.passages().to_vec()
}

fn text_of(passages: &[BiblePassage], book: &str, chapter: u16, verse: u16) -> String {
    passages
        .iter()
        .find(|p| {
            p.reference.book == book
                && p.reference.chapter == chapter
                && p.reference.verse_start == verse
        })
        .map(|p| p.text.clone())
        .unwrap_or_default()
}

#[test]
fn a_seb_note_goes_with_the_space_before_it() {
    let texts = mysword_texts(&[(
        1,
        2,
        14,
        "Tretia rieka je Tigris <FI>[ Pozn.: iný názov.]<Fi>. Tečie na východ <FI>[ Hebr. slovo.]<Fi>.",
    )]);
    assert_eq!(texts, ["Tretia rieka je Tigris. Tečie na východ."]);
}

#[test]
fn bare_brackets_belong_to_the_translation_and_stay() {
    let texts = mysword_texts(&[
        (
            41,
            16,
            20,
            "Oni odišli <FI>[ V niektorých rkp. chýba.]<Fi>. [Kratší záver textu. Amen <FI>[ Pozn. k záveru.]<Fi>.]",
        ),
        (52, 3, 13, "Nech vás posilní vo svätosti. [Amen.]"),
    ]);
    assert_eq!(
        texts,
        [
            "Oni odišli. [Kratší záver textu. Amen.]",
            "Nech vás posilní vo svätosti. [Amen.]",
        ]
    );
}

#[test]
fn a_roh_heading_is_dropped_from_its_verse() {
    let texts = mysword_texts(&[
        (
            1,
            1,
            1,
            "<TS>Nadpis kapitoly<Ts>  Na začiatku bol text verša. ",
        ),
        (1, 1, 2, "<ts>Iný nadpis<ts> Druhý verš."),
    ]);
    assert_eq!(texts, ["Na začiatku bol text verša.", "Druhý verš."]);
}

#[test]
fn roh_bracketed_words_stay() {
    let texts = mysword_texts(&[(
        40,
        17,
        21,
        " Taký druh nevyženiete inak ako modlitbou [a pôstom]. ",
    )]);
    assert_eq!(
        texts,
        ["Taký druh nevyženiete inak ako modlitbou [a pôstom]."]
    );
}

#[test]
fn whitespace_before_punctuation_goes_in_mysword_modules() {
    let texts = mysword_texts(&[
        (
            40,
            21,
            42,
            "Povedal: <FR> Kameň sa stal základom<Fr> <FR> .<Fr> <FR> Pán to urobil,<Fr>",
        ),
        (
            58,
            4,
            7,
            "Určuje iný <FR> dnes <Fr> , keď hovorí ! Áno ; teda : tak ?",
        ),
    ]);
    assert_eq!(
        texts,
        [
            "Povedal: Kameň sa stal základom. Pán to urobil,",
            "Určuje iný dnes, keď hovorí! Áno; teda: tak?",
        ]
    );
}

#[test]
fn a_kjv_footnote_goes_with_its_text() {
    let passages = usfm_passages(
        r#"\id GEN Genesis
\c 2
\p
\v 14 And the \w name|strong="H8034"\w* of the \w third|strong="H7992"\w* \w river|strong="H5104"\w* \add is\add* \w Hiddekel|strong="H2313"\w*: that \add is\add* it which goeth \w toward|strong="H1980"\w* the \w east|strong="H6926"\w* of \w Assyria|strong="H0804"\w*. And the \w fourth|strong="H7243"\w* \w river|strong="H5104"\w* \add is\add* \w Euphrates|strong="H6578"\w*.\f + \fr 2.14 \ft toward…: or, eastward to Assyria\f*
"#,
    );
    assert_eq!(
        text_of(&passages, "Genesis", 2, 14),
        "And the name of the third river is Hiddekel: that is it which goeth toward the east of Assyria. And the fourth river is Euphrates."
    );
}

#[test]
fn a_leading_pilcrow_goes_even_behind_a_marker() {
    let passages = usfm_passages(
        r#"\id JHN John
\c 3
\v 16 \wj ¶ \+w For|strong="G1063"\+w* \+w God|strong="G2316"\+w* \+w so|strong="G3779"\+w* \+w loved|strong="G0025"\+w* \+w the world|strong="G2889"\+w*\wj*
\v 17 ¶ For God sent not his Son into the world.
"#,
    );
    assert_eq!(
        text_of(&passages, "John", 3, 16),
        "For God so loved the world"
    );
    assert_eq!(
        text_of(&passages, "John", 3, 17),
        "For God sent not his Son into the world."
    );
}

#[test]
fn heading_lines_never_join_a_verse() {
    let passages = usfm_passages(
        r#"\id PSA Psalms
\c 119
\s1 \tl  א ALEPH.\tl*
\q1
\v 8 I will keep thy statutes: O forsake me not utterly.
\s1 \tl  ב BETH.\tl*
\q1
\v 9 Wherewithal shall a young man cleanse his way?
\id HEB Hebrews
\c 13
\v 25 Grace be with you all. Amen.
\s1 Written to the Hebrews from Italy by Timothy.
"#,
    );
    assert_eq!(
        text_of(&passages, "Psalms", 119, 8),
        "I will keep thy statutes: O forsake me not utterly."
    );
    assert_eq!(
        text_of(&passages, "Psalms", 119, 9),
        "Wherewithal shall a young man cleanse his way?"
    );
    assert_eq!(
        text_of(&passages, "Hebrews", 13, 25),
        "Grace be with you all. Amen."
    );
}

#[test]
fn the_bundled_kjv_has_no_import_defects() {
    let bytes = std::fs::read(BUNDLED_KJV).expect("bundled KJV archive");
    let (batch, _) =
        parsers::build_ingestion_batch(&bytes, &king_james_spec()).expect("parse the KJV");
    let passages = batch.passages();
    let footnote = Regex::new(r"\+ \d+\.\d+").expect("footnote pattern");

    assert_eq!(passages.len(), 31_102);
    let pilcrows = passages.iter().filter(|p| p.text.contains('¶')).count();
    let footnotes = passages
        .iter()
        .filter(|p| footnote.is_match(&p.text))
        .count();
    let markers = passages.iter().filter(|p| p.text.contains('\\')).count();
    assert_eq!((pilcrows, footnotes, markers), (0, 0, 0));
    assert!(text_of(passages, "Genesis", 2, 14).ends_with("the fourth river is Euphrates."));
    assert!(text_of(passages, "John", 3, 16).starts_with("For God so loved the world"));
    assert_eq!(
        text_of(passages, "Hebrews", 13, 25),
        "Grace be with you all. Amen."
    );
    assert_eq!(
        text_of(passages, "Psalms", 119, 8),
        "I will keep thy statutes: O forsake me not utterly."
    );
}
```

- [ ] **Step 2: Declare it** — `crates/presenter-bible/src/lib.rs`, replace line 1 `mod parsers;` with:

```rust
mod parsers;

#[cfg(test)]
mod import_fixes_tests;
```

(A `#[cfg(test)] mod x;` DECLARATION is not a file-size boundary for `count_prod_lines.sh`; lib.rs stays at 590 + 3 production lines.)

- [ ] **Step 3: Correct the two fixtures that pinned the defect.** Both wrote a translator's note WITHOUT its `<FI>` wrapper, a shape no SEB verse has (all 969 notes are `<FI>[ … ]<Fi>`), and asserted it vanishes — which is exactly §5.1 row 4's bug (bare brackets are translation text). With the wrapper, both assertions hold before AND after the fix.

`crates/presenter-bible/src/lib.rs` `make_mysword_archive` — replace

```rust
        // Include variant marker to test stripping
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (
                1,
                1,
                1,
                "Na počiatku stvoril Boh nebo a zem. <f>[++]</f> [ Var.: + vám.]",
            ),
        )
```

with

```rust
        // A translator's note in its real `<FI>[ … ]<Fi>` form (#851 D1).
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (
                1,
                1,
                1,
                "Na počiatku stvoril Boh nebo a zem. <f>[++]</f> <FI>[ Var.: + vám.]<Fi>",
            ),
        )
```

`crates/presenter-bible/src/parsers.rs` — replace

```rust
    #[test]
    fn sanitize_mysword_strips_bracketed_variants() {
        assert_eq!(
            sanitize_mysword_text("text [ Var.: + extra] more"),
            "text more"
        );
    }
```

with

```rust
    #[test]
    fn sanitize_mysword_strips_bracketed_notes() {
        // A SEB translator's note is always `<FI>[ … ]<Fi>`; a bare `[ … ]` is
        // translation text and stays (#851 D1, `import_fixes_tests`).
        assert_eq!(
            sanitize_mysword_text("text <FI>[ Var.: + extra]<Fi> more"),
            "text more"
        );
    }
```

- [ ] **Step 4: Pin the parser version** — append inside `mod tests` of `crates/presenter-importer/src/bible_digest.rs` (after `golden_vector_hello_world_v1`):

```rust

    #[test]
    fn the_d1_import_fixes_change_every_stored_digest() {
        // #851 D1 changed the parsed text of every translation: a digest stored
        // under parser version 1 must not match, so the next deploy re-imports
        // all five translations instead of printing "Skipping … (up-to-date)".
        assert_ne!(
            compute_source_digest(b"archive", PARSER_VERSION),
            compute_source_digest(b"archive", 1),
        );
    }
```

- [ ] **Step 5: Why each test is RED on this commit** (CI compiles only the pushed head; the order is pinned by the commit sequence). `a_seb_note_goes_with_the_space_before_it` → "…Tigris . Tečie na východ ." (the old regex keeps the space before `<FI>`); `bare_brackets_belong_to_the_translation_and_stay` → "Oni odišli . .]" (the regex eats from the outer `[` to the note's `]`) and "…svätosti." without "[Amen.]"; `a_roh_heading_is_dropped_from_its_verse` → "Nadpis kapitoly Na začiatku…"; `roh_bracketed_words_stay` → "…modlitbou ."; `whitespace_before_punctuation_goes_in_mysword_modules` → "…základom . Pán…"; `a_kjv_footnote_goes_with_its_text` → "…Euphrates.+ 2.14 toward…"; `a_leading_pilcrow_goes_even_behind_a_marker` → "¶ For God…"; `heading_lines_never_join_a_verse` → "…utterly. ב BETH." and "…Amen. Written to the Hebrews…"; `the_bundled_kjv_has_no_import_defects` → (2970, 5844, 0); `the_d1_import_fixes_change_every_stored_digest` → equal digests (version still 1). The two corrected fixtures stay green.

- [ ] **Step 6: Commit** — paths `crates/presenter-bible/src/import_fixes_tests.rs crates/presenter-bible/src/lib.rs crates/presenter-bible/src/parsers.rs crates/presenter-importer/src/bible_digest.rs`, message:

```
test(#851): [red] import defects that reach the walls (D1)

Synthetic SEB/ROH archives (the modules go private with #859) and the
bundled public-domain KJV: SEB notes leave a space before punctuation,
bare brackets are deleted, ROH headings are glued into verse 1, KJV
footnote text / leading pilcrows / \s1 heading lines (PSA 119 acrostic,
epistle subscriptions) end up in verses, PARSER_VERSION still 1.

Two fixtures wrote a translator's note without its <FI> wrapper (no SEB
verse has that shape) and asserted it vanishes, pinning the bracket bug;
they now use the real <FI>[ ... ]<Fi> form and pass before and after.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 3: GREEN — `cleanup.rs` and the call sites, `PARSER_VERSION` 2

**Files:**
- Create: `crates/presenter-bible/src/cleanup.rs`
- Modify: `crates/presenter-bible/src/lib.rs:1` (`mod cleanup;`)
- Modify: `crates/presenter-bible/src/parsers.rs:3-20` (imports, the old regex goes), `:324` and `:329-334` (USFM call sites), `:417` (new helper above `sanitize_text`), `:467-469` and `:498` (MySword call sites)
- Modify: `crates/presenter-importer/src/bible_digest.rs:10`

**Interfaces:**
- Consumes: the private `sanitize_text` / `sanitize_markup_text` in `parsers.rs` (unchanged).
- Produces (all `pub(crate)`, crate-internal, deleted again by D2a): `strip_mysword_notes_and_headings(input: &str) -> String`, `drop_space_before_punctuation(text: &str) -> String`, `strip_usfm_notes(line: &str) -> Cow<'_, str>`, `strip_leading_pilcrow(text: String) -> String`, `is_usfm_heading_line(line: &str) -> bool`; `PARSER_VERSION = 2`.

- [ ] **Step 1: The helpers with their unit tests** — `crates/presenter-bible/src/cleanup.rs`:

```rust
//! #851 D1 import fixes (spec §5.1): the spans a verse must not carry.
//!
//! - MySword (SEB, ROH, MIL): a translator's note is `<FI>[ … ]<Fi>`; it goes,
//!   with the whitespace before it. A BARE `[ … ]` is translation text (ROH
//!   Mt 17:21 "[a pôstom]", SEB 1 Thes 3:13 "[Amen.]") and stays. A
//!   `<TS>…<Ts>` span is a section heading (ROH, 2,785 verses); it goes. After
//!   the tags are stripped, whitespace before `. , ; : ! ?` goes (the modules'
//!   own spacing around `<FR>` quotes).
//! - USFM (KJV): `\f … \f*` footnotes and `\x … \x*` cross-references go with
//!   their text, a leading `¶` paragraph mark goes, and a heading line (`\s1`,
//!   `\d`, `\ms1`, …) is never verse text (PSA 119's acrostic letters and the
//!   epistles' subscriptions were glued to the verse before them).
//!
//! D2 replaces these helpers with the markup parsers (`markup/`), which keep
//! the notes and headings as study markup instead of dropping them.

use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;

static MYSWORD_NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s*<FI>\s*\[[^\]]*\]\s*<Fi>").ok());
static MYSWORD_HEADING_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?is)<TS>.*?<Ts>").ok());
static USFM_NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\\f\s.*?\\f\*|\\x\s.*?\\x\*").ok());
static SPACE_BEFORE_PUNCTUATION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s+([.,;:!?])").ok());

/// USFM markers whose whole line is a heading or a title, never verse text.
const USFM_HEADING_MARKERS: [&str; 13] = [
    "s", "ms", "mr", "sr", "r", "d", "sp", "mt", "mte", "cl", "cd", "qa", "h",
];

/// `re` applied to `text`; the text unchanged when the pattern failed to
/// compile (it cannot: `every_pattern_compiles` pins all four).
fn replace_all<'a>(re: &LazyLock<Option<Regex>>, text: &'a str, with: &str) -> Cow<'a, str> {
    match re.as_ref() {
        Some(re) => re.replace_all(text, with),
        None => Cow::Borrowed(text),
    }
}

/// MySword: the verse without its `<FI>[ … ]<Fi>` notes and `<TS>…<Ts>`
/// headings. Runs BEFORE the tags are stripped.
pub(crate) fn strip_mysword_notes_and_headings(input: &str) -> String {
    let without_notes = replace_all(&MYSWORD_NOTE_RE, input, "");
    replace_all(&MYSWORD_HEADING_RE, &without_notes, "").into_owned()
}

/// MySword: no whitespace before `. , ; : ! ?`. Runs AFTER the tags are
/// stripped and the whitespace is normalized.
pub(crate) fn drop_space_before_punctuation(text: &str) -> String {
    replace_all(&SPACE_BEFORE_PUNCTUATION_RE, text, "$1").into_owned()
}

/// USFM: the line without `\f … \f*` footnotes and `\x … \x*`
/// cross-references. An unterminated note is left as it is.
pub(crate) fn strip_usfm_notes(line: &str) -> Cow<'_, str> {
    replace_all(&USFM_NOTE_RE, line, "")
}

/// The text without a leading `¶` paragraph mark and the whitespace after it.
pub(crate) fn strip_leading_pilcrow(text: String) -> String {
    if let Some(rest) = text.strip_prefix('¶') {
        return rest.trim_start().to_string();
    }
    text
}

/// Whether a USFM line is a heading or title paragraph (`\s1 …`, `\d …`,
/// `\ms1 …`): the marker NAME decides (letters, then optional digits, then
/// whitespace or the end of the line), so `\sig` or `\s1x` is not a heading.
pub(crate) fn is_usfm_heading_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('\\') else {
        return false;
    };
    let name_len = rest.chars().take_while(char::is_ascii_lowercase).count();
    let (Some(name), Some(after)) = (rest.get(..name_len), rest.get(name_len..)) else {
        return false;
    };
    let after = after.trim_start_matches(|c: char| c.is_ascii_digit());
    let marker_ends = after.is_empty() || after.starts_with(char::is_whitespace);
    marker_ends && USFM_HEADING_MARKERS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(MYSWORD_NOTE_RE.is_some() && MYSWORD_HEADING_RE.is_some());
        assert!(USFM_NOTE_RE.is_some() && SPACE_BEFORE_PUNCTUATION_RE.is_some());
    }

    #[test]
    fn a_mysword_note_needs_its_fi_wrapper() {
        assert_eq!(
            strip_mysword_notes_and_headings("Rieka <FI>[ Pozn.: iný názov.]<Fi>. Tečie"),
            "Rieka. Tečie"
        );
        assert_eq!(
            strip_mysword_notes_and_headings("iba modlitbou [a pôstom]."),
            "iba modlitbou [a pôstom]."
        );
        assert_eq!(
            strip_mysword_notes_and_headings("<FI>slovo<Fi> ostáva"),
            "<FI>slovo<Fi> ostáva"
        );
    }

    #[test]
    fn a_mysword_heading_goes_in_any_case() {
        assert_eq!(
            strip_mysword_notes_and_headings("<TS>Nadpis<Ts>  Text verša."),
            "  Text verša."
        );
        assert_eq!(
            strip_mysword_notes_and_headings("<ts>Iný nadpis<ts> Text."),
            " Text."
        );
    }

    #[test]
    fn whitespace_before_punctuation_goes() {
        assert_eq!(
            drop_space_before_punctuation("slovom . Pán , áno ; nie : prečo ? hej !"),
            "slovom. Pán, áno; nie: prečo? hej!"
        );
        assert_eq!(
            drop_space_before_punctuation("bez zmeny – pomlčka"),
            "bez zmeny – pomlčka"
        );
    }

    #[test]
    fn usfm_notes_go_with_their_text() {
        assert_eq!(
            strip_usfm_notes(r"and years:\f + \fr 1.14 \ft the day…: Heb. between\f*  "),
            "and years:  "
        );
        assert_eq!(
            strip_usfm_notes(r"word\x - \xo 1.1 \xt Jn 1.1\x* rest"),
            "word rest"
        );
        assert_eq!(
            strip_usfm_notes(r"an open \f + note without its end"),
            r"an open \f + note without its end"
        );
    }

    #[test]
    fn a_pilcrow_goes_only_at_the_start() {
        assert_eq!(strip_leading_pilcrow("¶ For God".to_string()), "For God");
        assert_eq!(strip_leading_pilcrow("For ¶ God".to_string()), "For ¶ God");
    }

    #[test]
    fn heading_lines_are_recognised_by_the_marker_name() {
        for line in [
            r"\s1 Written to the Hebrews from Italy by Timothy.",
            r"\s1 \tl  ב BETH.\tl*",
            r"\d A Psalm of David.",
            r"\ms1 The Epistle of Jeremy",
            r"\qa Aleph",
            r"\s",
        ] {
            assert!(is_usfm_heading_line(line), "{line}");
        }
        for line in [
            r"\v 1 In the beginning",
            r"\q1",
            r"\p",
            r"\sig Paul",
            r"\s1x",
            "text",
        ] {
            assert!(!is_usfm_heading_line(line), "{line}");
        }
    }
}
```

- [ ] **Step 2: Declare it** — `crates/presenter-bible/src/lib.rs`, line 1 becomes:

```rust
mod cleanup;
mod parsers;
```

- [ ] **Step 3: `parsers.rs` imports** — replace lines 3-20 (the import block through `BRACKETED_COMMENT_RE`) with:

```rust
use anyhow::{anyhow, Context, Result};
use presenter_core::{bible::BibleIngestionBatch, BiblePassage, BibleReference, BibleTranslation};
use rusqlite::{types::ValueRef, Connection};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io::{Cursor, Read, Write};
use std::str;
use tempfile::NamedTempFile;
use tracing::warn;
use zip::read::ZipArchive;

use crate::cleanup;
use crate::{BibleImportSummary, BibleSourceFormat, BibleTranslationSpec};
```

(`regex::Regex` and `LazyLock` had no other user in `parsers.rs`; left in, they fail `unused_imports` under `-D warnings`.)

- [ ] **Step 4: USFM call sites.** In `parse_usfm_document`, the `\v` branch: replace `let clean_text = sanitize_text(text);` with `let clean_text = clean_usfm_text(text);`. The continuation branch — replace

```rust
        if let Some(builder) = current_builder.as_mut() {
            let clean_text = sanitize_text(line);
            if !clean_text.is_empty() {
```

with

```rust
        if let Some(builder) = current_builder.as_mut() {
            // #851 D1: a heading line (`\s1`, `\d`, …) is never verse text.
            if cleanup::is_usfm_heading_line(line) {
                continue;
            }
            let clean_text = clean_usfm_text(line);
            if !clean_text.is_empty() {
```

(`parse_usfm_document` grows 97 → 101 lines: warn band, under the 120 cap.) Insert above `fn sanitize_text(input: &str) -> String {`:

```rust
/// One USFM line as verse text (#851 D1): footnotes and cross-references out
/// with their text, markers out, no leading `¶` (it sits behind `\wj` in the
/// words of Jesus, so it goes AFTER the markers).
fn clean_usfm_text(text: &str) -> String {
    cleanup::strip_leading_pilcrow(sanitize_text(&cleanup::strip_usfm_notes(text)))
}

```

- [ ] **Step 5: MySword call sites** — in `sanitize_mysword_text`, replace

```rust
    // First strip all bracketed comments like [ Var.: + vám.] or [ Adresa v Efeze chýba...]
    let without_variants = BRACKETED_COMMENT_RE.replace_all(input, "");
```

with

```rust
    // #851 D1: only `<FI>[ … ]<Fi>` notes and `<TS>…<Ts>` headings go; a bare
    // `[ … ]` is translation text and stays.
    let without_variants = cleanup::strip_mysword_notes_and_headings(input);
```

and its last line `    sanitize_markup_text(&output)` with `    cleanup::drop_space_before_punctuation(&sanitize_markup_text(&output))`. (Obohu/SEVP keeps calling `sanitize_markup_text` directly: the spacing rule is MySword-only, §5.1.)

- [ ] **Step 6: Bump the parser version** — `crates/presenter-importer/src/bible_digest.rs:10`:

```rust
/// 2 = #851 D1 (notes, headings, footnotes, `¶`, punctuation spacing).
pub const PARSER_VERSION: u32 = 2;
```

- [ ] **Step 7: How it is verified.** CI `test` runs Task 2's tests green on this head; `strips_strongs_concordance_markup_from_usfm`, `parses_usfm_archive_into_passages`, `parses_mysword_archive_into_passages` and the importer's digest tests stay green; E2E servers re-ingest through the same binary (`refreshDevData`), and no E2E spec asserts a `¶`, a ROH heading or a SEB " ." (grepped 2026-10-14: the "For God so loved the world" strings are request bodies, not DB reads).

- [ ] **Step 8: Commit** — paths `crates/presenter-bible/src/cleanup.rs crates/presenter-bible/src/lib.rs crates/presenter-bible/src/parsers.rs crates/presenter-importer/src/bible_digest.rs`, message:

```
fix(#851): [green] drop notes, headings and footnotes from verse text (D1)

New presenter-bible/src/cleanup.rs: MySword <FI>[...]<Fi> notes go with
the space before them, bare [...] stays, <TS>...<Ts> headings go, no
space before . , ; : ! ?; USFM \f/\x notes go with their text, a leading
pilcrow goes after the markers, \s1/\d/\ms heading lines never join a
verse. PARSER_VERSION 2 re-imports all five translations on deploy.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
```

### Task 4: `bible-import.md`, then ship D1 and check T1 on SNV and PP

**Files:**
- Create: `.claude/rules/bible-import.md`
- Modify: `CLAUDE.md` (one row appended at the end of the Playbook Router table — after the "Cloudflare Access reconcile" row, line 63 today)

**Interfaces:** none.

- [ ] **Step 1: The rule** — `.claude/rules/bible-import.md`:

```markdown
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
(PP: `companion-pp.lan`) with the query of the D1 plan's Task 1: ROH GEN 1:1 = "Na počiatku
stvoril Bôh nebesia a zem.", 0 KJV rows with `¶` or `+ N.N`, KJV GEN 2:14 ends "Euphrates.",
0 SEB rows `GLOB '* [.,;:!?]*'`, ROH MAT 17:21 ends "[a pôstom].", KJV HEB 13:25 = "Grace be
with you all. Amen.". The deploy log shows five `Imported … (digest …)` lines after a bump.
```

`CLAUDE.md` row:

```markdown
| Bible import (#851 D1): MySword `<FI>[…]<Fi>` notes go with the space before them, bare `[…]` stays (translation text), `<TS>` headings go, no space before `. , ; : ! ?`; USFM `\f`/`\x` go with their text, leading `¶` goes after the markers, `\s1`/`\d`/`\ms` lines never join a verse; every parse change bumps `PARSER_VERSION`; synthetic archives (modules private after #859) + the bundled-KJV corpus test; T1 post-deploy SQL | `.claude/rules/bible-import.md` (auto-loads on `presenter-bible/src/**`, `bible_digest.rs`, `ingest_bibles.rs`, `repository/bible/import.rs`) |
```

Commit — paths `.claude/rules/bible-import.md CLAUDE.md`, message `docs(playbook): Bible import rules (#851 D1) [no-test: docs only]` + one body line + the Co-Authored-By line.

- [ ] **Step 2: Local gates** with `<files>` = `crates/presenter-bible/src/cleanup.rs crates/presenter-bible/src/parsers.rs crates/presenter-bible/src/lib.rs crates/presenter-bible/src/import_fixes_tests.rs crates/presenter-importer/src/bible_digest.rs`. Expected: `parsers.rs` ≈ 711 production lines, `cleanup.rs` ≈ 88, no fn over 120.

- [ ] **Step 3: Ship** (Ship recipe) with `<V>` = 0.4.318 and `<title>` = "Bible import fixes: notes, headings, footnotes, pilcrows, punctuation (#851 D1)". The release PR also carries everything else on dev (#850 O1, #855, #858, #860 …) — list it in the body.

- [ ] **Step 4: Functional check on SNV** (after `deploy.yml`): the deploy log shows five `Imported <N> passages for translation <code> (digest …)` lines and no `Skipping`: `gh run view <deploy run id> --log | grep -E 'Imported [0-9]+ passages|Skipping'`. Then `ssh -o BatchMode=yes newlevel@presenter.lan "sqlite3 -readonly -header /opt/presenter/presenter.db" < "$SCRATCH/t1.sql"`. Expected: `Na počiatku stvoril Bôh nebesia a zem.` | 0 | `…the fourth river is Euphrates.` | 0 | 0 | `Ale tento druh nevychádza, iba modlitbou [a pôstom].` | `Grace be with you all. Amen.` | five digests different from Task 1's. One live read through the API the walls use: `curl -fsS -G http://10.77.9.205/bible/passage --data-urlencode translation=slk-roh --data-urlencode 'book=1. Mojžišova' --data-urlencode chapter=1 --data-urlencode verse_start=1` → `"text":"Na počiatku stvoril Bôh nebesia a zem."`. Saved presentations survive (spec §5.7): `SELECT COUNT(*) FROM bible_slides` before (Task 1 run) = after.

- [ ] **Step 5: PP** — after `release.yml`: the same `t1.sql` through `ssh -o BatchMode=yes newlevel@companion-pp.lan "sqlite3 -readonly -header /opt/presenter/presenter.db" < "$SCRATCH/t1.sql"` (python fallback if the CLI is missing there), same expected values.

- [ ] **Step 6:** `playbook-review`; comment on #851: D1 shipped, the Task 1 vs Task 4 numbers, the sixth defect (glued `\s1` lines, 35 KJV verses) as a FINDING, and that D2a's `testdata/kjv-content.sha256` must be computed from this parser's KJV output.

---

## PR unit P1 — the sermon core in presenter-core (v0.4.319)

Spec §7.2–§7.3 (the editing model, rendering, labels), §7.1 limits, §7.6 dates. Pure code, no I/O; the server (P3) and the WASM editor (E2) both call it. Starts after the D1 release (main = dev = 0.4.318).

### Task 5: Post-release version bump to 0.4.319

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.318; Tasks 10, 15, 18, 22, 26, 30, 33 repeat this recipe with their own numbers)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.318 (D1 not released yet), STOP: P1 starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3 (someone else bumped).

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.319 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.319 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 6: Types, tokens, labels and dates

**Files:**
- Create: `crates/presenter-core/src/bible/sermon/mod.rs`, `tokens.rs`, `label.rs`, `dates.rs`, `tests.rs` (exempt from the size gate: `**/tests.rs`)
- Modify: `crates/presenter-core/src/bible/mod.rs:4` (`pub mod sermon;` after `pub mod search;`)

**Interfaces:**
- Consumes: chrono (`Datelike`, `Days`, `NaiveDate`), serde.
- Produces (`presenter_core::bible::sermon::…`): `MAX_ITEM_VERSES: u16 = 60`; `struct SourceVerse { pub verse: u16, pub text: String }`; `struct VerseEdits { pub deleted: Vec<u16>, pub emphasized: Vec<u16> }` (Default); `struct ItemEdits(pub BTreeMap<u16, VerseEdits>)` (`#[serde(transparent)]`, Default, `fn verse(&self, u16) -> Option<&VerseEdits>`); `struct Token { pub start: usize, pub end: usize }`, `fn tokenize(&str) -> Vec<Token>`, `fn token_texts(&str) -> Vec<&str>`; `fn translation_short_code(&str) -> String`; `struct VerseMark { pub verse: u16, pub start_cut: bool, pub end_cut: bool }` + `const fn plain(u16)`; `fn verse_suffix(&[VerseMark]) -> String`; `fn item_label(book_name: &str, chapter: u16, suffix: &str, translation_code: &str) -> String`; `fn default_service_date(NaiveDate) -> NaiveDate`; `fn default_sermon_title(NaiveDate) -> String`. All derive `Debug, Clone, PartialEq, Eq` (+ serde where they cross the wire).

- [ ] **Step 1: Write the tests** — `crates/presenter-core/src/bible/sermon/tests.rs`:

```rust
//! #851 P1: the sermon core rules (spec §7.2–§7.3, §14 "presenter-core").

use super::*;
use chrono::NaiveDate;

/// SEB Heb 11:1, the mockup's example (12 tokens).
const HEB_11_1: &str = "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,";

fn source(verses: &[(u16, &str)]) -> Vec<SourceVerse> {
    verses
        .iter()
        .map(|(verse, text)| SourceVerse {
            verse: *verse,
            text: (*text).to_string(),
        })
        .collect()
}

fn edits(entries: &[(u16, &[u16], &[u16])]) -> ItemEdits {
    ItemEdits(
        entries
            .iter()
            .map(|(verse, deleted, emphasized)| {
                let verse_edits = VerseEdits {
                    deleted: deleted.to_vec(),
                    emphasized: emphasized.to_vec(),
                };
                (*verse, verse_edits)
            })
            .collect(),
    )
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

#[test]
fn tokens_keep_punctuation_and_diacritics() {
    let tokens = token_texts(HEB_11_1);
    assert_eq!(tokens.len(), 12);
    assert_eq!(&tokens[..4], ["Viera", "je", "zárukou", "toho,"]);
    assert_eq!(tokens[11], "nevidíme,");
    assert_eq!(tokenize(HEB_11_1)[2], Token { start: 9, end: 17 });
    assert_eq!(token_texts("  a\u{a0}b  c ").join(" "), "a b c");
}

#[test]
fn short_codes_and_labels() {
    assert_eq!(translation_short_code("slk-seb"), "SEB");
    assert_eq!(translation_short_code("eng-kjv"), "KJV");
    assert_eq!(translation_short_code("kjv"), "KJV");
    assert_eq!(
        item_label("Deuteronómium", 28, "20a", "slk-seb"),
        "Deuteronómium 28:20a (SEB)"
    );
}

#[test]
fn suffix_joins_runs_and_marks_only_their_edges() {
    let plain: Vec<VerseMark> = [5, 6, 7, 9].into_iter().map(VerseMark::plain).collect();
    assert_eq!(verse_suffix(&plain), "5-7, 9");
    let edges = [
        VerseMark {
            verse: 16,
            start_cut: true,
            end_cut: false,
        },
        VerseMark::plain(17),
        VerseMark {
            verse: 18,
            start_cut: false,
            end_cut: true,
        },
    ];
    assert_eq!(verse_suffix(&edges), "16b-18a");
    let single = [VerseMark {
        verse: 1,
        start_cut: false,
        end_cut: true,
    }];
    assert_eq!(verse_suffix(&single), "1a");
    let both = [VerseMark {
        verse: 1,
        start_cut: true,
        end_cut: true,
    }];
    assert_eq!(verse_suffix(&both), "1");
    assert_eq!(verse_suffix(&[]), "");
}

#[test]
fn default_date_is_the_next_sunday_or_today_on_a_sunday() {
    assert_eq!(default_service_date(date("2026-10-14")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-18")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-17")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-19")), date("2026-10-25"));
    assert_eq!(
        default_sermon_title(date("2026-10-19")),
        "Kázeň 19. 10. 2026"
    );
}

#[test]
fn json_shapes_match_the_stored_columns() {
    let stored = edits(&[(1, &[7, 8], &[0])]);
    let json = serde_json::to_string(&stored).expect("serialize");
    assert_eq!(json, r#"{"1":{"deleted":[7,8],"emphasized":[0]}}"#);
    assert_eq!(
        serde_json::from_str::<ItemEdits>(&json).expect("parse"),
        stored
    );
    let partial: ItemEdits = serde_json::from_str(r#"{"3":{"deleted":[1]}}"#).expect("parse");
    assert_eq!(partial, edits(&[(3, &[1], &[])]));
    let verse = serde_json::to_string(&source(&[(1, "Viera")])).expect("serialize");
    assert_eq!(verse, r#"[{"verse":1,"text":"Viera"}]"#);
}
```

- [ ] **Step 2: How it is verified.** CI `test` (Tier-0). Expected values were checked with the Python mirror: "zárukou" spans bytes 9..17 (á is two bytes), NBSP is whitespace for `char::is_whitespace`, 2026-10-14 is a Wednesday → Sunday 2026-10-18, 2026-10-19 (Monday) → 2026-10-25.

- [ ] **Step 3: Implement** — `crates/presenter-core/src/bible/sermon/mod.rs` (Task 7 and 8 add `edits` and `render`):

```rust
//! Sermon preparation core (#851 spec §7.2–§7.3), shared by the server (slide
//! generation) and the WASM editor (instant display), so the two never
//! disagree. A sermon item is one contiguous verse range of one chapter; its
//! text is the database text snapshotted when the item was added
//! ([`SourceVerse`]). The preacher's only edits are struck tokens and
//! emphasized tokens per verse (ROZHODNUTÉ Q2: no rewording). Everything that
//! reaches the wall derives from [`render_item`], and the reference label
//! (`16a`, `16b-18a`, `5-7, 9`) is computed, never typed.
//!
//! Consumers import `presenter_core::bible::sermon::…` (no root re-export).

mod dates;
mod label;
mod tokens;

pub use dates::{default_sermon_title, default_service_date};
pub use label::{item_label, translation_short_code, verse_suffix, VerseMark};
pub use tokens::{token_texts, tokenize, Token};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most verses one sermon item may hold (spec §7.1, decided default 13).
pub const MAX_ITEM_VERSES: u16 = 60;

/// One verse of an item's snapshot (`source_json`: `[{"verse":1,"text":"…"}]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceVerse {
    pub verse: u16,
    pub text: String,
}

/// The edits of one verse: token indices into [`tokenize`] of its source
/// text, sorted and deduplicated by [`normalize_edits`]. A struck token is
/// never emphasized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerseEdits {
    #[serde(default)]
    pub deleted: Vec<u16>,
    #[serde(default)]
    pub emphasized: Vec<u16>,
}

/// An item's edits keyed by verse number
/// (`edits_json`: `{"1":{"deleted":[7,8],"emphasized":[0]}}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemEdits(pub BTreeMap<u16, VerseEdits>);

impl ItemEdits {
    /// The edits of `verse`; `None` when the verse is untouched.
    pub fn verse(&self, verse: u16) -> Option<&VerseEdits> {
        self.0.get(&verse)
    }
}

#[cfg(test)]
mod tests;
```

`crates/presenter-core/src/bible/sermon/tokens.rs`:

```rust
//! Tokens of a verse (#851 spec §7.2): maximal runs of non-whitespace, so
//! "nevidíme," is one token with its punctuation. Imported text is
//! whitespace-normalized, so the kept tokens joined by one space reproduce
//! it, and a snapshot never changes, so token indices stay valid forever.

/// Byte span of one token in its verse text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
}

/// The tokens of `text`, in order.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        match (ch.is_whitespace(), start) {
            (true, Some(begin)) => {
                tokens.push(Token {
                    start: begin,
                    end: index,
                });
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(begin) = start {
        tokens.push(Token {
            start: begin,
            end: text.len(),
        });
    }
    tokens
}

/// The token strings of `text`, in order.
pub fn token_texts(text: &str) -> Vec<&str> {
    tokenize(text)
        .into_iter()
        .filter_map(|token| text.get(token.start..token.end))
        .collect()
}
```

`crates/presenter-core/src/bible/sermon/label.rs`:

```rust
//! Reference labels of sermon items (#851 spec §7.3). The live composer and the
//! sermon generator name a translation through [`translation_short_code`], so
//! "(SEB)" is formatted in one place.

/// "eng-kjv" → "KJV", "slk-seb" → "SEB": the part after the last dash, upper
/// case.
pub fn translation_short_code(code: &str) -> String {
    code.rsplit('-').next().unwrap_or(code).to_uppercase()
}

/// One verse of a label: its number and whether the label marks a cut at its
/// start (`b`) or at its end (`a`). [`render_item`](super::render_item) sets a
/// flag only where a label can express it (`b` on the first verse of a run,
/// `a` on the last); a plain label (the secondary line) sets none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerseMark {
    pub verse: u16,
    pub start_cut: bool,
    pub end_cut: bool,
}

impl VerseMark {
    pub const fn plain(verse: u16) -> Self {
        Self {
            verse,
            start_cut: false,
            end_cut: false,
        }
    }
}

/// The chapter-local part of a label: runs of consecutive verses joined by
/// ", " (`"1a"`, `"16b-18a"`, `"5-7, 9"`). `marks` must be in verse order.
pub fn verse_suffix(marks: &[VerseMark]) -> String {
    let mut runs: Vec<(VerseMark, VerseMark)> = Vec::new();
    for mark in marks {
        let extends = runs
            .last()
            .is_some_and(|(_, last)| last.verse.checked_add(1) == Some(mark.verse));
        if !extends {
            runs.push((*mark, *mark));
        } else if let Some((_, last)) = runs.last_mut() {
            *last = *mark;
        }
    }
    runs.into_iter()
        .map(|(first, last)| run_label(first, last))
        .collect::<Vec<_>>()
        .join(", ")
}

fn run_label(first: VerseMark, last: VerseMark) -> String {
    let (start, end) = (first.verse, last.verse);
    if start == end {
        let letter = match (first.start_cut, first.end_cut) {
            (false, true) => "a",
            (true, false) => "b",
            _ => "",
        };
        return format!("{start}{letter}");
    }
    let start_letter = if first.start_cut { "b" } else { "" };
    let end_letter = if last.end_cut { "a" } else { "" };
    format!("{start}{start_letter}-{end}{end_letter}")
}

/// "Hebrejom 11:1a (SEB)": the book name the translation stores, the chapter,
/// the suffix and the translation's short code.
pub fn item_label(book_name: &str, chapter: u16, suffix: &str, translation_code: &str) -> String {
    let short = translation_short_code(translation_code);
    format!("{book_name} {chapter}:{suffix} ({short})")
}
```

`crates/presenter-core/src/bible/sermon/dates.rs`:

```rust
//! Sermon defaults that depend on the date (#851 spec §7.6, decided default 7).
//! The server computes them from ITS clock and sends them, so a phone's clock
//! never decides.

use chrono::{Datelike, Days, NaiveDate};

/// The next Sunday; today when today is a Sunday.
pub fn default_service_date(today: NaiveDate) -> NaiveDate {
    let ahead = (7 - today.weekday().num_days_from_sunday()) % 7;
    today
        .checked_add_days(Days::new(u64::from(ahead)))
        .unwrap_or(today)
}

/// "Kázeň 19. 10. 2026".
pub fn default_sermon_title(date: NaiveDate) -> String {
    let (day, month, year) = (date.day(), date.month(), date.year());
    format!("Kázeň {day}. {month}. {year}")
}
```

`crates/presenter-core/src/bible/mod.rs` — after `pub mod search;`:

```rust
pub mod sermon;
```

- [ ] **Step 4: Commit** — paths `crates/presenter-core/src/bible/mod.rs crates/presenter-core/src/bible/sermon/`, message `feat(core): sermon types, tokens, labels and dates (#851 P1)` + body "Token = maximal non-whitespace run; labels mark only run edges; the default service date is the next Sunday (today on a Sunday)." + Co-Authored-By.

### Task 7: Edit validation and refresh upkeep

**Files:**
- Create: `crates/presenter-core/src/bible/sermon/edits.rs`
- Modify: `crates/presenter-core/src/bible/sermon/mod.rs` (declare + re-export), `tests.rs` (append)

**Interfaces:**
- Consumes: `tokenize`, `ItemEdits`, `SourceVerse`, `VerseEdits` (Task 6).
- Produces: `enum EditError { UnknownVerse { verse: u16 }, IndexOutOfRange { verse: u16, index: u16 } }` (`thiserror`, Slovak `Display`: "Verš {verse} nie je v tejto položke", "Verš {verse} nemá slovo s indexom {index}"); `fn normalize_edits(&[SourceVerse], &ItemEdits) -> Result<ItemEdits, EditError>`; `fn retain_unchanged_edits(old: &[SourceVerse], new: &[SourceVerse], &ItemEdits) -> ItemEdits`.

- [ ] **Step 1: Append the tests** to `tests.rs`:

```rust

#[test]
fn normalize_drops_untouched_verses_and_refuses_bad_indices() {
    let heb = source(&[(1, HEB_11_1)]);
    assert_eq!(
        normalize_edits(&heb, &edits(&[(1, &[], &[])])).expect("valid"),
        ItemEdits::default()
    );
    assert_eq!(
        normalize_edits(&heb, &edits(&[(1, &[12], &[])])),
        Err(EditError::IndexOutOfRange {
            verse: 1,
            index: 12
        })
    );
    assert_eq!(
        normalize_edits(&heb, &edits(&[(2, &[0], &[])])),
        Err(EditError::UnknownVerse { verse: 2 })
    );
    assert_eq!(
        EditError::UnknownVerse { verse: 2 }.to_string(),
        "Verš 2 nie je v tejto položke"
    );
}

#[test]
fn refresh_keeps_only_the_edits_of_unchanged_verses() {
    let old = source(&[(1, "a b"), (2, "c d"), (3, "e f")]);
    let new = source(&[(1, "a b"), (2, "c x")]);
    let kept = retain_unchanged_edits(
        &old,
        &new,
        &edits(&[(1, &[0], &[]), (2, &[1], &[]), (3, &[0], &[])]),
    );
    assert_eq!(kept, edits(&[(1, &[0], &[])]));
}
```

- [ ] **Step 2: Implement** — `crates/presenter-core/src/bible/sermon/edits.rs`:

```rust
//! Validation and upkeep of a sermon item's edits (#851 spec §7.2, §7.5).

use super::{tokenize, ItemEdits, SourceVerse, VerseEdits};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Why edits were refused (the server answers 422 with this Slovak text).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    #[error("Verš {verse} nie je v tejto položke")]
    UnknownVerse { verse: u16 },
    #[error("Verš {verse} nemá slovo s indexom {index}")]
    IndexOutOfRange { verse: u16, index: u16 },
}

/// The edits checked against `source` and normalized: every verse key exists,
/// every index points at a token, indices are sorted and deduplicated, a
/// struck token is not emphasized, and untouched verses are dropped.
pub fn normalize_edits(source: &[SourceVerse], edits: &ItemEdits) -> Result<ItemEdits, EditError> {
    let mut normalized = BTreeMap::new();
    for (&verse, verse_edits) in &edits.0 {
        let Some(source_verse) = source.iter().find(|candidate| candidate.verse == verse) else {
            return Err(EditError::UnknownVerse { verse });
        };
        let len = tokenize(&source_verse.text).len();
        let deleted = checked_indices(verse, &verse_edits.deleted, len)?;
        let emphasized = checked_indices(verse, &verse_edits.emphasized, len)?;
        let emphasized: Vec<u16> = emphasized.difference(&deleted).copied().collect();
        if deleted.is_empty() && emphasized.is_empty() {
            continue;
        }
        normalized.insert(
            verse,
            VerseEdits {
                deleted: deleted.into_iter().collect(),
                emphasized,
            },
        );
    }
    Ok(ItemEdits(normalized))
}

fn checked_indices(verse: u16, indices: &[u16], len: usize) -> Result<BTreeSet<u16>, EditError> {
    let mut checked = BTreeSet::new();
    for &index in indices {
        if usize::from(index) >= len {
            return Err(EditError::IndexOutOfRange { verse, index });
        }
        checked.insert(index);
    }
    Ok(checked)
}

/// "Obnoviť text" (spec §7.5): the edits of the verses whose text is the same
/// in `old` and `new`; the edits of a changed or vanished verse are cleared.
pub fn retain_unchanged_edits(
    old: &[SourceVerse],
    new: &[SourceVerse],
    edits: &ItemEdits,
) -> ItemEdits {
    let text_of = |source: &[SourceVerse], verse: u16| {
        source
            .iter()
            .find(|candidate| candidate.verse == verse)
            .map(|candidate| candidate.text.clone())
    };
    ItemEdits(
        edits
            .0
            .iter()
            .filter(|(verse, _)| {
                let before = text_of(old, **verse);
                before.is_some() && before == text_of(new, **verse)
            })
            .map(|(verse, verse_edits)| (*verse, verse_edits.clone()))
            .collect(),
    )
}
```

`mod.rs`: the module list becomes `mod dates; mod edits; mod label; mod tokens;` (one per line) and add `pub use edits::{normalize_edits, retain_unchanged_edits, EditError};` after the `dates` re-export.

- [ ] **Step 3: Commit** — paths `crates/presenter-core/src/bible/sermon/`, message `feat(core): validate and normalize sermon edits (#851 P1)` + Co-Authored-By.

### Task 8: `render_item` — the text and the label after the edits

**Files:**
- Create: `crates/presenter-core/src/bible/sermon/render.rs`
- Modify: `crates/presenter-core/src/bible/sermon/mod.rs` (declare + re-export), `tests.rs` (append)

**Interfaces:**
- Consumes: `token_texts`, `verse_suffix`, `VerseMark`, `ItemEdits`, `VerseEdits`, `SourceVerse`; `normalize_edits` (in one test).
- Produces: `struct RenderedVerse { pub verse: u16, pub text: String }`; `struct RenderedItem { pub verses: Vec<RenderedVerse>, pub suffix: String }` with `fn is_empty(&self) -> bool` and `fn numbered_text(&self) -> String` (`"1. text\n3. text"`); `fn render_item(&[SourceVerse], &ItemEdits) -> RenderedItem`.

- [ ] **Step 1: Append the tests** to `tests.rs` — every §7.3 row, the mockup example and the edge cases of Review Focus 3:

```rust

fn texts(item: &RenderedItem) -> Vec<(u16, &str)> {
    item.verses
        .iter()
        .map(|verse| (verse.verse, verse.text.as_str()))
        .collect()
}

#[test]
fn the_mockup_example_cuts_to_1a_with_emphasis() {
    let item = render_item(
        &source(&[(1, HEB_11_1)]),
        &edits(&[(1, &[7, 8, 9, 10, 11], &[0])]),
    );
    assert_eq!(texts(&item), [(1, "VIERA je zárukou toho, v čo dúfame")]);
    assert_eq!(item.suffix, "1a");
    assert_eq!(
        item_label("Hebrejom", 11, &item.suffix, "slk-seb"),
        "Hebrejom 11:1a (SEB)"
    );
    assert_eq!(
        item.numbered_text(),
        "1. VIERA je zárukou toho, v čo dúfame"
    );
}

#[test]
fn a_start_cut_is_b_in_the_original_case() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[0, 1], &[])]));
    assert_eq!(
        texts(&item),
        [(
            1,
            "zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,"
        )]
    );
    assert_eq!(item.suffix, "1b");
}

#[test]
fn a_range_cut_at_both_edges_is_b_to_a() {
    let item = render_item(
        &source(&[
            (16, "Prvý verš má päť slov."),
            (17, "Druhý verš ostáva celý."),
            (18, "Tretí verš končí čiarkou, a pokračuje"),
        ]),
        &edits(&[(16, &[0], &[]), (18, &[4, 5], &[])]),
    );
    assert_eq!(
        texts(&item),
        [
            (16, "verš má päť slov."),
            (17, "Druhý verš ostáva celý."),
            (18, "Tretí verš končí čiarkou"),
        ]
    );
    assert_eq!(item.suffix, "16b-18a");
}

#[test]
fn a_gap_inside_a_verse_reads_as_an_ellipsis() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[3, 4, 5], &[])]));
    assert_eq!(
        texts(&item),
        [(
            1,
            "Viera je zárukou … dúfame, a zdôvodnením toho, čo nevidíme,"
        )]
    );
    assert_eq!(item.suffix, "1");
}

#[test]
fn one_verse_cut_at_both_ends_has_ellipses_and_no_letter() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[0, 1, 11], &[])]));
    assert_eq!(
        texts(&item),
        [(1, "… zárukou toho, v čo dúfame, a zdôvodnením toho, čo …")]
    );
    assert_eq!(item.suffix, "1");
}

#[test]
fn inner_edges_get_ellipses_but_never_after_a_full_stop() {
    let item = render_item(
        &source(&[
            (16, "Prvý verš končí bodkou. A pokračuje"),
            (17, "Stredný verš má na začiatku aj na konci niečo navyše,"),
            (18, "Posledný verš."),
        ]),
        &edits(&[(16, &[4, 5], &[]), (17, &[0, 9], &[])]),
    );
    assert_eq!(
        texts(&item),
        [
            (16, "Prvý verš končí bodkou."),
            (17, "… verš má na začiatku aj na konci niečo …"),
            (18, "Posledný verš."),
        ]
    );
    assert_eq!(item.suffix, "16-18");
}

#[test]
fn a_struck_verse_splits_the_label() {
    let item = render_item(
        &source(&[
            (5, "a b,"),
            (6, "c d"),
            (7, "e f, g"),
            (8, "h i"),
            (9, "j k"),
        ]),
        &edits(&[(7, &[2], &[]), (8, &[0, 1], &[])]),
    );
    assert_eq!(
        texts(&item),
        [(5, "a b,"), (6, "c d"), (7, "e f"), (9, "j k")]
    );
    assert_eq!(item.suffix, "5-7a, 9");
}

#[test]
fn an_end_cut_drops_a_trailing_dash_or_comma_but_adds_no_period() {
    let item = render_item(
        &source(&[(
            20,
            "Zošle kliatbu na všetko, čo budeš podnikať — až kým nezahynieš",
        )]),
        &edits(&[(20, &[8, 9, 10], &[])]),
    );
    assert_eq!(
        texts(&item),
        [(20, "Zošle kliatbu na všetko, čo budeš podnikať")]
    );
    assert_eq!(item.suffix, "20a");
}

#[test]
fn an_item_with_every_token_struck_renders_nothing() {
    let item = render_item(
        &source(&[(1, "a b"), (2, "c d")]),
        &edits(&[(1, &[0, 1], &[]), (2, &[0, 1], &[])]),
    );
    assert!(item.is_empty());
    assert_eq!(item.suffix, "1-2");
}

#[test]
fn emphasis_is_unicode_upper_case_and_struck_beats_emphasized() {
    let item = render_item(
        &source(&[(14, "aby sme skrze vieru dostali prisľúbeného Ducha.")]),
        &edits(&[(14, &[], &[3, 4, 5])]),
    );
    assert_eq!(
        texts(&item),
        [(14, "aby sme skrze VIERU DOSTALI PRISĽÚBENÉHO Ducha.")]
    );
    let normalized = normalize_edits(
        &source(&[(1, HEB_11_1)]),
        &edits(&[(1, &[2, 0, 2], &[0, 4, 4])]),
    )
    .expect("valid edits");
    assert_eq!(normalized, edits(&[(1, &[0, 2], &[4])]));
}
```

- [ ] **Step 2: Implement** — `crates/presenter-core/src/bible/sermon/render.rs`:

```rust
//! The text and the label of a sermon item after its edits (#851 spec §7.3):
//!
//! - an end cut on the last verse of a run is `16a`, its trailing `, ; : – —`
//!   goes; a start cut on the first verse of a run is `16b`, original case;
//! - a cut the label cannot express (an inner verse, the inner edge of a run's
//!   first or last verse, one verse cut at both ends) gets a leading `… `
//!   and/or a trailing ` …`, never a trailing ellipsis after `. ! ?`;
//! - a gap with kept words on both sides reads ` … `;
//! - a wholly struck verse is omitted and splits the label (`5-7, 9`);
//! - an emphasized token is upper case ("vieru" → "VIERU").

use super::label::{verse_suffix, VerseMark};
use super::{token_texts, ItemEdits, SourceVerse, VerseEdits};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One verse as it goes on the wall. The slide line stays
/// `"{verse}. {text}"`: the integer verse prefixes it, the label carries the
/// letter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedVerse {
    pub verse: u16,
    pub text: String,
}

/// An item after its edits. `verses` is empty when every token is struck (the
/// item produces no slides); `suffix` then names the whole source range, so
/// the editor can still label the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedItem {
    pub verses: Vec<RenderedVerse>,
    pub suffix: String,
}

impl RenderedItem {
    pub fn is_empty(&self) -> bool {
        self.verses.is_empty()
    }

    /// `"1. text\n2. text"`: the slides' line format.
    pub fn numbered_text(&self) -> String {
        self.verses
            .iter()
            .map(|verse| format!("{}. {}", verse.verse, verse.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const END_CUT_TRIM: &[char] = &[',', ';', ':', '–', '—'];

/// One verse with its kept tokens, before its place in a run is known.
struct KeptVerse {
    verse: u16,
    start_cut: bool,
    end_cut: bool,
    body: String,
}

/// The item's verses and label suffix after `edits` (normalized or not: an
/// index past the verse's end is simply never matched).
pub fn render_item(source: &[SourceVerse], edits: &ItemEdits) -> RenderedItem {
    let kept: Vec<KeptVerse> = source
        .iter()
        .filter_map(|verse| keep_verse(verse, edits.verse(verse.verse)))
        .collect();
    if kept.is_empty() {
        let marks: Vec<VerseMark> = source.iter().map(|v| VerseMark::plain(v.verse)).collect();
        return RenderedItem {
            verses: Vec::new(),
            suffix: verse_suffix(&marks),
        };
    }
    let marks = label_marks(&kept);
    let verses = kept
        .iter()
        .zip(&marks)
        .map(|(verse, mark)| RenderedVerse {
            verse: verse.verse,
            text: finish_text(verse, mark),
        })
        .collect();
    RenderedItem {
        verses,
        suffix: verse_suffix(&marks),
    }
}

fn keep_verse(source: &SourceVerse, edits: Option<&VerseEdits>) -> Option<KeptVerse> {
    let tokens = token_texts(&source.text);
    let deleted: BTreeSet<u16> = edits
        .map(|e| e.deleted.iter().copied().collect())
        .unwrap_or_default();
    let emphasized: BTreeSet<u16> = edits
        .map(|e| e.emphasized.iter().copied().collect())
        .unwrap_or_default();
    let mut body = String::new();
    let mut gap = false;
    for (index, token) in tokens.iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        if deleted.contains(&index) {
            gap = !body.is_empty();
            continue;
        }
        if !body.is_empty() {
            body.push_str(if gap { " … " } else { " " });
        }
        if emphasized.contains(&index) {
            body.push_str(&token.to_uppercase());
        } else {
            body.push_str(token);
        }
        gap = false;
    }
    if body.is_empty() {
        return None;
    }
    let last = u16::try_from(tokens.len().saturating_sub(1)).unwrap_or(u16::MAX);
    Some(KeptVerse {
        verse: source.verse,
        start_cut: deleted.contains(&0),
        end_cut: deleted.contains(&last),
        body,
    })
}

/// Which cuts the label can express: `b` on the first verse of a run, `a` on
/// the last; nothing on a single verse cut at both ends.
fn label_marks(kept: &[KeptVerse]) -> Vec<VerseMark> {
    let follows = |a: &KeptVerse, b: &KeptVerse| a.verse.checked_add(1) == Some(b.verse);
    kept.iter()
        .enumerate()
        .map(|(index, verse)| {
            let first = index
                .checked_sub(1)
                .and_then(|previous| kept.get(previous))
                .is_none_or(|previous| !follows(previous, verse));
            let last = kept.get(index + 1).is_none_or(|next| !follows(verse, next));
            if first && last && verse.start_cut && verse.end_cut {
                return VerseMark::plain(verse.verse);
            }
            VerseMark {
                verse: verse.verse,
                start_cut: first && verse.start_cut,
                end_cut: last && verse.end_cut,
            }
        })
        .collect()
}

fn finish_text(verse: &KeptVerse, mark: &VerseMark) -> String {
    let mut text = if verse.end_cut {
        trim_end_cut(&verse.body)
    } else {
        verse.body.clone()
    };
    if verse.start_cut && !mark.start_cut {
        text = format!("… {text}");
    }
    if verse.end_cut && !mark.end_cut && !text.ends_with(['.', '!', '?']) {
        text = format!("{text} …");
    }
    text
}

/// The body without its trailing `, ; : – —` (and the spaces they leave);
/// unchanged when nothing else would remain.
fn trim_end_cut(body: &str) -> String {
    let mut text = body;
    loop {
        let trimmed = text.trim_end_matches(END_CUT_TRIM).trim_end();
        if trimmed.len() == text.len() {
            break;
        }
        text = trimmed;
    }
    if text.is_empty() {
        body.to_string()
    } else {
        text.to_string()
    }
}
```

`mod.rs` — the final module list and re-exports (this is the whole head of the file after Task 8):

```rust
mod dates;
mod edits;
mod label;
mod render;
mod tokens;

pub use dates::{default_sermon_title, default_service_date};
pub use edits::{normalize_edits, retain_unchanged_edits, EditError};
pub use label::{item_label, translation_short_code, verse_suffix, VerseMark};
pub use render::{render_item, RenderedItem, RenderedVerse};
pub use tokens::{token_texts, tokenize, Token};
```

- [ ] **Step 3: How it is verified.** CI. The expected strings come from a Python mirror of these exact rules (run 2026-10-14): mockup → `(1, "VIERA je zárukou toho, v čo dúfame")` + `1a`; `[0, 1]` → `1b`; `16b-18a`; `[3, 4, 5]` → "Viera je zárukou … dúfame, …"; `[0, 1, 11]` → "… zárukou … čo …" + `1`; inner edges `16-18`; `5-7a, 9`; dash → "…podnikať" + `20a`; all struck → `1-2`. `text.ends_with(['.', '!', '?'])` uses the `[char; N]` pattern (stable since Rust 1.51 — inside msrv 1.85); `Option::is_none_or` is stable since 1.82.

- [ ] **Step 4: Commit** — paths `crates/presenter-core/src/bible/sermon/`, message `feat(core): render sermon items and compute their labels (#851 P1)` + body "§7.3: letters on run edges, ellipses elsewhere, end cuts trim , ; : – —, struck verses split the label, emphasis is upper case." + Co-Authored-By.

### Task 9: `sermon-prep.md`, ship P1, mutation sweep

**Files:**
- Create: `.claude/rules/sermon-prep.md`
- Modify: `CLAUDE.md` (one row appended at the end of the Playbook Router table)

**Interfaces:** none.

- [ ] **Step 1: The rule** — `.claude/rules/sermon-prep.md` (later units append sections):

```markdown
---
paths:
  - "crates/presenter-core/src/bible/sermon/**"
  - "crates/presenter-migration/src/m20261014_000003_bible_sermons.rs"
  - "crates/presenter-persistence/src/entities/sermon.rs"
  - "crates/presenter-persistence/src/repository/bible/sermons.rs"
  - "crates/presenter-persistence/src/repository/bible/sermons/**"
  - "crates/presenter-server/src/state/sermons/**"
  - "crates/presenter-server/src/router/bible/sermons.rs"
  - "crates/presenter-server/src/router/bible/sermons/**"
  - "crates/presenter-server/src/router/access_identity.rs"
---

# Sermon preparation (#851 P and E units)

## The core — `presenter_core::bible::sermon` (P1), shared by the server and the WASM editor

- **Edits are deletions + emphasis only** (ROZHODNUTÉ Q2, no rewording):
  `ItemEdits(BTreeMap<verse, VerseEdits { deleted, emphasized }>)`, token indices into
  `tokenize(source text)` = maximal non-whitespace runs ("nevidíme," is one token). A
  snapshot never changes, so the indices stay valid; only a translation change or a refresh
  re-snapshots (spec §7.5). `normalize_edits` sorts, deduplicates, drops struck-and-emphasized
  emphasis and untouched verses, and refuses an unknown verse / an index past the verse with a
  Slovak `EditError` (the server answers 422 with its text).
- **Everything the wall shows derives from `render_item(source, edits)`.** Stored slide text
  is a cache that every sermon write regenerates; never treat it as the truth.
- **Label rules (§7.3, decided defaults 3–5):** letters only on the EDGES of a run of
  consecutive kept verses (`b` = start cut on the run's first verse, `a` = end cut on its last;
  `16b-18a`); a cut the label cannot express (an inner verse, the inner edge of a run's first or
  last verse, one verse cut at both ends) reads `… ` / ` …`; an end cut trims a trailing
  `, ; : – —` (also before a trailing ellipsis) and never adds a period; no trailing ellipsis
  after `. ! ?`; a gap inside a verse reads ` … `; a wholly struck verse splits the label
  (`5-7, 9`); every token struck → no verses and the suffix names the whole source range (the
  editor still labels the card); emphasis = Unicode upper case ("vieru" → "VIERU").
- **One `translation_short_code`** (core `label.rs`) formats "(SEB)" for the live composer and
  the sermon labels. `resolume/handlers.rs` keeps its own older copy (out of scope).
- `MAX_ITEM_VERSES` = 60. `default_service_date(today)` = the next Sunday (today on a Sunday),
  `default_sermon_title(date)` = "Kázeň D. M. YYYY" — computed from the SERVER's date
  (`state::sermons::local_today`), never the phone's.
- Tests are plain `#[test]` fns with inline `assert_eq!` (the quality job's assertion check reads
  `#[test]` bodies; a helper-only body fails it). Check new expected strings in a throwaway
  Python mirror first (Tier-0, no local `cargo test`), then port.
- After P1 merges: `/mutation-sweep` (`gh workflow run mutation-full.yml --ref main`); a survivor
  under `bible/sermon/` is P1 work, killed on `dev` with a test, never a follow-up ticket.
```

`CLAUDE.md` row:

```markdown
| Sermon preparation (#851 P/E): core = deletions + emphasis on token indices, `render_item` is the only truth for wall text, labels mark only run edges (`16b-18a`, `5-7, 9`), one `translation_short_code`; persistence = sermon IS a presentation, NO FK to translations (re-import safety), ONE revision-guarded `save_sermon_change`; generation via `compose_bible_slides_labelled`, v5 slide ids, secondary = full kept verses; API refusals 404/409 central, 422 `SermonRefusal` via `sermon_error`; sermon slides read-only on the presentation routes | `.claude/rules/sermon-prep.md` (auto-loads on `bible/sermon/**`, the sermon migration/entities/repository, `state/sermons/**`, `router/bible/sermons*`, `router/access_identity.rs`) |
```

Commit — paths `.claude/rules/sermon-prep.md CLAUDE.md`, message `docs(playbook): sermon core rules (#851 P1) [no-test: docs only]` + body line + Co-Authored-By.

- [ ] **Step 2: Local gates** with `<files>` = the six `crates/presenter-core/src/bible/sermon/*.rs` files and `crates/presenter-core/src/bible/mod.rs`. Expected: `render.rs` the largest (≈ 180 lines total), no fn over 80.

- [ ] **Step 3: Ship** with `<V>` = 0.4.319, `<title>` = "Sermon core: tokens, edits, rendering, labels (#851 P1)". Functional check on SNV/PP: version only (no runtime surface yet) + `/healthz` 200.

- [ ] **Step 4: Mutation sweep (spec §14).** After the release: `gh workflow run mutation-full.yml --ref main`, monitor to a terminal state, download the report; every surviving mutant under `crates/presenter-core/src/bible/sermon/` gets a killing test on `dev` in the next unit's first feature commit (P2a), not a follow-up ticket.

- [ ] **Step 5:** `playbook-review`; comment on #851 (P1 shipped, the mutation result).

---

## PR unit P2a — sermon schema, reads, `sermon_date`, caller labels for the composer (v0.4.320)

Spec §7.1 (data model), §5.7 (no FK to translations), §7.4 first bullet (composer refactor, no behaviour change). Everything here is either read by the next unit or has a production caller now (`compose_bible_slides` calls the labelled form; `sermon_date` fills the presentation types), so no `dead_code` at the head.

### Task 10: Post-release version bump to 0.4.320

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.319)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.319 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.320 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.320 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 11: Migration `m20261014_000003_bible_sermons`

**Files:**
- Create: `crates/presenter-migration/src/m20261014_000003_bible_sermons.rs`
- Modify: `crates/presenter-migration/src/lib.rs:26` (module list: `mod m20261014_000003_bible_sermons;` after `mod m20260920_000001_seed_timer_output;`) and `:58` (`Box::new(m20261014_000003_bible_sermons::Migration),` as the LAST entry of `migrations()`)

**Interfaces:**
- Consumes: the existing `bible_presentations` table (`m20260410_000001_separate_bible`).
- Produces: tables `bible_sermons` (PK `presentation_id` → `bible_presentations(id)` ON DELETE CASCADE ON UPDATE CASCADE; `service_date` TEXT `YYYY-MM-DD`; `default_translation_code`; nullable `secondary_translation_code`, `author_email`; `revision` INTEGER default 0; timestamps) and `bible_sermon_items` (PK `id`; `presentation_id` → `bible_sermons(presentation_id)` ON DELETE CASCADE; `item_order`; `translation_code` with NO foreign key; `book_code`, `book_number`, `book_name`, `chapter`, `verse_start`, `verse_end`, `source_json`, `edits_json`, `slide_count` default 0; timestamps); indexes `idx_bible_sermons_service_date`, `idx_bible_sermon_items_order (presentation_id, item_order)`.

- [ ] **Step 1: Write the migration with its tests** — `crates/presenter-migration/src/m20261014_000003_bible_sermons.rs`:

```rust
//! #851 P2: sermons (spec §7.1). A sermon IS a Bible presentation plus one
//! `bible_sermons` row (service date, translations, author label, revision)
//! and its source items. Additive and idempotent (`IF NOT EXISTS`): the
//! `bible_*` tables hold production data, so this is a NEW migration, never an
//! edit of an applied one. Raw SQL like `m20260410_000001_separate_bible`, so
//! the schema reads exactly as the spec writes it.
//!
//! Deliberately NO foreign key from `bible_sermon_items.translation_code` to
//! `bible_translations` (spec §5.7): every Bible re-import deletes the
//! translation row and re-inserts it, and a cascading key would delete the
//! items with it. Items keep their own text snapshot instead.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_SERMONS: &str = r#"CREATE TABLE IF NOT EXISTS "bible_sermons" (
    "presentation_id" varchar(36) NOT NULL PRIMARY KEY,
    "service_date" text NOT NULL,
    "default_translation_code" text NOT NULL,
    "secondary_translation_code" text NULL,
    "author_email" text NULL,
    "revision" integer NOT NULL DEFAULT 0,
    "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY ("presentation_id") REFERENCES "bible_presentations"("id")
        ON DELETE CASCADE ON UPDATE CASCADE
)"#;

const CREATE_SERMON_DATE_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_sermons_service_date" ON "bible_sermons" ("service_date")"#;

const CREATE_ITEMS: &str = r#"CREATE TABLE IF NOT EXISTS "bible_sermon_items" (
    "id" varchar(36) NOT NULL PRIMARY KEY,
    "presentation_id" varchar(36) NOT NULL,
    "item_order" integer NOT NULL,
    "translation_code" text NOT NULL,
    "book_code" text NOT NULL,
    "book_number" integer NOT NULL,
    "book_name" text NOT NULL,
    "chapter" integer NOT NULL,
    "verse_start" integer NOT NULL,
    "verse_end" integer NOT NULL,
    "source_json" text NOT NULL,
    "edits_json" text NOT NULL,
    "slide_count" integer NOT NULL DEFAULT 0,
    "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY ("presentation_id") REFERENCES "bible_sermons"("presentation_id")
        ON DELETE CASCADE
)"#;

const CREATE_ITEM_ORDER_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_sermon_items_order" ON "bible_sermon_items" ("presentation_id", "item_order")"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            CREATE_SERMONS,
            CREATE_SERMON_DATE_INDEX,
            CREATE_ITEMS,
            CREATE_ITEM_ORDER_INDEX,
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Brand-new tables in this migration; a rollback removes them (items
        // first, they reference the sermons).
        let db = manager.get_connection();
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_sermon_items""#)
            .await?;
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_sermons""#)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DbBackend, Statement};

    const SELECT_ALL_COLUMNS: [&str; 2] = [
        "SELECT presentation_id, service_date, default_translation_code, \
         secondary_translation_code, author_email, revision, created_at, updated_at \
         FROM bible_sermons",
        "SELECT id, presentation_id, item_order, translation_code, book_code, book_number, \
         book_name, chapter, verse_start, verse_end, source_json, edits_json, slide_count, \
         created_at, updated_at FROM bible_sermon_items",
    ];

    async fn one_connection_db() -> sea_orm::DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1).min_connections(1);
        Database::connect(options).await.expect("connect")
    }

    async fn query(db: &sea_orm::DatabaseConnection, sql: &str) -> Result<(), DbErr> {
        db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn up_creates_both_tables_with_every_column_and_is_idempotent() {
        let db = one_connection_db().await;
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_err(), "precondition: {sql}");
        }
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");
        Migration.up(&manager).await.expect("second up is a no-op");
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_ok(), "after up: {sql}");
        }
    }

    #[tokio::test]
    async fn down_removes_both_tables() {
        let db = one_connection_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("up");
        Migration.down(&manager).await.expect("down");
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_err(), "after down: {sql}");
        }
    }
}
```

- [ ] **Step 2: Register it** in `crates/presenter-migration/src/lib.rs` (module line + last `Box::new(...)` entry, see Files). Migration files are exempt from the size gate (`*/presenter-migration/src/*.rs`).

- [ ] **Step 3: How it is verified.** CI: the two migration tests (single-connection in-memory DB — `sqlite::memory:` without `cache=shared` is one database per connection) and every existing persistence test, which runs the full `Migrator::up` through `Repository::connect_in_memory`. sqlx-sqlite 0.8.6 sets `foreign_keys = ON` on every pooled connection by default (`SqliteConnectOptions::new`), so the cascades of Task 12's tests are real.

- [ ] **Step 4: Commit** — paths `crates/presenter-migration/src/m20261014_000003_bible_sermons.rs crates/presenter-migration/src/lib.rs`, message `feat(migration): bible_sermons and bible_sermon_items tables (#851 P2a)` + body "No FK from items to bible_translations: a re-import deletes and re-inserts the translation row (spec §5.7)." + Co-Authored-By.

### Task 12: Sermon entities, create/fetch, and `sermon_date` on presentations

**Files:**
- Create: `crates/presenter-persistence/src/entities/sermon.rs`
- Create: `crates/presenter-persistence/src/repository/bible/sermons.rs`
- Create: `crates/presenter-persistence/src/repository/bible/sermons/tests.rs` (exempt: `**/tests/*.rs`)
- Modify: `crates/presenter-persistence/src/entities.rs:479` (after the `bible_slide` module: declare + re-export the sermon entities)
- Modify: `crates/presenter-persistence/src/repository/bible/mod.rs:13` (`mod sermons;` + re-export), `repository/mod.rs:62` (re-export), `lib.rs:30-34` (public re-export)
- Modify: `crates/presenter-core/src/bible/presentation.rs` (`sermon_date` on `BiblePresentation` and `BiblePresentationSummary`, the 1 test literal)
- Modify: `crates/presenter-persistence/src/repository/bible/presentations.rs:19-63` (list), `:65-97` (fetch), `:115-120` (create) — the 3 production literals

**Interfaces:**
- Consumes: `ItemEdits`, `SourceVerse` (P1); `parse_uuid`, `RepositoryError` (`repository/util.rs`).
- Produces (`presenter_persistence::…`): `struct NewSermon { title, service_date: NaiveDate, default_translation_code, secondary_translation_code: Option<String>, author_email: Option<String> }`; `struct SermonRecord { id: BiblePresentationId, title, service_date, default_translation_code, secondary_translation_code, author_email, revision: i64, created_at: DateTime<Utc>, updated_at: DateTime<Utc> }`; `struct SermonItemRecord { id: Uuid, translation_code, book_code, book_number: u16, book_name, chapter: u16, verse_start: u16, verse_end: u16, source: Vec<SourceVerse>, edits: ItemEdits, slide_count: u32, created_at, updated_at }` (order = position in the list); `struct SermonSnapshot { sermon: SermonRecord, items: Vec<SermonItemRecord> }`; `const SERMON_NOT_FOUND: &str = "sermon not found"`; `Repository::create_sermon(&NewSermon) -> anyhow::Result<SermonRecord>`, `fetch_sermon(BiblePresentationId) -> anyhow::Result<Option<SermonSnapshot>>`, `is_sermon(BiblePresentationId) -> anyhow::Result<bool>`; `pub(super) sermon_dates() -> HashMap<String, NaiveDate>`, `pub(super) sermon_date_of(&str) -> Option<NaiveDate>`; `BiblePresentation::sermon_date` / `BiblePresentationSummary::sermon_date: Option<NaiveDate>`; entities `bible_sermon`, `bible_sermon_item`.

- [ ] **Step 1: Write the tests** — `crates/presenter-persistence/src/repository/bible/sermons/tests.rs`:

```rust
//! #851 P2a/P2b: sermon persistence (spec §7.1, §7.5, §5.7 / T6).

use super::*;

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

fn new_sermon(title: &str, service_date: &str) -> NewSermon {
    NewSermon {
        title: title.to_string(),
        service_date: date(service_date),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: Some("eng-kjv".to_string()),
        author_email: Some("preacher@example.org".to_string()),
    }
}

#[tokio::test]
async fn a_new_sermon_is_a_presentation_with_a_date() {
    let repo = repo().await;
    let created = repo
        .create_sermon(&new_sermon("Kázeň 18. 10. 2026", "2026-10-18"))
        .await
        .expect("create");

    let snapshot = repo
        .fetch_sermon(created.id)
        .await
        .expect("fetch")
        .expect("sermon exists");
    assert_eq!(snapshot.sermon.title, "Kázeň 18. 10. 2026");
    assert_eq!(snapshot.sermon.service_date, date("2026-10-18"));
    assert_eq!(snapshot.sermon.revision, 0);
    assert_eq!(
        snapshot.sermon.author_email.as_deref(),
        Some("preacher@example.org")
    );
    assert!(snapshot.items.is_empty());
    assert!(repo.is_sermon(created.id).await.expect("is_sermon"));

    let plain = repo
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain");
    assert!(!repo.is_sermon(plain.id).await.expect("is_sermon"));
    let summaries = repo
        .list_bible_presentation_summaries()
        .await
        .expect("list");
    let dates: Vec<(String, Option<NaiveDate>)> = summaries
        .into_iter()
        .map(|summary| (summary.name, summary.sermon_date))
        .collect();
    assert_eq!(
        dates,
        [
            ("Ad hoc".to_string(), None),
            ("Kázeň 18. 10. 2026".to_string(), Some(date("2026-10-18"))),
        ]
    );
    let detail = repo
        .fetch_bible_presentation(created.id)
        .await
        .expect("fetch presentation")
        .expect("presentation exists");
    assert_eq!(detail.sermon_date, Some(date("2026-10-18")));
}

#[tokio::test]
async fn deleting_the_presentation_deletes_its_sermon() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");

    repo.delete_bible_presentation(sermon.id)
        .await
        .expect("delete");

    assert!(repo.fetch_sermon(sermon.id).await.expect("fetch").is_none());
    assert!(!repo.is_sermon(sermon.id).await.expect("is_sermon"));
}
```

- [ ] **Step 2: The entities** — `crates/presenter-persistence/src/entities/sermon.rs`:

```rust
//! #851 P2 sermon entities (spec §7.1), in their own file because
//! `entities.rs` sits at 927 production lines (cap 1000). Re-exported from
//! `entities` as `bible_sermon` / `bible_sermon_item`. Every string primary key
//! is `auto_increment = false` and is written with `Entity::insert(..).exec`
//! (database skill #658: `ActiveModel::insert` cannot read a String key back).

pub mod bible_sermon {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_sermons")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub presentation_id: String,
        /// `YYYY-MM-DD`, the server-local date of the service.
        pub service_date: String,
        pub default_translation_code: String,
        pub secondary_translation_code: Option<String>,
        pub author_email: Option<String>,
        pub revision: i64,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_sermon_item {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_sermon_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub item_order: i32,
        /// No foreign key on purpose (spec §5.7).
        pub translation_code: String,
        pub book_code: String,
        pub book_number: i32,
        pub book_name: String,
        pub chapter: i32,
        pub verse_start: i32,
        pub verse_end: i32,
        pub source_json: String,
        pub edits_json: String,
        pub slide_count: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
```

`crates/presenter-persistence/src/entities.rs` — after the closing `}` of `pub mod bible_slide` (line 479), before `pub use app_settings::Entity as AppSettingsEntity;`:

```rust

mod sermon;
pub use sermon::{bible_sermon, bible_sermon_item};
```

- [ ] **Step 3: The repository** — `crates/presenter-persistence/src/repository/bible/sermons.rs`:

```rust
//! Sermons (#851 spec §7.1, §7.5). A sermon IS a Bible presentation (its slides
//! are what the operator, the tablet and the walls already play) plus a
//! `bible_sermons` row and its source items. The server generates the slides
//! from the items; `save_sermon_change` (P2b) stores items and slides in ONE
//! transaction guarded by the sermon's `revision`, so a stale write from a
//! second device is a 409 and never half-applied.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use tracing::instrument;
use uuid::Uuid;

use crate::entities::{bible_presentation, bible_sermon, bible_sermon_item};
use crate::repository::util::{parse_uuid, RepositoryError};
use crate::repository::Repository;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::BiblePresentationId;

pub const SERMON_NOT_FOUND: &str = "sermon not found";
const DATE_FORMAT: &str = "%Y-%m-%d";

/// A sermon to create: the presentation's name is `title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSermon {
    pub title: String,
    pub service_date: NaiveDate,
    pub default_translation_code: String,
    pub secondary_translation_code: Option<String>,
    pub author_email: Option<String>,
}

/// The `bible_sermons` row with its presentation's name as the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonRecord {
    pub id: BiblePresentationId,
    pub title: String,
    pub service_date: NaiveDate,
    pub default_translation_code: String,
    pub secondary_translation_code: Option<String>,
    pub author_email: Option<String>,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One source item; its order is its position in the sermon's item list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonItemRecord {
    pub id: Uuid,
    pub translation_code: String,
    pub book_code: String,
    pub book_number: u16,
    pub book_name: String,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
    pub source: Vec<SourceVerse>,
    pub edits: ItemEdits,
    pub slide_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A sermon with its items in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonSnapshot {
    pub sermon: SermonRecord,
    pub items: Vec<SermonItemRecord>,
}

impl Repository {
    /// Create the presentation and its sermon row (revision 0) in one
    /// transaction.
    #[instrument(skip_all)]
    pub async fn create_sermon(&self, new: &NewSermon) -> anyhow::Result<SermonRecord> {
        let id = BiblePresentationId::new();
        let now = Utc::now();
        let txn = self.db.begin().await?;
        bible_presentation::Entity::insert(bible_presentation::ActiveModel {
            id: Set(id.to_string()),
            name: Set(new.title.clone()),
            created_at: Set(now.into()),
        })
        .exec(&txn)
        .await?;
        bible_sermon::Entity::insert(bible_sermon::ActiveModel {
            presentation_id: Set(id.to_string()),
            service_date: Set(new.service_date.format(DATE_FORMAT).to_string()),
            default_translation_code: Set(new.default_translation_code.clone()),
            secondary_translation_code: Set(new.secondary_translation_code.clone()),
            author_email: Set(new.author_email.clone()),
            revision: Set(0),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        })
        .exec(&txn)
        .await?;
        txn.commit().await?;
        Ok(SermonRecord {
            id,
            title: new.title.clone(),
            service_date: new.service_date,
            default_translation_code: new.default_translation_code.clone(),
            secondary_translation_code: new.secondary_translation_code.clone(),
            author_email: new.author_email.clone(),
            revision: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// The sermon with its items in order; `None` when `id` is no sermon.
    #[instrument(skip_all)]
    pub async fn fetch_sermon(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<SermonSnapshot>> {
        let id_str = id.to_string();
        let Some(row) = bible_sermon::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let title = bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
            .map(|presentation| presentation.name)
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        let items = bible_sermon_item::Entity::find()
            .filter(bible_sermon_item::Column::PresentationId.eq(id_str))
            .order_by_asc(bible_sermon_item::Column::ItemOrder)
            .all(&self.db)
            .await?
            .into_iter()
            .map(item_from_model)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(SermonSnapshot {
            sermon: sermon_from_model(row, title)?,
            items,
        }))
    }

    /// Whether the presentation is a sermon (its slides are generated).
    #[instrument(skip_all)]
    pub async fn is_sermon(&self, id: BiblePresentationId) -> anyhow::Result<bool> {
        Ok(bible_sermon::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .is_some())
    }

    /// The service date of every sermon, by presentation id (the
    /// `sermon_date` of presentation summaries and details).
    pub(super) async fn sermon_dates(&self) -> anyhow::Result<HashMap<String, NaiveDate>> {
        let mut dates = HashMap::new();
        for row in bible_sermon::Entity::find().all(&self.db).await? {
            let date = parse_date(&row.service_date)?;
            dates.insert(row.presentation_id, date);
        }
        Ok(dates)
    }

    /// The service date of one presentation; `None` for a plain presentation.
    pub(super) async fn sermon_date_of(
        &self,
        presentation_id: &str,
    ) -> anyhow::Result<Option<NaiveDate>> {
        bible_sermon::Entity::find_by_id(presentation_id.to_string())
            .one(&self.db)
            .await?
            .map(|row| parse_date(&row.service_date))
            .transpose()
    }
}

fn parse_date(raw: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, DATE_FORMAT)
        .with_context(|| format!("invalid sermon service_date '{raw}' in the database"))
}

fn sermon_from_model(model: bible_sermon::Model, title: String) -> anyhow::Result<SermonRecord> {
    Ok(SermonRecord {
        id: BiblePresentationId::from_uuid(parse_uuid(&model.presentation_id)?),
        title,
        service_date: parse_date(&model.service_date)?,
        default_translation_code: model.default_translation_code,
        secondary_translation_code: model.secondary_translation_code,
        author_email: model.author_email,
        revision: model.revision,
        created_at: model.created_at.with_timezone(&Utc),
        updated_at: model.updated_at.with_timezone(&Utc),
    })
}

fn stored_u16(value: i32, column: &str) -> anyhow::Result<u16> {
    u16::try_from(value).with_context(|| format!("sermon item {column} {value} out of range"))
}

fn item_from_model(model: bible_sermon_item::Model) -> anyhow::Result<SermonItemRecord> {
    Ok(SermonItemRecord {
        id: parse_uuid(&model.id)?,
        translation_code: model.translation_code,
        book_code: model.book_code,
        book_number: stored_u16(model.book_number, "book_number")?,
        book_name: model.book_name,
        chapter: stored_u16(model.chapter, "chapter")?,
        verse_start: stored_u16(model.verse_start, "verse_start")?,
        verse_end: stored_u16(model.verse_end, "verse_end")?,
        source: serde_json::from_str(&model.source_json).context("sermon item source_json")?,
        edits: serde_json::from_str(&model.edits_json).context("sermon item edits_json")?,
        slide_count: u32::try_from(model.slide_count).context("sermon item slide_count")?,
        created_at: model.created_at.with_timezone(&Utc),
        updated_at: model.updated_at.with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests;
```

Exports: `crates/presenter-persistence/src/repository/bible/mod.rs` — after `mod query;` add `mod sermons;`, and after the module list:

```rust
pub use sermons::{NewSermon, SermonItemRecord, SermonRecord, SermonSnapshot, SERMON_NOT_FOUND};
```

`crates/presenter-persistence/src/repository/mod.rs` — before `pub use library::TrashedLibrary;`:

```rust
pub use bible::{NewSermon, SermonItemRecord, SermonRecord, SermonSnapshot, SERMON_NOT_FOUND};
```

`crates/presenter-persistence/src/lib.rs` — the `pub use repository::{…}` block becomes:

```rust
pub use repository::{
    sync_should_apply, DatabaseSettings, NewSermon, NewStreamAsset, NewStreamFont, Repository,
    RepositoryError, SermonItemRecord, SermonRecord, SermonSnapshot, SyncApplyOutcome,
    SyncLibraryManifestRow, SyncManifestRow, SyncPresentation, TrashedLibrary, TrashedPresentation,
    PRUNE_HORIZON, SERMON_NOT_FOUND,
};
```

- [ ] **Step 4: `sermon_date` on the presentation types.** First `grep -rn 'BiblePresentation {\|BiblePresentationSummary {' crates tests` — expected exactly 4 literals (core test `presentation.rs:69`, persistence `presentations.rs:57, 90, 115`). `crates/presenter-core/src/bible/presentation.rs`: `use chrono::{DateTime, Utc};` → `use chrono::{DateTime, NaiveDate, Utc};`; append to `BiblePresentation` after `created_at` and to `BiblePresentationSummary` after `slide_count`:

```rust
    /// The service date when the presentation is a sermon (#851 §7.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sermon_date: Option<NaiveDate>,
```

and in the test `bible_presentation_serialization_uses_camel_case` add `sermon_date: None,` after `created_at: …`. `crates/presenter-persistence/src/repository/bible/presentations.rs`: in `list_bible_presentation_summaries`, after the `let counts: … collect();` statement insert `let sermon_dates = self.sermon_dates().await?;` and add `sermon_date: sermon_dates.get(&model.id).copied(),` after `slide_count,` in the pushed literal (a different field than the moved `model.name` — partial moves are fine); in `fetch_bible_presentation`, right after the `let Some(model) = … else { return Ok(None); };` statement insert `let sermon_date = self.sermon_date_of(&id_str).await?;` (before `id_str` moves into the slide filter) and add `sermon_date,` after `created_at: model.created_at.into(),`; in `create_bible_presentation` add `sermon_date: None,` after `created_at: now,`.

- [ ] **Step 5: How it is verified.** CI: Step 1's two tests; every existing presentation test still compiles and passes with the new field (the 4 literals updated); the AI tools serialize `BiblePresentationSummary` unchanged for plain presentations (`skip_serializing_if`).

- [ ] **Step 6: Commit** — paths `crates/presenter-persistence/src/entities.rs crates/presenter-persistence/src/entities/sermon.rs crates/presenter-persistence/src/repository/bible/mod.rs crates/presenter-persistence/src/repository/bible/sermons.rs crates/presenter-persistence/src/repository/bible/sermons/tests.rs crates/presenter-persistence/src/repository/mod.rs crates/presenter-persistence/src/lib.rs crates/presenter-persistence/src/repository/bible/presentations.rs crates/presenter-core/src/bible/presentation.rs`, message `feat(persistence): create and read sermons, sermon_date on presentations (#851 P2a)` + Co-Authored-By.

### Task 13: The composer takes caller labels (`compose_bible_slides_labelled`)

**Files:**
- Modify: `crates/presenter-server/src/state/slides/compose.rs:5-13` (import the core short code, delete the local copy) and `:424-490` (`compose_bible_slides` becomes a wrapper over the new labelled function)
- Modify: `crates/presenter-server/src/state/slides/tests.rs:1` (one import line) + append two tests
- Modify: `.claude/rules/bible-slide-composer.md` (append a section)

**Interfaces:**
- Consumes: `presenter_core::bible::sermon::translation_short_code` (P1).
- Produces: `pub(crate) struct SlideLabels { pub(crate) main: String, pub(crate) translation: Option<String> }` and `pub(crate) fn compose_bible_slides_labelled(main_translation: &BibleTranslation, secondary_translation: Option<&BibleTranslation>, main_passages: &[BiblePassage], secondary_lookup: &HashMap<u16, BiblePassage>, character_limit: u32, labels: SlideLabels) -> anyhow::Result<Vec<Slide>>` in `state::slides::compose`. Not re-exported from `state/slides.rs` yet (an unused `pub(crate) use` fails `-D warnings`); P3 adds the re-export together with its first caller.

- [ ] **Step 1: Append the tests** to `crates/presenter-server/src/state/slides/tests.rs`, and add `use super::compose::{compose_bible_slides_labelled, SlideLabels};` below its first `use` line (`compose` is a private module of `slides`; `tests` is its child, so the path resolves):

```rust

fn bible(slide: &presenter_core::Slide) -> &presenter_core::slide::BibleSlideMetadata {
    slide
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.bible.as_ref())
        .expect("bible metadata")
}

#[test]
fn labelled_composer_puts_the_given_labels_on_every_slide() {
    let seb = test_translation("slk-seb");
    let kjv = test_translation("eng-kjv");
    let passages = vec![
        test_passage("Hebrejom", 11, 1, "Viera je zárukou toho, v čo dúfame"),
        test_passage("Hebrejom", 11, 3, "Vierou poznávame, že svety vznikli slovom."),
    ];
    let labels = SlideLabels {
        main: "Hebrejom 11:1a, 3 (SEB)".to_string(),
        translation: Some("Hebrews 11:1, 3 (KJV)".to_string()),
    };

    let slides =
        compose_bible_slides_labelled(&seb, Some(&kjv), &passages, &HashMap::new(), 40, labels)
            .expect("compose");

    assert_eq!(slides.len(), 2, "40 chars hold one verse per slide");
    for slide in &slides {
        assert_eq!(slide.content.stage.value(), "Hebrejom 11:1a, 3 (SEB)");
        let meta = bible(slide);
        assert_eq!(meta.main_reference_label.as_deref(), Some("Hebrejom 11:1a, 3 (SEB)"));
        assert_eq!(
            meta.translation_reference_label.as_deref(),
            Some("Hebrews 11:1, 3 (KJV)")
        );
        assert_eq!(meta.secondary_translation_code.as_deref(), Some("eng-kjv"));
    }
    assert_eq!(slides[0].content.main.value(), "1. Viera je zárukou toho, v čo dúfame");
    assert_eq!(bible(&slides[1]).verses[0].start, 3);
}

#[test]
fn labelled_composer_splits_by_the_longer_text_and_keeps_a_lone_verse_whole() {
    let seb = test_translation("slk-seb");
    let kjv = test_translation("eng-kjv");
    let passages = vec![
        test_passage("Hebrejom", 11, 2, "Krátky verš."),
        test_passage("Hebrejom", 11, 3, "Ďalší krátky."),
    ];
    let mut lookup = HashMap::new();
    for (verse, text) in [
        (2, "For by it the elders obtained a good report."),
        (3, "Through faith we understand the worlds were framed."),
    ] {
        lookup.insert(verse, test_passage("Hebrews", 11, verse, text));
    }
    let labels = || SlideLabels {
        main: "Hebrejom 11:2-3 (SEB)".to_string(),
        translation: Some("Hebrews 11:2-3 (KJV)".to_string()),
    };

    let split =
        compose_bible_slides_labelled(&seb, Some(&kjv), &passages, &lookup, 60, labels())
            .expect("compose");
    let lone = compose_bible_slides_labelled(&seb, None, &passages[..1], &HashMap::new(), 5, labels())
        .expect("compose");

    // The SEB lines fit 60 chars together, the KJV lines (102) do not.
    assert_eq!(split.len(), 2);
    assert_eq!(lone.len(), 1);
    assert_eq!(lone[0].content.main.value(), "2. Krátky verš.");
}
```

Lengths checked: "1. Viera je zárukou toho, v čo dúfame" = 37 chars and "3. Vierou poznávame, že svety vznikli slovom." = 45 (> 40 together); the SEB pair = 32 ≤ 60 while the KJV pair = 47 + 1 + 54 = 102 > 60.

- [ ] **Step 2: Implement.** In `compose.rs` replace

```rust
use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
```

with

```rust
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
```

delete the local `translation_short_code` (lines 9-13 with its doc comment and the blank line after it — same body, now in core), and replace everything from `pub(crate) fn compose_bible_slides(` to the end of the file with:

```rust
/// The labels every slide of one composed passage carries (#851 P3): the live
/// path builds them from the verse range, a sermon item passes its computed
/// `16a`-style label and the secondary book's plain-number label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlideLabels {
    pub(crate) main: String,
    pub(crate) translation: Option<String>,
}

pub(crate) fn compose_bible_slides(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[BiblePassage],
    secondary_lookup: &HashMap<u16, BiblePassage>,
    character_limit: u32,
    full_verse_start: u16,
    full_verse_end: u16,
) -> anyhow::Result<Vec<Slide>> {
    let Some(first) = main_passages.first() else {
        return Ok(Vec::new());
    };
    let chapter = first.reference.chapter;

    // The full reference label appears on all slides (includes translation
    // code). A secondary translation gets its own label.
    let labels = SlideLabels {
        main: build_reference_label(
            &first.reference.book,
            chapter,
            full_verse_start,
            full_verse_end,
            &main_translation.code,
        ),
        translation: secondary_translation.and_then(|t| {
            let secondary_book = secondary_book_name(main_passages, secondary_lookup)?;
            Some(build_reference_label(
                secondary_book,
                chapter,
                full_verse_start,
                full_verse_end,
                &t.code,
            ))
        }),
    };
    compose_bible_slides_labelled(
        main_translation,
        secondary_translation,
        main_passages,
        secondary_lookup,
        character_limit,
        labels,
    )
}

/// The live composer with the caller's labels (#851 P3): one line
/// `"{verse}. {text}"` per passage, a new slide when the LONGER of the main and
/// the translation text would pass `character_limit` characters (#828), a lone
/// oversized verse kept whole (#434).
pub(crate) fn compose_bible_slides_labelled(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[BiblePassage],
    secondary_lookup: &HashMap<u16, BiblePassage>,
    character_limit: u32,
    labels: SlideLabels,
) -> anyhow::Result<Vec<Slide>> {
    let Some(first) = main_passages.first() else {
        return Ok(Vec::new());
    };
    let frame = BibleSlideFrame {
        main_translation,
        secondary_translation,
        book: first.reference.book.clone(),
        book_code: first.reference.book_code.clone(),
        book_number: first.reference.book_number,
        chapter: first.reference.chapter,
        reference_label: labels.main,
        translation_reference_label: labels.translation,
    };

    let limit = character_limit as usize;
    let mut slides = Vec::new();
    let mut draft = SlideDraft::default();
    for passage in main_passages {
        let verse = passage.reference.verse_start;
        let main_line = format!("{verse}. {}", passage.text);
        let translation_line = secondary_lookup
            .get(&verse)
            .map(|sec| format!("{}. {}", sec.reference.verse_start, sec.text));
        // A lone verse longer than the limit stays one whole slide (#434).
        if !draft.is_empty() && draft.would_exceed(&main_line, translation_line.as_deref(), limit) {
            frame.push_slide(&mut slides, std::mem::take(&mut draft))?;
        }
        draft.push(
            &main_line,
            translation_line.as_deref(),
            (verse, passage.reference.verse_end),
        );
    }
    frame.push_slide(&mut slides, draft)?;

    Ok(slides)
}
```

- [ ] **Step 3: The rule** — append to `.claude/rules/bible-slide-composer.md`:

```markdown

## Caller labels: `compose_bible_slides_labelled` (#851 P2a)

`compose_bible_slides` is a thin wrapper: it builds today's integer-range labels and calls
`compose_bible_slides_labelled(main, secondary, passages, lookup, limit, SlideLabels { main,
translation })`, which holds the whole #828 loop. A sermon item passes its computed label
("Hebrejom 11:1a (SEB)") and the secondary book's plain-number label; the line format stays
`"{verse}. {text}"` (the integer verse prefixes the line, the label carries the letter). A new
caller with its own labels calls the labelled form; never fork the loop. "(SEB)" comes from
`presenter_core::bible::sermon::translation_short_code` (one copy for the composer and the
sermon labels).
```

- [ ] **Step 4: How it is verified.** CI: every existing composer test in `state/slides/tests.rs` passes unchanged (the wrapper builds the same labels), plus the two new ones; `compose_bible_slides` shrinks to ≈ 45 lines, the labelled form ≈ 45.

- [ ] **Step 5: Commit** — paths `crates/presenter-server/src/state/slides/compose.rs crates/presenter-server/src/state/slides/tests.rs .claude/rules/bible-slide-composer.md`, message `refactor(server): the Bible composer takes caller labels (#851 P2a)` + body "No behaviour change: compose_bible_slides builds today's labels and calls compose_bible_slides_labelled; the short code comes from presenter-core." + Co-Authored-By.

### Task 14: Rule section, ship P2a, check the schema on SNV and PP

**Files:**
- Modify: `.claude/rules/sermon-prep.md` (append)

- [ ] **Step 1: Append** to `.claude/rules/sermon-prep.md`:

```markdown

## Persistence (P2a, P2b)

- **A sermon IS a Bible presentation**: `bible_sermons.presentation_id` → `bible_presentations(id)`
  ON DELETE CASCADE; the title is the presentation's name; deleting the presentation (Prepared
  tab, AI tool, `/ui/study`) removes the sermon and its items.
- **Deliberately NO foreign key from `bible_sermon_items.translation_code`** (spec §5.7): every
  Bible re-import deletes the translation row and re-inserts it, and a cascading key would
  delete the items. Items keep a text snapshot (`source_json`). The re-import test
  `a_reimport_leaves_slides_sermons_and_items_byte_identical` fails if a key sneaks in.
- Migration `m20261014_000003_bible_sermons.rs` is raw SQL (`IF NOT EXISTS`), like
  `m20260410_000001_separate_bible`, so it reads like the spec; never edit it after it ran on
  SNV/PP — the next change is a new migration.
- Entities live in `entities/sermon.rs` (re-exported from `entities`: `bible_sermon`,
  `bible_sermon_item`) because `entities.rs` sits at 927 production lines. String keys are
  `auto_increment = false` and written with `Entity::insert(..).exec` (database skill #658).
- `BiblePresentationSummary` / `BiblePresentation` carry `sermon_date: Option<NaiveDate>`
  (`skip_serializing_if = "Option::is_none"`): filled by `sermon_dates()` / `sermon_date_of()`.
  A new literal of either struct must name the field (E0063 is CI-only).
```

Commit — `docs(playbook): sermon persistence rules (#851 P2a) [no-test: docs only]`.

- [ ] **Step 2: Local gates** with `<files>` = `crates/presenter-migration/src/m20261014_000003_bible_sermons.rs crates/presenter-migration/src/lib.rs crates/presenter-persistence/src/entities.rs crates/presenter-persistence/src/entities/sermon.rs crates/presenter-persistence/src/repository/bible/sermons.rs crates/presenter-persistence/src/repository/bible/presentations.rs crates/presenter-persistence/src/repository/bible/mod.rs crates/presenter-persistence/src/repository/mod.rs crates/presenter-persistence/src/lib.rs crates/presenter-core/src/bible/presentation.rs crates/presenter-server/src/state/slides/compose.rs`. Expected: `entities.rs` 929 production lines (< 1000), `compose.rs` ≈ 500.

- [ ] **Step 3: Ship** with `<V>` = 0.4.320, `<title>` = "Sermon schema and reads; composer caller labels (#851 P2a)".

- [ ] **Step 4: Functional check, SNV then PP** (read-only):

```bash
ssh -o BatchMode=yes newlevel@presenter.lan "sqlite3 -readonly /opt/presenter/presenter.db \
  \"SELECT version FROM seaql_migrations WHERE version LIKE 'm20261014%'; \
    SELECT name FROM sqlite_master WHERE name LIKE '%sermon%' ORDER BY name; \
    SELECT COUNT(*) FROM bible_sermons;\""
curl -fsS http://10.77.9.205/bible/presentations | head -c 300
```

Expected: `m20261014_000003_bible_sermons`; `bible_sermon_items`, `bible_sermons`, `idx_bible_sermon_items_order`, `idx_bible_sermons_service_date`; `0`; the presentation list unchanged (no `sermonDate` keys — the server DTOs gain it in P4b). Same on `companion-pp.lan` after the release.

- [ ] **Step 5:** `playbook-review`; comment on #851.

---

## PR unit P2b — the revision-guarded sermon write, summaries, re-import safety (v0.4.321)

Spec §7.5 (one transaction, revision → 409), §11 list data, §5.7/T6 (re-import safety). All new items are `pub` methods of the exported `Repository` (no dead-code risk before their P3/P4 callers).

### Task 15: Post-release version bump to 0.4.321

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.320)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.320 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.321 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.321 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 16: `save_sermon_change` — items + slides in ONE revision-guarded transaction

**Files:**
- Modify: `crates/presenter-persistence/src/repository/bible/sermons.rs` (head, two structs, the write, three helpers)
- Modify: `crates/presenter-persistence/src/repository/bible/presentations.rs:258` (`bible_slide_to_active_model` becomes `pub(super)`)
- Modify: `crates/presenter-persistence/src/repository/bible/sermons/tests.rs` (imports + tests)
- Modify: exports in `repository/bible/mod.rs`, `repository/mod.rs`, `lib.rs`

**Interfaces:**
- Consumes: Task 12's types and helpers (`item_from_model`, `parse_date`, …); `bible_slide_to_active_model(&BiblePresentationSlide, &str) -> anyhow::Result<bible_slide::ActiveModel>`.
- Produces: `const SERMON_CONFLICT: &str = "sermon changed on another device"`; `struct SermonWrite { expected_revision: i64, title: String, service_date: NaiveDate, secondary_translation_code: Option<String>, items: Vec<SermonItemRecord>, slides: Vec<BiblePresentationSlide> }`; `Repository::save_sermon_change(id: BiblePresentationId, write: &SermonWrite) -> anyhow::Result<i64>` (the new revision; `Conflict(SERMON_CONFLICT)` when stale, `NotFound(SERMON_NOT_FOUND)` when gone).

- [ ] **Step 1: Write the tests.** In `sermons/tests.rs` replace the line `use super::*;` with

```rust
use super::*;
use presenter_core::bible::sermon::VerseEdits;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleSlideId, BibleTranslation, SlideText};
```

and append:

```rust

// ---- #851 P2b: sermon writes ------------------------------------------------

fn item(verse: u16, text: &str, slide_count: u32) -> SermonItemRecord {
    let mut edits = ItemEdits::default();
    edits.0.insert(
        verse,
        VerseEdits {
            deleted: vec![1],
            emphasized: vec![0],
        },
    );
    SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: verse,
        verse_end: verse,
        source: vec![SourceVerse {
            verse,
            text: text.to_string(),
        }],
        edits,
        slide_count,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn slide(text: &str) -> BiblePresentationSlide {
    BiblePresentationSlide {
        id: BibleSlideId::new(),
        order: 0,
        main: SlideText::new(text).expect("slide text"),
        main_reference: "Hebrejom 11:1a (SEB)".to_string(),
        secondary: SlideText::new("").expect("slide text"),
        secondary_reference: String::new(),
        metadata: None,
    }
}

fn write(
    expected_revision: i64,
    title: &str,
    items: Vec<SermonItemRecord>,
    slides: Vec<BiblePresentationSlide>,
) -> SermonWrite {
    SermonWrite {
        expected_revision,
        title: title.to_string(),
        service_date: date("2026-10-18"),
        secondary_translation_code: None,
        items,
        slides,
    }
}

fn refusal(err: &anyhow::Error) -> Option<&RepositoryError> {
    err.downcast_ref::<RepositoryError>()
}

#[tokio::test]
async fn a_write_replaces_items_and_slides_and_bumps_the_revision() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    let items = vec![item(1, "Viera je zárukou", 2), item(3, "Vierou chápeme", 1)];
    let slides = vec![
        slide("1. VIERA zárukou"),
        slide("1. pokračovanie"),
        slide("3. Vierou"),
    ];

    let revision = repo
        .save_sermon_change(sermon.id, &write(0, "Viera", items.clone(), slides.clone()))
        .await
        .expect("save");

    assert_eq!(revision, 1);
    let snapshot = repo
        .fetch_sermon(sermon.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert_eq!(snapshot.sermon.revision, 1);
    assert_eq!(snapshot.sermon.title, "Viera");
    assert_eq!(snapshot.sermon.secondary_translation_code, None);
    let stored: Vec<(Uuid, Vec<SourceVerse>, ItemEdits, u32)> = snapshot
        .items
        .into_iter()
        .map(|item| (item.id, item.source, item.edits, item.slide_count))
        .collect();
    let expected: Vec<(Uuid, Vec<SourceVerse>, ItemEdits, u32)> = items
        .into_iter()
        .map(|item| (item.id, item.source, item.edits, item.slide_count))
        .collect();
    assert_eq!(stored, expected);
    let presentation = repo
        .fetch_bible_presentation(sermon.id)
        .await
        .expect("fetch presentation")
        .expect("exists");
    let stored_slides: Vec<(BibleSlideId, u32)> = presentation
        .slides
        .iter()
        .map(|slide| (slide.id, slide.order))
        .collect();
    let expected_slides: Vec<(BibleSlideId, u32)> = slides
        .iter()
        .zip(0..)
        .map(|(slide, order)| (slide.id, order))
        .collect();
    assert_eq!(stored_slides, expected_slides);
    assert_eq!(presentation.name, "Viera");
}

#[tokio::test]
async fn a_stale_revision_is_a_conflict_and_changes_nothing() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    let first = item(1, "Prvý zápis", 1);
    repo.save_sermon_change(
        sermon.id,
        &write(0, "Kázeň", vec![first.clone()], vec![slide("1. Prvý")]),
    )
    .await
    .expect("first write");

    let err = repo
        .save_sermon_change(
            sermon.id,
            &write(0, "Iný", vec![item(2, "Druhý", 1)], Vec::new()),
        )
        .await
        .expect_err("stale revision");

    assert!(matches!(
        refusal(&err),
        Some(RepositoryError::Conflict(SERMON_CONFLICT))
    ));
    let snapshot = repo
        .fetch_sermon(sermon.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert_eq!(snapshot.sermon.revision, 1);
    assert_eq!(snapshot.sermon.title, "Kázeň");
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [first.id]
    );
    let presentation = repo
        .fetch_bible_presentation(sermon.id)
        .await
        .expect("fetch presentation")
        .expect("exists");
    assert_eq!(presentation.slides.len(), 1);
}

#[tokio::test]
async fn writing_a_missing_sermon_is_not_found() {
    let repo = repo().await;
    let err = repo
        .save_sermon_change(
            BiblePresentationId::new(),
            &write(0, "Nič", Vec::new(), Vec::new()),
        )
        .await
        .expect_err("missing sermon");
    assert!(matches!(
        refusal(&err),
        Some(RepositoryError::NotFound(SERMON_NOT_FOUND))
    ));
}

#[tokio::test]
async fn deleting_the_presentation_cascades_to_the_sermon_and_its_items() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    repo.save_sermon_change(
        sermon.id,
        &write(0, "Kázeň", vec![item(1, "Text", 0)], Vec::new()),
    )
    .await
    .expect("write");

    repo.delete_bible_presentation(sermon.id)
        .await
        .expect("delete");

    assert!(repo.fetch_sermon(sermon.id).await.expect("fetch").is_none());
    let items = bible_sermon_item::Entity::find()
        .all(repo.connection())
        .await
        .expect("items");
    assert!(items.is_empty(), "{items:?}");
}
```

- [ ] **Step 2: Implement.** `sermons.rs`: replace everything above `pub const SERMON_NOT_FOUND` (the module doc and the imports) with

```rust
//! Sermons (#851 spec §7.1, §7.5). A sermon IS a Bible presentation (its slides
//! are what the operator, the tablet and the walls already play) plus a
//! `bible_sermons` row and its source items. The server generates the slides
//! from the items; [`Repository::save_sermon_change`] stores items and slides
//! in ONE transaction guarded by the sermon's `revision`, so a stale write from
//! a second device is a 409 and never half-applied.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use tracing::instrument;
use uuid::Uuid;

use super::presentations::bible_slide_to_active_model;
use crate::entities::{bible_presentation, bible_sermon, bible_sermon_item, bible_slide};
use crate::repository::util::{parse_uuid, RepositoryError};
use crate::repository::Repository;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::{BiblePresentationId, BiblePresentationSlide};

/// The 409 of every stale sermon write (spec §7.5).
pub const SERMON_CONFLICT: &str = "sermon changed on another device";
```

Insert after the `SermonSnapshot` struct:

```rust

/// The whole new state of one sermon write: meta, items in order (each with
/// its `slide_count`) and the generated slides.
#[derive(Debug, Clone, PartialEq)]
pub struct SermonWrite {
    pub expected_revision: i64,
    pub title: String,
    pub service_date: NaiveDate,
    pub secondary_translation_code: Option<String>,
    pub items: Vec<SermonItemRecord>,
    pub slides: Vec<BiblePresentationSlide>,
}
```

Append inside `impl Repository`, after `sermon_date_of`:

```rust

    /// Store one sermon write (spec §7.5) in ONE transaction: bump the revision
    /// only when it still equals `expected_revision` (else `Conflict` → 409, or
    /// `NotFound` → 404 when the sermon is gone), then the meta, the items and
    /// the slides. Returns the new revision.
    #[instrument(skip_all)]
    pub async fn save_sermon_change(
        &self,
        id: BiblePresentationId,
        write: &SermonWrite,
    ) -> anyhow::Result<i64> {
        let id_str = id.to_string();
        let now: DateTimeWithTimeZone = Utc::now().into();
        let txn = self.db.begin().await?;
        let bumped = bible_sermon::Entity::update_many()
            .col_expr(
                bible_sermon::Column::Revision,
                Expr::col(bible_sermon::Column::Revision).add(1),
            )
            .col_expr(bible_sermon::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                bible_sermon::Column::ServiceDate,
                Expr::value(write.service_date.format(DATE_FORMAT).to_string()),
            )
            .col_expr(
                bible_sermon::Column::SecondaryTranslationCode,
                Expr::value(write.secondary_translation_code.clone()),
            )
            .filter(bible_sermon::Column::PresentationId.eq(id_str.clone()))
            .filter(bible_sermon::Column::Revision.eq(write.expected_revision))
            .exec(&txn)
            .await?;
        if bumped.rows_affected == 0 {
            let exists = bible_sermon::Entity::find_by_id(id_str.clone())
                .one(&txn)
                .await?
                .is_some();
            let refusal = if exists {
                RepositoryError::Conflict(SERMON_CONFLICT)
            } else {
                RepositoryError::NotFound(SERMON_NOT_FOUND)
            };
            return Err(refusal.into());
        }
        bible_presentation::Entity::update_many()
            .col_expr(
                bible_presentation::Column::Name,
                Expr::value(write.title.clone()),
            )
            .filter(bible_presentation::Column::Id.eq(id_str.clone()))
            .exec(&txn)
            .await?;
        replace_items(&txn, &id_str, &write.items).await?;
        replace_slides(&txn, &id_str, &write.slides).await?;
        txn.commit().await?;
        Ok(write.expected_revision + 1)
    }
```

Insert after the closing `}` of `impl Repository` (before `fn parse_date`):

```rust

async fn replace_items<C: ConnectionTrait>(
    txn: &C,
    presentation_id: &str,
    items: &[SermonItemRecord],
) -> anyhow::Result<()> {
    bible_sermon_item::Entity::delete_many()
        .filter(bible_sermon_item::Column::PresentationId.eq(presentation_id.to_string()))
        .exec(txn)
        .await?;
    for (order, item) in items.iter().enumerate() {
        bible_sermon_item::Entity::insert(item_to_active(item, presentation_id, order)?)
            .exec(txn)
            .await?;
    }
    Ok(())
}

async fn replace_slides<C: ConnectionTrait>(
    txn: &C,
    presentation_id: &str,
    slides: &[BiblePresentationSlide],
) -> anyhow::Result<()> {
    bible_slide::Entity::delete_many()
        .filter(bible_slide::Column::PresentationId.eq(presentation_id.to_string()))
        .exec(txn)
        .await?;
    for (order, slide) in slides.iter().enumerate() {
        let mut normalized = slide.clone();
        normalized.order = u32::try_from(order).context("too many sermon slides")?;
        bible_slide::Entity::insert(bible_slide_to_active_model(&normalized, presentation_id)?)
            .exec(txn)
            .await?;
    }
    Ok(())
}
```

Append at the end of the file, after `item_from_model` (before `#[cfg(test)] mod tests;`):

```rust

fn item_to_active(
    item: &SermonItemRecord,
    presentation_id: &str,
    order: usize,
) -> anyhow::Result<bible_sermon_item::ActiveModel> {
    Ok(bible_sermon_item::ActiveModel {
        id: Set(item.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        item_order: Set(i32::try_from(order).context("too many sermon items")?),
        translation_code: Set(item.translation_code.clone()),
        book_code: Set(item.book_code.clone()),
        book_number: Set(i32::from(item.book_number)),
        book_name: Set(item.book_name.clone()),
        chapter: Set(i32::from(item.chapter)),
        verse_start: Set(i32::from(item.verse_start)),
        verse_end: Set(i32::from(item.verse_end)),
        source_json: Set(serde_json::to_string(&item.source)?),
        edits_json: Set(serde_json::to_string(&item.edits)?),
        slide_count: Set(i32::try_from(item.slide_count).context("sermon item slide_count")?),
        created_at: Set(item.created_at.into()),
        updated_at: Set(item.updated_at.into()),
    })
}
```

`presentations.rs`: `fn bible_slide_to_active_model(` → `pub(super) fn bible_slide_to_active_model(` (one row mapping for presentation and sermon slides). Exports — `repository/bible/mod.rs` and `repository/mod.rs` re-export `SermonWrite, SERMON_CONFLICT` next to the Task 12 names; `lib.rs`'s list gains them too (rustfmt order: `… SermonSnapshot, SermonWrite, SyncApplyOutcome, … PRUNE_HORIZON, SERMON_CONFLICT, SERMON_NOT_FOUND`).

- [ ] **Step 3: Why the first statement is the conditional UPDATE.** sqlx opens `BEGIN DEFERRED`; a read first would let two writers both hold SHARED locks and deadlock into `SQLITE_BUSY` on the upgrade (a 500). The `UPDATE … WHERE revision = expected` takes the write lock at once: a concurrent writer waits (`busy_timeout = 5000`), then sees the bumped revision and gets the 409. Returning `Err` before `commit()` drops the transaction = rollback (`a_stale_revision_is_a_conflict_and_changes_nothing` pins that nothing changed). `Expr::value(now)` binds a `DateTimeWithTimeZone` (sea-orm's default `with-chrono` feature).

- [ ] **Step 4: Commit** — paths `crates/presenter-persistence/src/repository/bible/sermons.rs crates/presenter-persistence/src/repository/bible/sermons/tests.rs crates/presenter-persistence/src/repository/bible/presentations.rs crates/presenter-persistence/src/repository/bible/mod.rs crates/presenter-persistence/src/repository/mod.rs crates/presenter-persistence/src/lib.rs`, message `feat(persistence): revision-guarded sermon writes (#851 P2b)` + body "One transaction: conditional revision bump first (409 when stale, 404 when gone), then name, items and slides." + Co-Authored-By.

### Task 17: Summaries, date lookups, the re-import test; ship P2b

**Files:**
- Modify: `crates/presenter-persistence/src/repository/bible/sermons.rs` (one struct, three reads, two helpers), `sermons/tests.rs` (append), exports, `.claude/rules/sermon-prep.md` (append)

**Interfaces:**
- Produces: `struct SermonSummary { id, title, service_date, author_email, item_count: usize, slide_count: usize, revision: i64, updated_at }`; `Repository::list_sermon_summaries() -> anyhow::Result<Vec<SermonSummary>>` (by service date, then creation), `sermon_ids_on(NaiveDate)`, `sermon_ids_from(NaiveDate) -> anyhow::Result<Vec<BiblePresentationId>>` (ISO dates compare as text).

- [ ] **Step 1: Append the tests** (T6 is the last one):

```rust

#[tokio::test]
async fn summaries_and_date_lookups() {
    let repo = repo().await;
    let later = repo
        .create_sermon(&new_sermon("Neskôr", "2026-10-25"))
        .await
        .expect("create");
    let today = repo
        .create_sermon(&new_sermon("Dnes", "2026-10-18"))
        .await
        .expect("create");
    let past = repo
        .create_sermon(&new_sermon("Minule", "2026-10-11"))
        .await
        .expect("create");
    let items = vec![item(1, "a", 2), item(2, "b", 3)];
    repo.save_sermon_change(today.id, &write(0, "Dnes", items, Vec::new()))
        .await
        .expect("write");

    let summaries = repo.list_sermon_summaries().await.expect("summaries");
    let rows: Vec<(&str, usize, usize, i64)> = summaries
        .iter()
        .map(|s| (s.title.as_str(), s.item_count, s.slide_count, s.revision))
        .collect();
    assert_eq!(
        rows,
        [("Minule", 0, 0, 0), ("Dnes", 2, 5, 1), ("Neskôr", 0, 0, 0)]
    );
    assert_eq!(
        repo.sermon_ids_on(date("2026-10-18")).await.expect("on"),
        [today.id]
    );
    assert_eq!(
        repo.sermon_ids_from(date("2026-10-18"))
            .await
            .expect("from"),
        [today.id, later.id]
    );
    assert!(!repo
        .sermon_ids_from(date("2026-10-18"))
        .await
        .expect("from")
        .contains(&past.id));
}

/// T6 / spec §5.7: a re-import (PARSER_VERSION bump) deletes and re-inserts
/// the translation; saved presentations and sermons must come out identical.
#[tokio::test]
async fn a_reimport_leaves_slides_sermons_and_items_byte_identical() {
    let repo = repo().await;
    let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    let passage = |text: &str| {
        let reference =
            BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, 1, 1).expect("reference");
        BiblePassage::new(reference, seb.clone(), text.to_string())
    };
    let first =
        BibleIngestionBatch::new(seb.clone(), vec![passage("Viera je zárukou")]).expect("batch");
    repo.replace_bible_translation_passages(&first)
        .await
        .expect("first import");
    let plain = repo
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain");
    repo.replace_bible_presentation_slides(plain.id, &[slide("1. Viera")])
        .await
        .expect("plain slides");
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    repo.save_sermon_change(
        sermon.id,
        &write(
            0,
            "Kázeň",
            vec![item(1, "Viera je zárukou", 1)],
            vec![slide("1. VIERA")],
        ),
    )
    .await
    .expect("sermon write");
    let db = repo.connection();
    let slides_before = bible_slide::Entity::find().all(db).await.expect("slides");
    let sermons_before = bible_sermon::Entity::find().all(db).await.expect("sermons");
    let items_before = bible_sermon_item::Entity::find()
        .all(db)
        .await
        .expect("items");

    let second =
        BibleIngestionBatch::new(seb.clone(), vec![passage("Viera je istota")]).expect("batch");
    repo.replace_bible_translation_passages(&second)
        .await
        .expect("re-import");

    assert_eq!(
        bible_slide::Entity::find().all(db).await.expect("slides"),
        slides_before
    );
    assert_eq!(
        bible_sermon::Entity::find().all(db).await.expect("sermons"),
        sermons_before
    );
    assert_eq!(
        bible_sermon_item::Entity::find()
            .all(db)
            .await
            .expect("items"),
        items_before
    );
    assert_eq!(slides_before.len(), 2);
    assert_eq!(items_before.len(), 1);
}
```

- [ ] **Step 2: Implement.** `sermons.rs` — insert after `SermonSnapshot` (before `SermonWrite`):

```rust

/// One row of the sermon list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonSummary {
    pub id: BiblePresentationId,
    pub title: String,
    pub service_date: NaiveDate,
    pub author_email: Option<String>,
    pub item_count: usize,
    pub slide_count: usize,
    pub revision: i64,
    pub updated_at: DateTime<Utc>,
}
```

after `SermonWrite`:

```rust

#[derive(Debug, FromQueryResult)]
struct ItemTotals {
    presentation_id: String,
    item_count: i64,
    slide_count: Option<i64>,
}
```

inside `impl Repository` after `fetch_sermon`:

```rust

    /// Every sermon, by service date, with its item and slide counts.
    #[instrument(skip_all)]
    pub async fn list_sermon_summaries(&self) -> anyhow::Result<Vec<SermonSummary>> {
        let rows = bible_sermon::Entity::find()
            .order_by_asc(bible_sermon::Column::ServiceDate)
            .order_by_asc(bible_sermon::Column::CreatedAt)
            .all(&self.db)
            .await?;
        let names: HashMap<String, String> = bible_presentation::Entity::find()
            .filter(
                bible_presentation::Column::Id
                    .is_in(rows.iter().map(|row| row.presentation_id.clone())),
            )
            .all(&self.db)
            .await?
            .into_iter()
            .map(|presentation| (presentation.id, presentation.name))
            .collect();
        let totals = item_totals(&self.db).await?;
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let (item_count, slide_count) =
                totals.get(&row.presentation_id).copied().unwrap_or((0, 0));
            let title = names.get(&row.presentation_id).cloned().unwrap_or_default();
            let sermon = sermon_from_model(row, title)?;
            summaries.push(SermonSummary {
                id: sermon.id,
                title: sermon.title,
                service_date: sermon.service_date,
                author_email: sermon.author_email,
                item_count,
                slide_count,
                revision: sermon.revision,
                updated_at: sermon.updated_at,
            });
        }
        Ok(summaries)
    }

    /// The sermons whose service date is `date`, oldest first.
    #[instrument(skip_all)]
    pub async fn sermon_ids_on(&self, date: NaiveDate) -> anyhow::Result<Vec<BiblePresentationId>> {
        let rows = bible_sermon::Entity::find()
            .filter(bible_sermon::Column::ServiceDate.eq(date.format(DATE_FORMAT).to_string()))
            .order_by_asc(bible_sermon::Column::CreatedAt)
            .all(&self.db)
            .await?;
        sermon_ids(rows)
    }

    /// The sermons whose service date is `date` or later, by date.
    #[instrument(skip_all)]
    pub async fn sermon_ids_from(
        &self,
        date: NaiveDate,
    ) -> anyhow::Result<Vec<BiblePresentationId>> {
        let rows = bible_sermon::Entity::find()
            .filter(bible_sermon::Column::ServiceDate.gte(date.format(DATE_FORMAT).to_string()))
            .order_by_asc(bible_sermon::Column::ServiceDate)
            .all(&self.db)
            .await?;
        sermon_ids(rows)
    }
```

after `replace_slides` (outside the impl):

```rust

/// Items and summed slide counts per sermon.
async fn item_totals<C: ConnectionTrait>(
    db: &C,
) -> anyhow::Result<HashMap<String, (usize, usize)>> {
    let rows = bible_sermon_item::Entity::find()
        .select_only()
        .column(bible_sermon_item::Column::PresentationId)
        .column_as(
            Expr::col(bible_sermon_item::Column::Id).count(),
            "item_count",
        )
        .column_as(
            Expr::col(bible_sermon_item::Column::SlideCount).sum(),
            "slide_count",
        )
        .group_by(bible_sermon_item::Column::PresentationId)
        .into_model::<ItemTotals>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let items = usize::try_from(row.item_count).unwrap_or(0);
            let slides = usize::try_from(row.slide_count.unwrap_or(0)).unwrap_or(0);
            (row.presentation_id, (items, slides))
        })
        .collect())
}

fn sermon_ids(rows: Vec<bible_sermon::Model>) -> anyhow::Result<Vec<BiblePresentationId>> {
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(BiblePresentationId::from_uuid(parse_uuid(
            &row.presentation_id,
        )?));
    }
    Ok(ids)
}
```

Exports gain `SermonSummary` (all three files).

- [ ] **Step 3: The rule** — append to the "Persistence" section of `.claude/rules/sermon-prep.md`:

```markdown
- **`save_sermon_change` is the ONE sermon write** (spec §7.5): one transaction whose FIRST
  statement is the conditional `UPDATE bible_sermons SET revision = revision + 1 … WHERE
  revision = expected` — a write lock at once, so a concurrent writer waits (busy_timeout)
  and then finds the revision moved → `Conflict(SERMON_CONFLICT)` → 409; a missing row →
  `NotFound(SERMON_NOT_FOUND)` → 404. Never read-then-update: two deferred readers upgrading
  to writers deadlock into SQLITE_BUSY (500), not 409. Then it replaces the presentation name,
  ALL items (order = position) and ALL slides (orders normalized) — the state computes the full
  new state in memory; the repository is dumb on purpose.
- Slides are written with `presentations::bible_slide_to_active_model` (`pub(super)`), never a
  second row mapping.
```

- [ ] **Step 4: Commit** — paths as Task 16 plus `.claude/rules/sermon-prep.md`, message `feat(persistence): sermon summaries, date lookups, re-import safety test (#851 P2b)` + Co-Authored-By.

- [ ] **Step 5: Local gates** with `<files>` = `crates/presenter-persistence/src/repository/bible/sermons.rs crates/presenter-persistence/src/repository/bible/presentations.rs crates/presenter-persistence/src/repository/bible/mod.rs crates/presenter-persistence/src/repository/mod.rs crates/presenter-persistence/src/lib.rs`. Expected: `sermons.rs` ≈ 470 production lines; `save_sermon_change` ≈ 55 lines.

- [ ] **Step 6: Ship** with `<V>` = 0.4.321, `<title>` = "Revision-guarded sermon writes and summaries (#851 P2b)". Functional check: version + `/bible/presentations` 200 on SNV and PP (no runtime caller yet).

- [ ] **Step 7:** `playbook-review`; comment on #851.

---

## PR unit P3 — slide generation, the commit path, the character-limit pass (v0.4.322)

Spec §7.4 (generation, secondary line, stable ids, character-limit regeneration), §7.5 (the commit), §13 (the INFO line). The only production caller of generation is the commit path, and the only P3 caller of the commit path is the character-limit pass in `set_bible_preferences` — that chain is why P3 cannot be split further without dead code at a head (~720 lines, the one unit over the cap).

### Task 18: Post-release version bump to 0.4.322

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.321)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.321 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.322 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.322 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 19: Generate a sermon's slides through the live composer

**Files:**
- Create: `crates/presenter-server/src/state/sermons/mod.rs`, `generate.rs`, `tests.rs` (exempt)
- Modify: `crates/presenter-server/src/state/mod.rs:51` (`pub(crate) mod sermons;` after `mod seed;` — the router names its types from P4a on)
- Modify: `crates/presenter-server/src/state/slides.rs:18` (re-export the labelled composer)
- Modify: `crates/presenter-server/src/state/bible.rs:183` (`secondary_verse_lookup` becomes `pub(super)`)

**Interfaces:**
- Consumes: `render_item`, `item_label`, `verse_suffix`, `VerseMark`, `RenderedItem` (P1); `SermonRecord`, `SermonItemRecord` (P2a); `compose_bible_slides_labelled`, `SlideLabels` (P2a); `AppState::get_bible_preferences`, `list_bible_translations` (dispatch), `secondary_verse_lookup(&BibleTranslation, book: &str, book_code: Option<&str>, chapter, verse_start, verse_end) -> anyhow::Result<(HashMap<u16, BiblePassage>, Option<String>)>`.
- Produces: `pub(crate) fn local_today() -> NaiveDate` (the only clock read); `pub(crate) struct GeneratedSermon { slides: Vec<BiblePresentationSlide>, slide_counts: Vec<u32>, warnings: Vec<String> }`; `pub(crate) fn stable_slide_id(item_id: Uuid, n: usize) -> BibleSlideId` (`Uuid::new_v5(&item_id, "slide/{n}")`); `pub(crate) async fn AppState::generate_sermon_slides(&self, &SermonRecord, &[SermonItemRecord]) -> anyhow::Result<GeneratedSermon>`.

- [ ] **Step 1: Write the tests** — `crates/presenter-server/src/state/sermons/tests.rs`:

```rust
//! #851 P3: sermon slides through the real composer and the commit path
//! (spec §7.4, §7.5, §13). SEB / KJV Hebrews 11:1-3 rows are seeded into the
//! in-memory repository; the NLT comes from a mock API (an external network
//! service, the one thing tests may mock).

use super::generate::stable_slide_id;
use super::*;
use crate::bible_remote::{NltClient, NLT_CODE};
use crate::state::AppState;
use chrono::Utc;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse, VerseEdits};
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{
    BiblePassage, BiblePreferences, BiblePresentationId, BibleReference, BibleTranslation,
};
use presenter_persistence::{SermonItemRecord, SermonRecord};
use std::time::Duration;
use uuid::Uuid;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// SEB Heb 11:1 (the mockup's verse) and two made-up verses after it.
const SEB: [&str; 3] = [
    "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,",
    "Pre ňu dostali starší dobré svedectvo.",
    "Vierou poznávame, že svety vznikli Božím slovom.",
];
/// KJV Heb 11:1-3 (public domain), shortened at the end of verse 3.
const KJV: [&str; 3] = [
    "Now faith is the substance of things hoped for, the evidence of things not seen.",
    "For by it the elders obtained a good report.",
    "Through faith we understand that the worlds were framed by the word of God.",
];

async fn seed(state: &AppState, code: &str, language: &str, book: &str, texts: &[&str]) {
    let translation = BibleTranslation::new(code, code, language);
    let mut passages = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let verse = index as u16 + 1;
        let reference =
            BibleReference::new_with_code(book, "HEB", 58, 11, verse, verse).expect("reference");
        passages.push(BiblePassage::new(
            reference,
            translation.clone(),
            (*text).to_string(),
        ));
    }
    let batch = BibleIngestionBatch::new(translation, passages).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed translation");
}

async fn hebrews() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed(&state, "slk-seb", "sk", "Hebrejom", &SEB).await;
    seed(&state, "eng-kjv", "en", "Hebrews", &KJV).await;
    state
}

fn sermon(secondary: Option<&str>) -> SermonRecord {
    SermonRecord {
        id: BiblePresentationId::new(),
        title: "Kázeň".to_string(),
        service_date: local_today(),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: secondary.map(str::to_string),
        author_email: None,
        revision: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// An SEB Hebrews 11 item over `verses` (1-based, contiguous) with `edits`.
fn item(verses: std::ops::RangeInclusive<u16>, edits: &[(u16, &[u16])]) -> SermonItemRecord {
    let source = verses
        .clone()
        .map(|verse| SourceVerse {
            verse,
            text: SEB[usize::from(verse - 1)].to_string(),
        })
        .collect();
    let edits = ItemEdits(
        edits
            .iter()
            .map(|(verse, deleted)| {
                let verse_edits = VerseEdits {
                    deleted: deleted.to_vec(),
                    emphasized: Vec::new(),
                };
                (*verse, verse_edits)
            })
            .collect(),
    );
    SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: *verses.start(),
        verse_end: *verses.end(),
        source,
        edits,
        slide_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn set_limit(state: &AppState, limit: u32) {
    state
        .set_bible_preferences(BiblePreferences::default().with_character_limit(limit))
        .await
        .expect("preferences");
}

#[tokio::test]
async fn an_item_becomes_a_slide_with_its_computed_label() {
    let state = hebrews().await;
    let heb = item(1..=1, &[(1, &[7, 8, 9, 10, 11])]);
    let mut emphasized = heb.clone();
    emphasized.edits.0.entry(1).or_default().emphasized = vec![0];

    let generated = state
        .generate_sermon_slides(&sermon(None), &[emphasized.clone()])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [1]);
    let slide = &generated.slides[0];
    assert_eq!(slide.main.value(), "1. VIERA je zárukou toho, v čo dúfame");
    assert_eq!(slide.main_reference, "Hebrejom 11:1a (SEB)");
    assert_eq!(slide.secondary.value(), "");
    assert_eq!(slide.id, stable_slide_id(emphasized.id, 0));
    let metadata = slide.metadata.as_ref().expect("bible metadata");
    assert_eq!(
        (metadata.book_code.as_deref(), metadata.chapter),
        (Some("HEB"), 11)
    );
    assert!(generated.warnings.is_empty());
}

#[tokio::test]
async fn sermon_slides_obey_the_828_limits() {
    let state = hebrews().await;
    let whole = item(1..=3, &[]);
    set_limit(&state, 100).await;

    let main_only = state
        .generate_sermon_slides(&sermon(None), &[whole.clone()])
        .await
        .expect("main only");
    let with_kjv = state
        .generate_sermon_slides(&sermon(Some("eng-kjv")), &[whole.clone()])
        .await
        .expect("with KJV");
    set_limit(&state, 60).await;
    let narrow = state
        .generate_sermon_slides(&sermon(None), &[whole])
        .await
        .expect("narrow");

    // 2 + 3 SEB lines fit 100 chars; their KJV lines (126) do not: the longer decides.
    assert_eq!(main_only.slide_counts, [2]);
    assert_eq!(with_kjv.slide_counts, [3]);
    // A lone verse longer than the limit stays one whole slide.
    assert_eq!(narrow.slide_counts, [3]);
    assert_eq!(narrow.slides[0].main.value().chars().count(), 71);
}

#[tokio::test]
async fn the_secondary_line_has_the_full_kept_verses_and_its_own_book() {
    let state = hebrews().await;
    let cut = item(1..=3, &[(1, &[7, 8, 9, 10, 11]), (2, &[0, 1, 2, 3, 4, 5])]);

    let generated = state
        .generate_sermon_slides(&sermon(Some("eng-kjv")), &[cut])
        .await
        .expect("generate");

    assert_eq!(generated.slides.len(), 1);
    let slide = &generated.slides[0];
    assert_eq!(slide.main_reference, "Hebrejom 11:1a, 3 (SEB)");
    assert_eq!(
        slide.main.value(),
        "1. Viera je zárukou toho, v čo dúfame\n3. Vierou poznávame, že svety vznikli Božím slovom."
    );
    assert_eq!(slide.secondary_reference, "Hebrews 11:1, 3 (KJV)");
    assert_eq!(
        slide.secondary.value(),
        format!("1. {}\n3. {}", KJV[0], KJV[2])
    );
}

#[tokio::test]
async fn slide_ids_are_stable_across_a_regeneration() {
    let state = hebrews().await;
    let items = [item(1..=1, &[]), item(2..=3, &[])];

    let first = state
        .generate_sermon_slides(&sermon(None), &items)
        .await
        .expect("first");
    let second = state
        .generate_sermon_slides(&sermon(None), &items)
        .await
        .expect("second");

    let first_ids: Vec<_> = first.slides.iter().map(|slide| slide.id).collect();
    let second_ids: Vec<_> = second.slides.iter().map(|slide| slide.id).collect();
    assert_eq!(first_ids, second_ids);
    assert_eq!(
        first_ids,
        [
            stable_slide_id(items[0].id, 0),
            stable_slide_id(items[1].id, 0)
        ]
    );
}

#[tokio::test]
async fn an_unavailable_nlt_gives_slides_without_the_line_and_a_warning() {
    let mut state = hebrews().await;
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(5)));

    let generated = state
        .generate_sermon_slides(&sermon(Some(NLT_CODE)), &[item(1..=2, &[])])
        .await
        .expect("a failed secondary never fails the write");

    assert_eq!(generated.slide_counts, [1]);
    assert_eq!(generated.slides[0].secondary.value(), "");
    assert_eq!(generated.slides[0].secondary_reference, "");
    assert_eq!(generated.warnings.len(), 1, "{:?}", generated.warnings);
    assert!(generated.warnings[0].ends_with("sekundárny preklad vynechaný"));
}

#[tokio::test]
async fn an_item_with_every_token_struck_has_no_slides() {
    let state = hebrews().await;
    let struck = item(2..=2, &[(2, &[0, 1, 2, 3, 4, 5])]);

    let generated = state
        .generate_sermon_slides(&sermon(None), &[struck, item(3..=3, &[])])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [0, 1]);
    assert_eq!(generated.slides.len(), 1);
}

#[tokio::test]
async fn an_item_of_a_removed_translation_still_renders_from_its_snapshot() {
    let state = hebrews().await;
    let mut gone = item(1..=1, &[]);
    gone.translation_code = "slk-gone".to_string();

    let generated = state
        .generate_sermon_slides(&sermon(None), &[gone])
        .await
        .expect("generate");

    assert_eq!(generated.slide_counts, [1]);
    assert_eq!(generated.slides[0].main_reference, "Hebrejom 11:1 (GONE)");
}
```

- [ ] **Step 2: How it is verified.** CI. Lengths (Python, 2026-10-14): SEB lines 71 / 41 / 51 chars, KJV 83 / 47 / 78; at limit 100 the SEB pair 2+3 = 93 fits but the KJV pair = 126 does not → 2 vs 3 slides (#828 longer-of); at limit 60 verse 1 (71) stays one whole slide. The NLT mock answers 503 for every request: `secondary_verse_lookup` turns the typed `RemoteBibleError` into the warning "… — sekundárny preklad vynechaný" (existing #826 wording).

- [ ] **Step 3: Implement** — `crates/presenter-server/src/state/sermons/mod.rs` (Task 20 adds the commit path):

```rust
//! Sermon preparation on the server (#851 spec §7). A write loads the sermon,
//! computes the new item list in memory, generates the slides from the items
//! (`generate.rs`) and commits items + slides in ONE repository transaction
//! guarded by the expected revision (`commit.rs`): a stale write is
//! `RepositoryError::Conflict` → 409 and is never half-applied. Then
//! `LiveEvent::BibleSlidesChanged` is published and one INFO
//! `presenter::bible::sermon` line is logged. Item text is a snapshot, so a
//! Bible re-import never changes a sermon (spec §5.7).

mod generate;

use chrono::NaiveDate;

/// The server-local date: "today" for `/bible/sermons/today`, the default
/// service date and the character-limit pass. The only clock read of the
/// sermon code; everything below it takes the date as a parameter.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests;
```

`crates/presenter-server/src/state/sermons/generate.rs`:

```rust
//! Sermon slides (#851 spec §7.4). Each item is rendered (`render_item`) and
//! composed by the live composer (`compose_bible_slides_labelled`, so the #828
//! rules hold unchanged: the LONGER of main/secondary decides, in `chars()`, a
//! lone oversized verse stays whole) with the item's computed label. The
//! secondary line carries the FULL verses of the kept verse numbers in the
//! sermon's secondary translation, labelled with the secondary book and plain
//! numbers; an unavailable NLT gives slides without the line plus a warning,
//! never a failed write. NLT text stored this way is bounded to the sermon's
//! verses, exactly like the operator's append path.

use std::collections::HashMap;

use presenter_core::bible::sermon::{
    item_label, render_item, verse_suffix, RenderedItem, VerseMark,
};
use presenter_core::{
    BiblePassage, BiblePresentationSlide, BibleReference, BibleSlideId, BibleTranslation, Slide,
};
use presenter_persistence::{SermonItemRecord, SermonRecord};
use uuid::Uuid;

use super::super::slides::{compose_bible_slides_labelled, SlideLabels};
use super::super::AppState;

/// The slides of a whole sermon, in item order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GeneratedSermon {
    pub(crate) slides: Vec<BiblePresentationSlide>,
    /// Slides per item, in item order (`bible_sermon_items.slide_count`).
    pub(crate) slide_counts: Vec<u32>,
    /// Operator-facing warnings (an unavailable secondary translation).
    pub(crate) warnings: Vec<String>,
}

/// The id of an item's `n`-th slide. Stable while the item exists, so a slide
/// selected on the tablet or in the console survives an edit elsewhere.
pub(crate) fn stable_slide_id(item_id: Uuid, n: usize) -> BibleSlideId {
    BibleSlideId::from_uuid(Uuid::new_v5(&item_id, format!("slide/{n}").as_bytes()))
}

impl AppState {
    /// The slides of `items` under the current character limit.
    pub(crate) async fn generate_sermon_slides(
        &self,
        sermon: &SermonRecord,
        items: &[SermonItemRecord],
    ) -> anyhow::Result<GeneratedSermon> {
        let character_limit = self.get_bible_preferences().await?.character_limit;
        let translations = self.list_bible_translations().await?;
        let secondary = sermon
            .secondary_translation_code
            .as_deref()
            .map(|code| translation_or_code(&translations, code));
        let mut generated = GeneratedSermon {
            slides: Vec::new(),
            slide_counts: Vec::with_capacity(items.len()),
            warnings: Vec::new(),
        };
        for item in items {
            let main = translation_or_code(&translations, &item.translation_code);
            let slides = self
                .item_slides(
                    item,
                    &main,
                    secondary.as_ref(),
                    character_limit,
                    &mut generated.warnings,
                )
                .await?;
            generated
                .slide_counts
                .push(u32::try_from(slides.len()).unwrap_or(u32::MAX));
            generated.slides.extend(slides);
        }
        Ok(generated)
    }

    async fn item_slides(
        &self,
        item: &SermonItemRecord,
        main: &BibleTranslation,
        secondary: Option<&BibleTranslation>,
        character_limit: u32,
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<Vec<BiblePresentationSlide>> {
        let rendered = render_item(&item.source, &item.edits);
        if rendered.is_empty() {
            return Ok(Vec::new());
        }
        let passages = rendered_passages(item, main, &rendered)?;
        let (lookup, translation_label) = match secondary {
            Some(translation) => {
                self.sermon_secondary(item, translation, &rendered, warnings)
                    .await?
            }
            None => (HashMap::new(), None),
        };
        let secondary = secondary.filter(|_| translation_label.is_some());
        let labels = SlideLabels {
            main: item_label(
                &item.book_name,
                item.chapter,
                &rendered.suffix,
                &item.translation_code,
            ),
            translation: translation_label,
        };
        let slides = compose_bible_slides_labelled(
            main,
            secondary,
            &passages,
            &lookup,
            character_limit,
            labels,
        )?;
        Ok(slides
            .into_iter()
            .enumerate()
            .map(|(n, slide)| presentation_slide(item.id, n, slide))
            .collect())
    }

    /// The secondary verses of the KEPT verse numbers and their plain label;
    /// nothing when the translation has none of them (plus a warning when it
    /// was unavailable).
    async fn sermon_secondary(
        &self,
        item: &SermonItemRecord,
        translation: &BibleTranslation,
        rendered: &RenderedItem,
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<(HashMap<u16, BiblePassage>, Option<String>)> {
        let kept: Vec<u16> = rendered.verses.iter().map(|verse| verse.verse).collect();
        let (Some(&first), Some(&last)) = (kept.first(), kept.last()) else {
            return Ok((HashMap::new(), None));
        };
        let (mut lookup, warning) = self
            .secondary_verse_lookup(
                translation,
                &item.book_name,
                Some(item.book_code.as_str()),
                item.chapter,
                first,
                last,
            )
            .await?;
        if let Some(warning) = warning {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        lookup.retain(|verse, _| kept.contains(verse));
        let marks: Vec<VerseMark> = kept
            .iter()
            .copied()
            .filter(|verse| lookup.contains_key(verse))
            .map(VerseMark::plain)
            .collect();
        let Some(book) = lookup.values().next().map(|p| p.reference.book.clone()) else {
            return Ok((HashMap::new(), None));
        };
        let label = item_label(
            &book,
            item.chapter,
            &verse_suffix(&marks),
            &translation.code,
        );
        Ok((lookup, Some(label)))
    }
}

/// The installed translation, or a bare stand-in when it was removed after the
/// item was added: the snapshot still renders (spec §5.7).
fn translation_or_code(translations: &[BibleTranslation], code: &str) -> BibleTranslation {
    translations
        .iter()
        .find(|translation| translation.code.eq_ignore_ascii_case(code))
        .cloned()
        .unwrap_or_else(|| BibleTranslation::new(code, code, ""))
}

/// The rendered verses as passages for the composer: the stored book name and
/// code, one passage per kept verse.
fn rendered_passages(
    item: &SermonItemRecord,
    main: &BibleTranslation,
    rendered: &RenderedItem,
) -> anyhow::Result<Vec<BiblePassage>> {
    let mut passages = Vec::with_capacity(rendered.verses.len());
    for verse in &rendered.verses {
        let reference = BibleReference::new_with_code(
            item.book_name.as_str(),
            item.book_code.as_str(),
            item.book_number,
            item.chapter,
            verse.verse,
            verse.verse,
        )?;
        passages.push(BiblePassage::new(
            reference,
            main.clone(),
            verse.text.clone(),
        ));
    }
    Ok(passages)
}

/// A composed slide as a stored presentation slide with its stable id; the
/// labels come from the composer's metadata, like the `/bible/resolve` DTO.
fn presentation_slide(item_id: Uuid, n: usize, slide: Slide) -> BiblePresentationSlide {
    let bible = slide.metadata.and_then(|metadata| metadata.bible);
    let (main_reference, secondary_reference) = bible
        .as_ref()
        .map(|meta| {
            (
                meta.main_reference_label.clone().unwrap_or_default(),
                meta.translation_reference_label.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    BiblePresentationSlide {
        id: stable_slide_id(item_id, n),
        order: 0,
        main: slide.content.main,
        main_reference,
        secondary: slide.content.translation,
        secondary_reference,
        metadata: bible,
    }
}
```

`state/mod.rs` line 51: `mod seed;` → `mod seed;` + new line `pub(crate) mod sermons;`. `state/slides.rs` line 18:

```rust
pub(crate) use compose::{compose_bible_slides, compose_bible_slides_labelled, SlideLabels};
```

`state/bible.rs` line 183: `    async fn secondary_verse_lookup(` → `    pub(super) async fn secondary_verse_lookup(` (`pub(super)` from `state::bible` = visible in `state` and below; its private `SecondaryVerses` type alias is transparent).

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/state/sermons/ crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/slides.rs crates/presenter-server/src/state/bible.rs`, message `feat(server): generate sermon slides through the live composer (#851 P3)` + body "Item label + #828 limits unchanged, v5 slide ids, secondary line = full kept verses with the secondary book, NLT down = warning." + Co-Authored-By.

### Task 20: The commit path, the INFO line, the character-limit pass

**Files:**
- Create: `crates/presenter-server/src/state/sermons/commit.rs`
- Modify: `crates/presenter-server/src/state/sermons/mod.rs` (declare `commit`, add `SermonOrigin` and `SermonOp`), `tests.rs` (imports + append)
- Modify: `crates/presenter-server/src/state/bible.rs:436-445` (`set_bible_preferences`)

**Interfaces:**
- Consumes: `generate_sermon_slides` (Task 19); `Repository::fetch_sermon`, `sermon_ids_from`, `save_sermon_change` (P2a/P2b); `LiveHub::publish`.
- Produces: `pub(crate) struct SermonOrigin { author: Option<String>, client_ip: String }` + `fn system()`; `pub(crate) enum SermonOp { Regenerate }` + `const fn as_str(self) -> &'static str`; in `commit.rs`: `pub(crate) struct CommittedSermon { slide_count: usize, warnings: Vec<String> }`, `pub(crate) struct SermonChange<'a> { op, origin: &'a SermonOrigin, expected_revision: i64, sermon: SermonRecord, items: Vec<SermonItemRecord>, item_id: Option<Uuid> }`, `pub(crate) async fn AppState::commit_sermon(&self, SermonChange<'_>) -> anyhow::Result<CommittedSermon>`, `pub(crate) async fn AppState::regenerate_future_sermons(&self, today: NaiveDate)`.

- [ ] **Step 1: Append the test** to `tests.rs`, and extend its imports: `use crate::live::LiveEvent;`, `use crate::tracing_capture::{quoted, Capture};`, `use chrono::Utc;` → `use chrono::{Days, Utc};`, `use presenter_persistence::{SermonItemRecord, SermonRecord};` → `use presenter_persistence::{NewSermon, SermonItemRecord, SermonRecord, SermonWrite};`:

```rust

/// A stored sermon dated `date` holding `items` (revision 1, no slides yet).
async fn stored_sermon(
    state: &AppState,
    date: chrono::NaiveDate,
    items: Vec<SermonItemRecord>,
) -> BiblePresentationId {
    let new = NewSermon {
        title: "Kázeň".to_string(),
        service_date: date,
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: None,
        author_email: None,
    };
    let created = state
        .repository()
        .create_sermon(&new)
        .await
        .expect("create");
    let write = SermonWrite {
        expected_revision: 0,
        title: "Kázeň".to_string(),
        service_date: date,
        secondary_translation_code: None,
        items,
        slides: Vec::new(),
    };
    state
        .repository()
        .save_sermon_change(created.id, &write)
        .await
        .expect("seed write");
    created.id
}

#[tokio::test]
async fn a_new_character_limit_regenerates_future_sermons_only() {
    let state = hebrews().await;
    let today = local_today();
    let past_date = today.checked_sub_days(Days::new(7)).expect("a week ago");
    let future = stored_sermon(&state, today, vec![item(1..=3, &[])]).await;
    let past = stored_sermon(&state, past_date, vec![item(1..=3, &[])]).await;
    let mut events = state.live_hub().subscribe();
    let capture = Capture::install();

    set_limit(&state, 60).await;

    let fetch = |id| {
        let state = state.clone();
        async move {
            state
                .repository()
                .fetch_sermon(id)
                .await
                .expect("fetch")
                .expect("sermon")
        }
    };
    let future_now = fetch(future).await;
    assert_eq!(future_now.sermon.revision, 2);
    assert_eq!(future_now.items[0].slide_count, 3);
    let future_slides = state
        .bible_presentation_detail(future)
        .await
        .expect("detail")
        .expect("presentation")
        .slides;
    assert_eq!(future_slides.len(), 3);
    assert_eq!(fetch(past).await.sermon.revision, 1);
    let mut changed = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LiveEvent::BibleSlidesChanged { presentation_id } = event {
            changed.push(presentation_id);
        }
    }
    assert_eq!(changed, [future.to_string()]);
    let writes: Vec<_> = capture
        .events("presenter::bible::sermon")
        .into_iter()
        .filter(|line| line.field("message") == Some("sermon write"))
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].level, tracing::Level::INFO);
    assert_eq!(writes[0].field("op"), Some(quoted("regenerate").as_str()));
    assert_eq!(
        writes[0].field("client_ip"),
        Some(quoted("system").as_str())
    );
    assert_eq!(writes[0].field("author"), Some(quoted("-").as_str()));
    assert_eq!(writes[0].field("slides"), Some("3"));

    set_limit(&state, 60).await;
    assert_eq!(
        fetch(future).await.sermon.revision,
        2,
        "same limit, no pass"
    );
}
```

- [ ] **Step 2: Implement** — `crates/presenter-server/src/state/sermons/commit.rs`:

```rust
//! The one write path of a sermon (#851 spec §7.5): generate the slides from
//! the new item list, store items + slides in ONE transaction guarded by the
//! expected revision, publish `BibleSlidesChanged`, log one INFO line.

use std::time::{Duration, Instant};

use chrono::NaiveDate;
use presenter_core::BiblePresentationId;
use presenter_persistence::{
    RepositoryError, SermonItemRecord, SermonRecord, SermonWrite, SERMON_NOT_FOUND,
};
use uuid::Uuid;

use super::super::AppState;
use super::{SermonOp, SermonOrigin};
use crate::live::LiveEvent;

/// What a committed write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedSermon {
    pub(crate) slide_count: usize,
    /// Operator-facing warnings of the generation (an unavailable NLT).
    pub(crate) warnings: Vec<String>,
}

/// The new state of one sermon, computed in memory before the commit.
pub(crate) struct SermonChange<'a> {
    pub(crate) op: SermonOp,
    pub(crate) origin: &'a SermonOrigin,
    pub(crate) expected_revision: i64,
    /// The sermon with its new title, date and secondary translation.
    pub(crate) sermon: SermonRecord,
    /// The new item list, in order; `slide_count` is filled in here.
    pub(crate) items: Vec<SermonItemRecord>,
    /// The item the write is about (the log line's `item_id`).
    pub(crate) item_id: Option<Uuid>,
}

/// The fields of one `presenter::bible::sermon` line (spec §13).
struct SermonLine<'a> {
    op: SermonOp,
    sermon_id: BiblePresentationId,
    item_id: Option<Uuid>,
    revision: i64,
    items: usize,
    slides: usize,
    origin: &'a SermonOrigin,
    elapsed: Duration,
    warning: Option<&'a str>,
}

fn log_sermon_write(line: &SermonLine<'_>) {
    let item_id = line
        .item_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    tracing::info!(
        target: "presenter::bible::sermon",
        op = line.op.as_str(),
        sermon_id = %line.sermon_id,
        item_id = %item_id,
        revision = line.revision,
        items = line.items,
        slides = line.slides,
        author = line.origin.author.as_deref().unwrap_or("-"),
        client_ip = line.origin.client_ip.as_str(),
        ms = u64::try_from(line.elapsed.as_millis()).unwrap_or(u64::MAX),
        warning = line.warning.unwrap_or("-"),
        "sermon write"
    );
}

fn is_conflict(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<RepositoryError>(),
        Some(RepositoryError::Conflict(_))
    )
}

impl AppState {
    /// Generate, store (409 when `expected_revision` is stale), publish, log.
    pub(crate) async fn commit_sermon(
        &self,
        change: SermonChange<'_>,
    ) -> anyhow::Result<CommittedSermon> {
        let started = Instant::now();
        let generated = self
            .generate_sermon_slides(&change.sermon, &change.items)
            .await?;
        let mut items = change.items;
        for (item, count) in items.iter_mut().zip(&generated.slide_counts) {
            item.slide_count = *count;
        }
        let write = SermonWrite {
            expected_revision: change.expected_revision,
            title: change.sermon.title.clone(),
            service_date: change.sermon.service_date,
            secondary_translation_code: change.sermon.secondary_translation_code.clone(),
            items,
            slides: generated.slides,
        };
        let revision = self
            .repository
            .save_sermon_change(change.sermon.id, &write)
            .await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: change.sermon.id.to_string(),
        });
        log_sermon_write(&SermonLine {
            op: change.op,
            sermon_id: change.sermon.id,
            item_id: change.item_id,
            revision,
            items: write.items.len(),
            slides: write.slides.len(),
            origin: change.origin,
            elapsed: started.elapsed(),
            warning: generated.warnings.first().map(String::as_str),
        });
        Ok(CommittedSermon {
            slide_count: write.slides.len(),
            warnings: generated.warnings,
        })
    }

    /// Generate the slides of the sermon's CURRENT items again, at its current
    /// revision (the character-limit pass).
    async fn regenerate_current(
        &self,
        id: BiblePresentationId,
        origin: &SermonOrigin,
    ) -> anyhow::Result<CommittedSermon> {
        let snapshot = self
            .repository
            .fetch_sermon(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        self.commit_sermon(SermonChange {
            op: SermonOp::Regenerate,
            origin,
            expected_revision: snapshot.sermon.revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }

    /// Spec §7.4: a new character limit regenerates the slides of every
    /// sermon dated `today` or later (a few rows), retrying once on a revision
    /// conflict. A failure is a WARN and never fails the preference write.
    pub(crate) async fn regenerate_future_sermons(&self, today: NaiveDate) {
        let ids = match self.repository.sermon_ids_from(today).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    target: "presenter::bible::sermon",
                    error = %err,
                    "character limit changed: future sermons not listed"
                );
                return;
            }
        };
        let origin = SermonOrigin::system();
        let mut slides = 0;
        for id in &ids {
            let first = self.regenerate_current(*id, &origin).await;
            let result = match first {
                Err(err) if is_conflict(&err) => self.regenerate_current(*id, &origin).await,
                other => other,
            };
            match result {
                Ok(committed) => {
                    slides += committed.slide_count;
                    for warning in &committed.warnings {
                        tracing::warn!(
                            target: "presenter::bible::sermon",
                            sermon_id = %id,
                            warning = warning.as_str(),
                            "character limit changed: sermon regenerated with a warning"
                        );
                    }
                }
                Err(err) => tracing::warn!(
                    target: "presenter::bible::sermon",
                    sermon_id = %id,
                    error = %err,
                    "character limit changed: sermon not regenerated"
                ),
            }
        }
        tracing::info!(
            target: "presenter::bible::sermon",
            sermons = ids.len(),
            slides,
            "character limit changed: future sermons regenerated"
        );
    }
}
```

`mod.rs` — the whole file after this task:

```rust
//! Sermon preparation on the server (#851 spec §7). A write loads the sermon,
//! computes the new item list in memory, generates the slides from the items
//! (`generate.rs`) and commits items + slides in ONE repository transaction
//! guarded by the expected revision (`commit.rs`): a stale write is
//! `RepositoryError::Conflict` → 409 and is never half-applied. Then
//! `LiveEvent::BibleSlidesChanged` is published and one INFO
//! `presenter::bible::sermon` line is logged. Item text is a snapshot, so a
//! Bible re-import never changes a sermon (spec §5.7).

mod commit;
mod generate;

use chrono::NaiveDate;

/// Who wrote (spec §13): the Cloudflare Access e-mail when the request carried
/// one, and the client IP by the #793 rule. Both are labels, never
/// permissions (spec §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonOrigin {
    pub(crate) author: Option<String>,
    pub(crate) client_ip: String,
}

impl SermonOrigin {
    /// A write the server makes on its own (the character-limit pass).
    pub(crate) fn system() -> Self {
        Self {
            author: None,
            client_ip: "system".to_string(),
        }
    }
}

/// The `op` field of the `presenter::bible::sermon` line (spec §13). A unit
/// adds its variant together with the write that constructs it (`dead_code`
/// under `-D warnings` otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SermonOp {
    Regenerate,
}

impl SermonOp {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Regenerate => "regenerate",
        }
    }
}

/// The server-local date: "today" for `/bible/sermons/today`, the default
/// service date and the character-limit pass. The only clock read of the
/// sermon code; everything below it takes the date as a parameter.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests;
```

`state/bible.rs` — replace `set_bible_preferences` (lines 436-445) with:

```rust
    pub async fn set_bible_preferences(&self, prefs: BiblePreferences) -> anyhow::Result<()> {
        let key = "bible-preferences";
        // #851 P3: a new character limit regenerates the sermons from today on;
        // an unreadable stored row counts as a change.
        let previous_limit = self
            .get_bible_preferences()
            .await
            .ok()
            .map(|preferences| preferences.character_limit);
        let json = serde_json::to_string(&prefs)?;
        self.repository.set_app_setting(key, &json).await?;
        self.live_hub.publish(LiveEvent::BiblePreferencesChanged {
            character_limit: prefs.character_limit,
        });
        if previous_limit != Some(prefs.character_limit) {
            self.regenerate_future_sermons(super::sermons::local_today())
                .await;
        }
        Ok(())
    }
```

(`state/bible.rs` → 763 + 11 production lines.)

- [ ] **Step 3: How it is verified.** CI: the char-limit test (a future sermon goes revision 1 → 2 with 3 slides at limit 60, the past one stays at 1, exactly one `BibleSlidesChanged` for the future sermon, exactly one INFO "sermon write" line with `op="regenerate"`, `client_ip="system"`, `author="-"`, `slides=3`; the same limit again changes nothing). Every other test that calls `set_bible_preferences` with a new limit now also runs the pass over zero sermons (one SELECT, one INFO "… sermons=0" line) — harmless.

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/state/sermons/ crates/presenter-server/src/state/bible.rs`, message `feat(server): sermon commit path, INFO line, character-limit regeneration (#851 P3)` + Co-Authored-By.

### Task 21: Rule section, ship P3

**Files:**
- Modify: `.claude/rules/sermon-prep.md` (append)

- [ ] **Step 1: Append** to `.claude/rules/sermon-prep.md`:

```markdown

## Slide generation and the commit path (P3, `state/sermons/{generate,commit}.rs`)

- Each item: `render_item` → one `BiblePassage` per kept verse (stored book name and code,
  rendered text) → `compose_bible_slides_labelled` with `item_label(...)`, so #828 holds
  unchanged (LONGER of main/secondary in `chars()`, a lone oversized verse stays whole). An
  all-struck item produces 0 slides.
- **Slide ids are `Uuid::new_v5(item_id, "slide/{n}")`** (`stable_slide_id`): unchanged items keep
  their ids across every regeneration, so a tablet/console selection survives an edit elsewhere;
  `SermonItemView::slide_ids` / the slide → item map are derived from `slide_count`, not stored.
- **Secondary line** = the sermon's `secondary_translation_code` (default at creation: the Bible
  preference's secondary): the FULL verses of the KEPT verse numbers (no deletions mapped
  across languages), label = the SECONDARY translation's book name + plain-number runs
  ("Hebrews 11:1, 3 (KJV)"). Read through `AppState::secondary_verse_lookup` (`pub(super)`):
  an unavailable NLT → slides without the line + a `warnings` entry, never a failed write; NLT
  text stored this way is bounded to the sermon's verses.
- A translation uninstalled after the snapshot renders from the snapshot with a stand-in
  (`translation_or_code`): label "(GONE)" style short code, no failure.
- `commit_sermon(SermonChange)`: generate → `save_sermon_change` (409 when stale) → publish
  `LiveEvent::BibleSlidesChanged { presentation_id }` (no new variant, `companion/variables.rs`
  untouched) → ONE INFO `presenter::bible::sermon` "sermon write" line with `op`, `sermon_id`,
  `item_id`, `revision`, `items`, `slides`, `author`, `client_ip` (both `&str`, quoted — client
  controlled), `ms`, `warning`. `SermonOp` gets a variant only with the write that constructs it.
- **Character limit**: `set_bible_preferences` with a NEW `character_limit` (an unreadable stored
  row counts as new) runs `regenerate_future_sermons(local_today())`: every sermon dated today or
  later, current revision, one retry on a conflict, WARN on failure, never failing the
  preference write. Same limit → no pass.
- Dev2's `presenter-dev.service.d/gst-debug.conf` lists `presenter::bible::show=info` but not
  `presenter::bible::sermon`, so dev's journal shows no sermon lines; check T8 on SNV
  (`journalctl -u presenter | grep presenter::bible::sermon`).
```

Commit — `docs(playbook): sermon generation and commit rules (#851 P3) [no-test: docs only]`.

- [ ] **Step 2: Local gates** with `<files>` = `crates/presenter-server/src/state/sermons/mod.rs crates/presenter-server/src/state/sermons/generate.rs crates/presenter-server/src/state/sermons/commit.rs crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/slides.rs crates/presenter-server/src/state/bible.rs`. Expected: `state/mod.rs` 944, `state/bible.rs` ≈ 774 production lines; `generate_sermon_slides`, `item_slides`, `sermon_secondary`, `commit_sermon` each < 60 lines.

- [ ] **Step 3: Ship** with `<V>` = 0.4.322, `<title>` = "Sermon slide generation and commit path (#851 P3)". Functional check on SNV/PP: version + `journalctl -u presenter --since '-15 min' --no-pager | grep -c 'presenter::bible::sermon'` = 0 errors (no sermons exist yet; the pass runs only on a character-limit change, which is an operator setting — not touched here; P4b exercises the writes with a test sermon).

- [ ] **Step 4:** `playbook-review`; comment on #851 (P3 shipped; P3 measured ≈ 720 lines, over the cap for the reason above).

---

## PR unit P4a — sermon reads over HTTP, the Access identity (v0.4.323)

Spec §11 (`GET /bible/sermons`, `/bible/sermons/today`, `/bible/sermons/{id}` with the full `SermonDetail`, `GET /bible/study/identity`), §9 (the author label rule), §12 (WARN on every failed request). The DTO layer lands here because the reads need it; writes follow in P4b–P4d.

### Task 22: Post-release version bump to 0.4.323

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.322)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.322 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.323 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.323 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 23: The sermon views — detail, list, today, `sourceChanged`

**Files:**
- Create: `crates/presenter-server/src/state/sermons/view.rs`
- Modify: `crates/presenter-server/src/state/sermons/mod.rs` (declare + re-export), `tests.rs` (append)

**Interfaces:**
- Consumes: `Repository::fetch_sermon`, `fetch_bible_presentation`, `list_sermon_summaries`, `sermon_ids_on`; `stable_slide_id`; `render_item`, `item_label`, `default_service_date`; `AppState::bible_passage_range` (dispatch).
- Produces (re-exported from `state::sermons`): `SermonDetail { sermon: SermonRecord, items: Vec<SermonItemView>, slides: Vec<SermonSlideView>, warnings: Vec<String> }`; `SermonItemView { record: SermonItemRecord, label: String, rendered_text: String, source_changed: bool, slide_ids: Vec<BibleSlideId> }`; `SermonSlideView { item_id: Uuid, slide: BiblePresentationSlide }`; `SermonList { today: NaiveDate, default_service_date: NaiveDate, sermons: Vec<SermonSummary> }`; `pub(crate)` `AppState::sermon_detail(BiblePresentationId) -> anyhow::Result<Option<SermonDetail>>`, `list_sermons(today: NaiveDate) -> anyhow::Result<SermonList>`, `sermons_on(NaiveDate) -> anyhow::Result<Vec<SermonDetail>>`, `source_changed(&SermonItemRecord) -> anyhow::Result<bool>`.

- [ ] **Step 1: Append the tests** to `crates/presenter-server/src/state/sermons/tests.rs`:

```rust

// ---- #851 P4a: the sermon reads ---------------------------------------------

fn date(text: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

#[tokio::test]
async fn the_detail_names_items_and_owns_their_slides() {
    let state = hebrews().await;
    let cut = item(1..=1, &[(1, &[7, 8, 9, 10, 11])]);
    let id = stored_sermon(&state, local_today(), vec![cut, item(2..=3, &[])]).await;
    state.regenerate_future_sermons(local_today()).await;

    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");

    assert_eq!(detail.sermon.revision, 2);
    let labels: Vec<&str> = detail
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(labels, ["Hebrejom 11:1a (SEB)", "Hebrejom 11:2-3 (SEB)"]);
    assert_eq!(
        detail.items[0].rendered_text,
        "1. Viera je zárukou toho, v čo dúfame"
    );
    let owners: Vec<Uuid> = detail.slides.iter().map(|slide| slide.item_id).collect();
    assert_eq!(
        owners,
        [detail.items[0].record.id, detail.items[1].record.id]
    );
    assert_eq!(detail.items[0].slide_ids, [detail.slides[0].slide.id]);
    assert!(detail.items.iter().all(|item| !item.source_changed));
    assert!(detail.warnings.is_empty());
    let unknown = state
        .sermon_detail(BiblePresentationId::new())
        .await
        .expect("detail");
    assert!(unknown.is_none());
}

#[tokio::test]
async fn a_changed_database_text_flags_the_item_and_keeps_its_snapshot() {
    let state = hebrews().await;
    let id = stored_sermon(&state, local_today(), vec![item(1..=2, &[])]).await;

    seed(
        &state,
        "slk-seb",
        "sk",
        "Hebrejom",
        &[SEB[0], "Iné znenie druhého verša.", SEB[2]],
    )
    .await;

    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert!(detail.items[0].source_changed);
    assert_eq!(
        detail.items[0].record.source[1].text, SEB[1],
        "the snapshot stays"
    );
}

#[tokio::test]
async fn the_list_carries_the_server_dates_and_today_lists_todays_sermons() {
    let state = hebrews().await;
    let today = local_today();
    let tomorrow = today.checked_add_days(Days::new(1)).expect("tomorrow");
    let todays = stored_sermon(&state, today, Vec::new()).await;
    stored_sermon(&state, tomorrow, Vec::new()).await;

    let list = state.list_sermons(date("2026-10-14")).await.expect("list");
    let on: Vec<BiblePresentationId> = state
        .sermons_on(today)
        .await
        .expect("today")
        .into_iter()
        .map(|detail| detail.sermon.id)
        .collect();

    assert_eq!(list.today, date("2026-10-14"));
    assert_eq!(list.default_service_date, date("2026-10-18"));
    assert_eq!(list.sermons.len(), 2);
    assert_eq!(on, [todays]);
}
```

- [ ] **Step 2: Implement** — `crates/presenter-server/src/state/sermons/view.rs`:

```rust
//! What the sermon API answers (#851 spec §11 `SermonDetail`): the sermon,
//! its items with the computed label, the rendered text, `sourceChanged` (the
//! snapshot no longer matches the database, e.g. after a re-import) and the ids
//! of their slides, and the stored slides with their item. Reads go through the
//! `AppState` dispatch in `bible_source.rs` (bible-remote rule).

use std::collections::HashMap;

use chrono::NaiveDate;
use presenter_core::bible::sermon::{default_service_date, item_label, render_item};
use presenter_core::{BiblePresentationId, BiblePresentationSlide, BibleSlideId};
use presenter_persistence::{SermonItemRecord, SermonRecord, SermonSummary};
use uuid::Uuid;

use super::super::AppState;
use super::generate::stable_slide_id;

/// One sermon as the editor, the console and the tablet read it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SermonDetail {
    pub(crate) sermon: SermonRecord,
    pub(crate) items: Vec<SermonItemView>,
    pub(crate) slides: Vec<SermonSlideView>,
    /// Warnings of the write that produced this answer (an unavailable NLT);
    /// empty on a read.
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonItemView {
    pub(crate) record: SermonItemRecord,
    /// "Hebrejom 11:1a (SEB)".
    pub(crate) label: String,
    /// "1. VIERA je zárukou …" lines, as the slides carry them.
    pub(crate) rendered_text: String,
    pub(crate) source_changed: bool,
    pub(crate) slide_ids: Vec<BibleSlideId>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SermonSlideView {
    pub(crate) item_id: Uuid,
    pub(crate) slide: BiblePresentationSlide,
}

/// The sermon list with the server's dates (the phone's clock never decides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonList {
    pub(crate) today: NaiveDate,
    pub(crate) default_service_date: NaiveDate,
    pub(crate) sermons: Vec<SermonSummary>,
}

impl AppState {
    /// The sermon; `None` when `id` is no sermon.
    pub(crate) async fn sermon_detail(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<SermonDetail>> {
        let Some(snapshot) = self.repository.fetch_sermon(id).await? else {
            return Ok(None);
        };
        let stored = self
            .repository
            .fetch_bible_presentation(id)
            .await?
            .map(|presentation| presentation.slides)
            .unwrap_or_default();
        let mut owners: HashMap<BibleSlideId, Uuid> = HashMap::new();
        let mut items = Vec::with_capacity(snapshot.items.len());
        for record in snapshot.items {
            let slide_count = usize::try_from(record.slide_count).unwrap_or(0);
            let slide_ids: Vec<BibleSlideId> = (0..slide_count)
                .map(|n| stable_slide_id(record.id, n))
                .collect();
            owners.extend(slide_ids.iter().map(|slide_id| (*slide_id, record.id)));
            let rendered = render_item(&record.source, &record.edits);
            items.push(SermonItemView {
                label: item_label(
                    &record.book_name,
                    record.chapter,
                    &rendered.suffix,
                    &record.translation_code,
                ),
                rendered_text: rendered.numbered_text(),
                source_changed: self.source_changed(&record).await?,
                slide_ids,
                record,
            });
        }
        let slides = stored
            .into_iter()
            .filter_map(|slide| {
                let item_id = *owners.get(&slide.id)?;
                Some(SermonSlideView { item_id, slide })
            })
            .collect();
        Ok(Some(SermonDetail {
            sermon: snapshot.sermon,
            items,
            slides,
            warnings: Vec::new(),
        }))
    }

    /// Whether some snapshot verse differs from the database text now, or is
    /// gone (spec §7.5 `sourceChanged`).
    pub(crate) async fn source_changed(&self, item: &SermonItemRecord) -> anyhow::Result<bool> {
        let current = self
            .bible_passage_range(
                &item.translation_code,
                &item.book_name,
                Some(item.book_code.as_str()),
                item.chapter,
                item.verse_start,
                item.verse_end,
            )
            .await?;
        Ok(item.source.iter().any(|verse| {
            !current
                .iter()
                .any(|row| row.reference.verse_start == verse.verse && row.text == verse.text)
        }))
    }

    /// Every sermon plus `today` and the default service date.
    pub(crate) async fn list_sermons(&self, today: NaiveDate) -> anyhow::Result<SermonList> {
        Ok(SermonList {
            today,
            default_service_date: default_service_date(today),
            sermons: self.repository.list_sermon_summaries().await?,
        })
    }

    /// The sermons of `date` in full (`GET /bible/sermons/today`).
    pub(crate) async fn sermons_on(&self, date: NaiveDate) -> anyhow::Result<Vec<SermonDetail>> {
        let mut details = Vec::new();
        for id in self.repository.sermon_ids_on(date).await? {
            if let Some(detail) = self.sermon_detail(id).await? {
                details.push(detail);
            }
        }
        Ok(details)
    }
}
```

`mod.rs` — the module block becomes:

```rust
mod commit;
mod generate;
mod view;

pub(crate) use view::{SermonDetail, SermonItemView, SermonList, SermonSlideView};
```

- [ ] **Step 3: Commit** — paths `crates/presenter-server/src/state/sermons/`, message `feat(server): sermon detail, list and today views (#851 P4a)` + Co-Authored-By.

### Task 24: `GET /bible/sermons*` and `GET /bible/study/identity`

**Files:**
- Create: `crates/presenter-server/src/router/access_identity.rs`
- Create: `crates/presenter-server/src/router/bible/sermons.rs`, `crates/presenter-server/src/router/bible/sermons/tests.rs` (exempt)
- Modify: `crates/presenter-server/src/router.rs:1` (`mod access_identity;` before `mod ai;`) and `:442` (merge the sermon routes)
- Modify: `crates/presenter-server/src/router/bible/mod.rs:11` (`pub(super) mod sermons;` after `pub(super) mod resolve;`)

**Interfaces:**
- Consumes: Task 23's views; `AppError::{not_found, from}`; `build_router`; `local_today`.
- Produces: `pub(super) fn access_email(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String>` (in `router`); `pub(crate) fn router::bible::sermons::routes() -> Router<AppState>`; the private DTOs `SermonListDto`, `SermonSummaryDto`, `SermonDetailDto`, `SermonItemDto`, `SermonSlideDto`, `IdentityDto` and mappers `list_dto`, `item_dto`, `slide_dto`, `detail_dto`; `fn sermon_error(anyhow::Error) -> AppError`; `type Peer = Option<Extension<ConnectInfo<SocketAddr>>>`.

- [ ] **Step 1: Write the router tests** — `crates/presenter-server/src/router/bible/sermons/tests.rs`:

```rust
//! #851 P4a–P4d: the sermon API over HTTP (spec §11, §12): status codes, the
//! JSON shape (camelCase) and the author / client-IP wiring. The write
//! semantics are pinned at the state level (`state/sermons/tests.rs`).

use crate::router::build_router;
use crate::state::sermons::local_today;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse, VerseEdits};
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use presenter_persistence::{NewSermon, SermonItemRecord, SermonWrite};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;
use uuid::Uuid;

const HEB_11_1: &str = "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,";

/// An in-memory state with SEB Heb 11:1.
async fn seeded_state() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    let reference =
        BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, 1, 1).expect("reference");
    let passage = BiblePassage::new(reference, seb.clone(), HEB_11_1.to_string());
    let batch = BibleIngestionBatch::new(seb, vec![passage]).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
    state
}

async fn app() -> Router {
    build_router(seeded_state().await)
}

/// The app with one sermon dated today holding Heb 11:1 cut to `1a` with
/// "Viera" emphasized, its slides generated; returns the sermon id.
async fn app_with_sermon() -> (Router, String) {
    let state = seeded_state().await;
    let new = NewSermon {
        title: "Kázeň".to_string(),
        service_date: local_today(),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: None,
        author_email: None,
    };
    let created = state
        .repository()
        .create_sermon(&new)
        .await
        .expect("create");
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: vec![0],
        },
    );
    let item = SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: 1,
        verse_end: 1,
        source: vec![SourceVerse {
            verse: 1,
            text: HEB_11_1.to_string(),
        }],
        edits,
        slide_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let write = SermonWrite {
        expected_revision: 0,
        title: "Kázeň".to_string(),
        service_date: local_today(),
        secondary_translation_code: None,
        items: vec![item],
        slides: Vec::new(),
    };
    state
        .repository()
        .save_sermon_change(created.id, &write)
        .await
        .expect("seed write");
    state.regenerate_future_sermons(local_today()).await;
    (build_router(state), created.id.to_string())
}

/// Send `method uri` (JSON `body`, extra `headers`); the status and the JSON
/// body (`Value::Null` for an empty one).
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn the_reads_answer_camel_case_json() {
    let (app, id) = app_with_sermon().await;

    let (status, list) = call(&app, "GET", "/bible/sermons", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["today"], json!(local_today()));
    assert!(list["defaultServiceDate"].is_string());
    assert_eq!(list["sermons"][0]["id"], id.as_str());
    assert_eq!(list["sermons"][0]["itemCount"], 1);
    assert_eq!(list["sermons"][0]["slideCount"], 1);
    let (status, sermon) = call(&app, "GET", &format!("/bible/sermons/{id}"), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sermon["revision"], 2);
    assert_eq!(sermon["defaultTranslation"], "slk-seb");
    assert_eq!(sermon["secondaryTranslation"], Value::Null);
    let item = &sermon["items"][0];
    assert_eq!(item["label"], "Hebrejom 11:1a (SEB)");
    assert_eq!(item["book"], "Hebrejom");
    assert_eq!(item["source"][0]["text"], HEB_11_1);
    assert_eq!(item["edits"]["1"]["deleted"], json!([7, 8, 9, 10, 11]));
    assert_eq!(
        item["renderedText"],
        "1. VIERA je zárukou toho, v čo dúfame"
    );
    assert_eq!(item["sourceChanged"], false);
    let slide = &sermon["slides"][0];
    assert_eq!(slide["mainText"], "1. VIERA je zárukou toho, v čo dúfame");
    assert_eq!(slide["mainReference"], "Hebrejom 11:1a (SEB)");
    assert_eq!(slide["itemId"], item["id"]);
    assert_eq!(item["slideIds"][0], slide["id"]);
    let (status, today) = call(&app, "GET", "/bible/sermons/today", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(today[0]["id"], id.as_str());
}

#[tokio::test]
async fn an_unknown_sermon_is_404_and_a_bad_id_400() {
    let app = app().await;

    let unknown = format!("/bible/sermons/{}", Uuid::new_v4());
    let (status, _) = call(&app, "GET", &unknown, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/bible/sermons/not-a-uuid", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, today) = call(&app, "GET", "/bible/sermons/today", None, &[]).await;
    assert_eq!((status, today), (StatusCode::OK, json!([])));
}

#[tokio::test]
async fn identity_trusts_the_access_email_only_from_the_tunnel() {
    let app = app().await;
    let header = [("cf-access-authenticated-user-email", "preacher@example.org")];

    let (status, tunnel) = call(&app, "GET", "/bible/study/identity", None, &header).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tunnel, json!({ "email": "preacher@example.org" }));
    let mut lan = Request::builder()
        .uri("/bible/study/identity")
        .header(header[0].0, header[0].1)
        .body(Body::empty())
        .expect("request");
    let peer: SocketAddr = "192.0.2.7:40000".parse().expect("peer");
    lan.extensions_mut().insert(ConnectInfo(peer));
    let response = app.clone().oneshot(lan).await.expect("response");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(body, json!({ "email": null }));
}
```

- [ ] **Step 2: The identity rule with its unit tests** — `crates/presenter-server/src/router/access_identity.rs`:

```rust
//! The Cloudflare Access e-mail of a request (#851 spec §9): an author LABEL
//! for sermons, never a permission. Trusted only from a loopback peer
//! (`cloudflared` on the same box) or with no peer at all (tests): the exact
//! rule of `integrations::extract_client_ip` (#793), because a LAN client could
//! send the header itself. On the LAN and the church Wi-Fi bypass there is no
//! e-mail, and nothing depends on one.

use std::net::SocketAddr;

use axum::http::HeaderMap;

const ACCESS_EMAIL_HEADER: &str = "cf-access-authenticated-user-email";
const MAX_EMAIL_LEN: usize = 254;

/// The trimmed, lower-case e-mail; `None` from a LAN peer, without the header,
/// or when the value is no e-mail (no `@`, longer than 254 characters).
pub(super) fn access_email(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    if peer.is_some_and(|addr| !addr.ip().is_loopback()) {
        return None;
    }
    let email = headers
        .get(ACCESS_EMAIL_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .to_lowercase();
    (email.contains('@') && email.chars().count() <= MAX_EMAIL_LEN).then_some(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(email: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCESS_EMAIL_HEADER, email.parse().expect("header value"));
        headers
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(addr.parse().expect("socket address"))
    }

    #[test]
    fn the_tunnel_peer_and_no_peer_pass_the_email_lower_cased() {
        let header = headers("  Preacher@Example.org ");
        assert_eq!(
            access_email(&header, peer("127.0.0.1:40000")).as_deref(),
            Some("preacher@example.org")
        );
        assert_eq!(
            access_email(&header, None).as_deref(),
            Some("preacher@example.org")
        );
    }

    #[test]
    fn a_lan_peer_or_a_malformed_value_gives_none() {
        assert_eq!(
            access_email(&headers("preacher@example.org"), peer("192.0.2.7:40000")),
            None
        );
        assert_eq!(access_email(&headers("preacher"), None), None);
        let long = format!("{}@example.org", "a".repeat(250));
        assert_eq!(access_email(&headers(&long), None), None);
        assert_eq!(access_email(&HeaderMap::new(), None), None);
    }
}
```

- [ ] **Step 3: The read routes** — `crates/presenter-server/src/router/bible/sermons.rs`:

```rust
//! The sermon API (#851 spec §11) and `GET /bible/study/identity` (§9). JSON is
//! camelCase, query parameters snake_case, like the other Bible routes. The
//! reads come first (P4a); the writes carry `expectedRevision` and answer the
//! full sermon. `sermon_error` is the ONE place a sermon handler turns an error
//! into a response.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::BiblePresentationId;
use serde::Serialize;
use tracing::instrument;
use uuid::Uuid;

use super::super::access_identity::access_email;
use super::super::AppError;
use crate::state::sermons::{
    local_today, SermonDetail, SermonItemView, SermonList, SermonSlideView,
};
use crate::state::AppState;

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;
type SermonResult = Result<Json<SermonDetailDto>, AppError>;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/bible/sermons", get(list_sermons))
        .route("/bible/sermons/today", get(todays_sermons))
        .route("/bible/sermons/{id}", get(get_sermon))
        .route("/bible/study/identity", get(identity))
}

// ---- DTOs -----------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonListDto {
    today: NaiveDate,
    default_service_date: NaiveDate,
    sermons: Vec<SermonSummaryDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonSummaryDto {
    id: String,
    title: String,
    service_date: NaiveDate,
    author_email: Option<String>,
    item_count: usize,
    slide_count: usize,
    updated_at: DateTime<Utc>,
    revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonDetailDto {
    id: String,
    title: String,
    service_date: NaiveDate,
    author_email: Option<String>,
    default_translation: String,
    secondary_translation: Option<String>,
    revision: i64,
    items: Vec<SermonItemDto>,
    slides: Vec<SermonSlideDto>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonItemDto {
    id: String,
    order: usize,
    translation: String,
    book_code: String,
    book_number: u16,
    book: String,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
    source: Vec<SourceVerse>,
    edits: ItemEdits,
    label: String,
    rendered_text: String,
    source_changed: bool,
    slide_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonSlideDto {
    id: String,
    item_id: String,
    main_text: String,
    main_reference: String,
    secondary_text: String,
    secondary_reference: String,
}

#[derive(Debug, Serialize)]
struct IdentityDto {
    email: Option<String>,
}

// ---- Mapping ----------------------------------------------------------------

/// A failed sermon request (spec §12, logged at WARN with its cause), mapped
/// through the central `From<anyhow::Error> for AppError`.
fn sermon_error(err: anyhow::Error) -> AppError {
    tracing::warn!(
        target: "presenter::bible::sermon",
        error = %err,
        "sermon request refused"
    );
    AppError::from(err)
}

fn list_dto(list: SermonList) -> SermonListDto {
    SermonListDto {
        today: list.today,
        default_service_date: list.default_service_date,
        sermons: list
            .sermons
            .into_iter()
            .map(|summary| SermonSummaryDto {
                id: summary.id.to_string(),
                title: summary.title,
                service_date: summary.service_date,
                author_email: summary.author_email,
                item_count: summary.item_count,
                slide_count: summary.slide_count,
                updated_at: summary.updated_at,
                revision: summary.revision,
            })
            .collect(),
    }
}

fn item_dto(order: usize, view: SermonItemView) -> SermonItemDto {
    let SermonItemView {
        record,
        label,
        rendered_text,
        source_changed,
        slide_ids,
    } = view;
    SermonItemDto {
        id: record.id.to_string(),
        order,
        translation: record.translation_code,
        book_code: record.book_code,
        book_number: record.book_number,
        book: record.book_name,
        chapter: record.chapter,
        verse_start: record.verse_start,
        verse_end: record.verse_end,
        source: record.source,
        edits: record.edits,
        label,
        rendered_text,
        source_changed,
        slide_ids: slide_ids.iter().map(ToString::to_string).collect(),
    }
}

fn slide_dto(view: SermonSlideView) -> SermonSlideDto {
    SermonSlideDto {
        id: view.slide.id.to_string(),
        item_id: view.item_id.to_string(),
        main_text: view.slide.main.value().to_string(),
        main_reference: view.slide.main_reference,
        secondary_text: view.slide.secondary.value().to_string(),
        secondary_reference: view.slide.secondary_reference,
    }
}

fn detail_dto(detail: SermonDetail) -> SermonDetailDto {
    let SermonDetail {
        sermon,
        items,
        slides,
        warnings,
    } = detail;
    SermonDetailDto {
        id: sermon.id.to_string(),
        title: sermon.title,
        service_date: sermon.service_date,
        author_email: sermon.author_email,
        default_translation: sermon.default_translation_code,
        secondary_translation: sermon.secondary_translation_code,
        revision: sermon.revision,
        items: items
            .into_iter()
            .enumerate()
            .map(|(order, item)| item_dto(order, item))
            .collect(),
        slides: slides.into_iter().map(slide_dto).collect(),
        warnings,
    }
}

// ---- Handlers -----------------------------------------------------------------

#[instrument(skip_all)]
async fn list_sermons(State(state): State<AppState>) -> Result<Json<SermonListDto>, AppError> {
    let list = state
        .list_sermons(local_today())
        .await
        .map_err(sermon_error)?;
    Ok(Json(list_dto(list)))
}

#[instrument(skip_all)]
async fn todays_sermons(
    State(state): State<AppState>,
) -> Result<Json<Vec<SermonDetailDto>>, AppError> {
    let details = state
        .sermons_on(local_today())
        .await
        .map_err(sermon_error)?;
    Ok(Json(details.into_iter().map(detail_dto).collect()))
}

#[instrument(skip_all)]
async fn get_sermon(State(state): State<AppState>, Path(id): Path<Uuid>) -> SermonResult {
    let detail = state
        .sermon_detail(BiblePresentationId::from_uuid(id))
        .await
        .map_err(sermon_error)?
        .ok_or_else(|| AppError::not_found("sermon not found"))?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn identity(headers: HeaderMap, peer: Peer) -> Json<IdentityDto> {
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    Json(IdentityDto {
        email: access_email(&headers, peer),
    })
}

#[cfg(test)]
mod tests;
```

`router.rs`: add `mod access_identity;` as the first module line; replace `        .merge(stream_playback::routes());` with

```rust
        .merge(stream_playback::routes())
        // #851: sermons + the Access identity (`/bible/sermons*`,
        // `/bible/study/identity`); static `/today` wins over `/{id}`.
        .merge(bible::sermons::routes());
```

`router/bible/mod.rs`: add `pub(super) mod sermons;` after `pub(super) mod resolve;`.

- [ ] **Step 4: How it is verified.** CI: the three router tests (camelCase JSON incl. `items[0].edits["1"].deleted`, `slides[0].itemId == items[0].id`, `items[0].slideIds[0] == slides[0].id`; 404 unknown / 400 bad UUID / `[]` today; identity trusted without a peer, `null` from a `ConnectInfo(192.0.2.7)` peer) and the two `access_identity` unit tests. `GET /bible/sermons` is not timer-polled (no quiet-path entry needed, observability rule).

- [ ] **Step 5: Commit** — paths `crates/presenter-server/src/router/access_identity.rs crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router/bible/sermons/tests.rs crates/presenter-server/src/router.rs crates/presenter-server/src/router/bible/mod.rs`, message `feat(server): sermon read API and the Access identity (#851 P4a)` + body "GET /bible/sermons, /bible/sermons/today, /bible/sermons/{id}, /bible/study/identity; the Access e-mail is trusted only from a loopback peer (the #793 rule)." + Co-Authored-By.

### Task 25: Rule section, ship P4a

- [ ] **Step 1: Append** to `.claude/rules/sermon-prep.md`:

```markdown

## The API (P4a–P4d, `router/bible/sermons.rs`, `state/sermons/{view,write,snapshot,maintain,guard}.rs`)

- JSON camelCase, query parameters snake_case (the one exception, `DELETE …/items/{id}?expectedRevision=`,
  accepts `expected_revision` too). Every write carries `expectedRevision` and answers the full
  `SermonDetail`; `warnings` is filled only on a write answer.
- **`sermon_error` is the ONE place a sermon handler turns an error into a response** — every
  state call ends `.map_err(sermon_error)?`: WARN log with the cause (spec §12), then
  `SermonRefusal` → 422 (from P4b) and everything else through the central
  `From<anyhow::Error> for AppError` (404 `NotFound`, 409 `Conflict`, 502/503 NLT, else 500).
  A 400 is decided in the handler (`translation or refreshSource is required`); axum's own
  rejections give 400 (bad UUID path) / 422 (bad JSON).
- `sourceChanged` (view.rs) = some snapshot verse differs from the database text now, or is
  gone (read through `bible_passage_range`). A re-import never regenerates sermon slides.
- `router/access_identity.rs` `access_email(headers, peer)`: `Cf-Access-Authenticated-User-Email`
  only from a loopback peer (cloudflared) or no peer (tests) — the #793 `extract_client_ip`
  rule; trimmed, lower case, must contain `@`, ≤ 254 chars. A LABEL (stored as `author_email`
  on create, logged on every write), never a permission. `GET /bible/study/identity` →
  `{"email": … | null}` (lives here until D4 creates `router/bible/study.rs`).
- Router tests drive `build_router(state)` with `oneshot`; a LAN peer is simulated by inserting
  `ConnectInfo(SocketAddr)` into the request extensions.
```

Commit — `docs(playbook): sermon API rules (#851 P4a) [no-test: docs only]`.

- [ ] **Step 2: Local gates** with `<files>` = `crates/presenter-server/src/state/sermons/view.rs crates/presenter-server/src/state/sermons/mod.rs crates/presenter-server/src/router/access_identity.rs crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router.rs crates/presenter-server/src/router/bible/mod.rs`. Expected: `router.rs` 765, `sermons.rs` ≈ 260 production lines.

- [ ] **Step 3: Ship** with `<V>` = 0.4.323, `<title>` = "Sermon read API and Access identity (#851 P4a)".

- [ ] **Step 4: Functional check, SNV then PP:**

```bash
curl -fsS http://10.77.9.205/bible/sermons
curl -fsS http://10.77.9.205/bible/sermons/today
curl -fsS -H 'Cf-Access-Authenticated-User-Email: someone@example.org' http://10.77.9.205/bible/study/identity
```

Expected: `{"today":"<SNV's date>","defaultServiceDate":"<next Sunday>","sermons":[]}`, `[]`, and `{"email":null}` — dev2 is a LAN peer, so the spoofed header is ignored (the trust rule, live). Same on `companion-pp.lan`.

- [ ] **Step 5:** `playbook-review`; comment on #851.

---

## PR unit P4b — create and delete sermons, the slide guard, `sermonDate` (v0.4.324)

Spec §7.6 create defaults, §11 `POST /bible/sermons` + `DELETE /bible/sermons/{id}`, §8.2 server guards (sermon slides read-only on the presentation routes, rename/delete allowed), §7.1 `sermon_date` on the wire, §9 author label, §13 INFO line for create/delete.

### Task 26: Post-release version bump to 0.4.324

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.323)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.323 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.324 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.324 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 27: State — create, delete, `SermonRefusal`, the guard

**Files:**
- Create: `crates/presenter-server/src/state/sermons/write.rs`, `crates/presenter-server/src/state/sermons/guard.rs`
- Modify: `crates/presenter-server/src/state/sermons/commit.rs` (`SermonLine` usable by `write.rs`), `mod.rs` (modules, re-export, `SermonRefusal`, two `SermonOp` variants), `tests.rs` (append)
- Modify: `crates/presenter-server/src/state/bible.rs:267, :288, :324, :375` (guard first in the four slide-write methods)

**Interfaces:**
- Consumes: `Repository::create_sermon`, `fetch_sermon`, `delete_bible_presentation`, `is_sermon`; `default_service_date`, `default_sermon_title`; `bible_remote::is_remote_translation`; `log_sermon_write`.
- Produces: `pub(crate) struct SermonRefusal(pub(crate) String)` (+ `fn new(impl Into<String>)`, `thiserror`, `Display` = the message); `SermonOp::{Create, Delete}`; `pub(crate) struct SermonDraft { title: Option<String>, service_date: Option<NaiveDate>, default_translation: Option<String>, secondary_translation: Option<Option<String>> }` (Default); `pub(super) fn installed(&[BibleTranslation], code: &str, allow_remote: bool) -> Result<String, SermonRefusal>`; `pub(super) async fn AppState::load_sermon(BiblePresentationId) -> anyhow::Result<SermonSnapshot>`; `pub(crate) async fn create_sermon(SermonDraft, today: NaiveDate, &SermonOrigin) -> anyhow::Result<SermonDetail>`; `pub(crate) async fn delete_sermon(BiblePresentationId, &SermonOrigin) -> anyhow::Result<()>`; `pub(super) const SERMON_SLIDES_REFUSAL: &str`; `pub(crate) async fn refuse_if_sermon(BiblePresentationId) -> anyhow::Result<()>`; `SermonLine::new(op, sermon_id, origin, started)` with `pub(super)` fields.

- [ ] **Step 1: Append the tests** to `tests.rs`:

```rust

// ---- #851 P4b: create, delete, the slide guard -------------------------------

fn preacher() -> SermonOrigin {
    SermonOrigin {
        author: Some("preacher@example.org".to_string()),
        client_ip: "203.0.113.9".to_string(),
    }
}

fn refusal(err: &anyhow::Error) -> Option<String> {
    err.downcast_ref::<SermonRefusal>()
        .map(|refusal| refusal.0.clone())
}

/// A new SEB sermon for today with no secondary line.
async fn new_sermon(state: &AppState) -> SermonDetail {
    let draft = SermonDraft {
        service_date: Some(local_today()),
        default_translation: Some("slk-seb".to_string()),
        secondary_translation: Some(None),
        ..SermonDraft::default()
    };
    state
        .create_sermon(draft, local_today(), &preacher())
        .await
        .expect("create")
}

#[tokio::test]
async fn create_takes_the_server_dates_and_the_bible_preferences() {
    let state = hebrews().await;
    state
        .set_bible_preferences(
            BiblePreferences::default()
                .with_main_translation(Some("slk-seb".to_string()))
                .with_secondary_translation(Some("eng-kjv".to_string())),
        )
        .await
        .expect("preferences");

    let created = state
        .create_sermon(SermonDraft::default(), date("2026-10-14"), &preacher())
        .await
        .expect("create");

    let sermon = &created.sermon;
    assert_eq!(sermon.service_date, date("2026-10-18"));
    assert_eq!(sermon.title, "Kázeň 18. 10. 2026");
    assert_eq!(sermon.default_translation_code, "slk-seb");
    assert_eq!(
        sermon.secondary_translation_code.as_deref(),
        Some("eng-kjv")
    );
    assert_eq!(sermon.author_email.as_deref(), Some("preacher@example.org"));
    assert_eq!(sermon.revision, 0);
    let nlt = SermonDraft {
        default_translation: Some(NLT_CODE.to_string()),
        ..SermonDraft::default()
    };
    let err = state
        .create_sermon(nlt, date("2026-10-14"), &preacher())
        .await
        .expect_err("the NLT is never an item translation");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("NLT sa do kázne neukladá — vyber iný preklad")
    );
}

#[tokio::test]
async fn deleting_only_deletes_sermons_and_logs_with_the_author() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let plain = state
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain presentation");
    let capture = Capture::install();

    let err = state
        .delete_sermon(plain.id, &preacher())
        .await
        .expect_err("a plain presentation is no sermon");
    state.delete_sermon(id, &preacher()).await.expect("delete");

    assert!(matches!(
        err.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::NotFound(_))
    ));
    assert!(state.sermon_detail(id).await.expect("detail").is_none());
    let writes: Vec<_> = capture
        .events("presenter::bible::sermon")
        .into_iter()
        .filter(|line| line.field("message") == Some("sermon write"))
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].field("op"), Some(quoted("delete").as_str()));
    assert_eq!(
        writes[0].field("author"),
        Some(quoted("preacher@example.org").as_str())
    );
    assert_eq!(
        writes[0].field("client_ip"),
        Some(quoted("203.0.113.9").as_str())
    );
    assert_eq!(writes[0].field("sermon_id"), Some(id.to_string().as_str()));
}

#[tokio::test]
async fn sermon_slides_are_read_only_on_the_presentation_paths() {
    let state = hebrews().await;
    let id = stored_sermon(&state, local_today(), vec![item(1..=3, &[])]).await;
    state.regenerate_future_sermons(local_today()).await;
    let slides = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon")
        .slides;
    let slide_id = slides[0].slide.id;

    let refusals = [
        state
            .append_bible_presentation_slides(id, Vec::new())
            .await
            .map(|_| ()),
        state.delete_bible_slide(id, slide_id).await.map(|_| ()),
        state
            .reorder_bible_slides(id, vec![slide_id])
            .await
            .map(|_| ()),
        state
            .update_bible_slide(
                id,
                slide_id,
                "x".into(),
                "y".into(),
                String::new(),
                String::new(),
                None,
            )
            .await
            .map(|_| ()),
    ];

    for refused in refusals {
        let err = refused.expect_err("a sermon's slides are generated");
        assert!(matches!(
            err.downcast_ref::<presenter_persistence::RepositoryError>(),
            Some(presenter_persistence::RepositoryError::Conflict(
                super::guard::SERMON_SLIDES_REFUSAL
            ))
        ));
    }
    state
        .rename_bible_presentation(id, "Nový názov")
        .await
        .expect("rename stays allowed");
    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!(detail.sermon.title, "Nový názov");
    assert_eq!(detail.slides.len(), slides.len());
}
```

- [ ] **Step 2: Implement.** `crates/presenter-server/src/state/sermons/write.rs` (P4c adds the item writes):

```rust
//! The sermon writes (#851 spec §7.5, §11). This file starts with create and
//! delete; the item writes (`P4c`) load the sermon, refuse a stale
//! `expected_revision` with 409, compute the new item list in memory and commit
//! it through `commit_sermon`.

use std::time::Instant;

use chrono::NaiveDate;
use presenter_core::bible::sermon::{default_sermon_title, default_service_date};
use presenter_core::{BiblePresentationId, BibleTranslation};
use presenter_persistence::{NewSermon, RepositoryError, SermonSnapshot, SERMON_NOT_FOUND};

use super::super::AppState;
use super::commit::{log_sermon_write, SermonLine};
use super::view::SermonDetail;
use super::{SermonOp, SermonOrigin, SermonRefusal};
use crate::bible_remote;
use crate::live::LiveEvent;

/// `POST /bible/sermons`: every field optional (spec §7.6 defaults).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SermonDraft {
    pub(crate) title: Option<String>,
    pub(crate) service_date: Option<NaiveDate>,
    pub(crate) default_translation: Option<String>,
    /// `None` = the Bible preference's secondary; `Some(None)` = no line.
    pub(crate) secondary_translation: Option<Option<String>>,
}

/// An installed translation's own code; the NLT only when `allow_remote`.
pub(super) fn installed(
    translations: &[BibleTranslation],
    code: &str,
    allow_remote: bool,
) -> Result<String, SermonRefusal> {
    if !allow_remote && bible_remote::is_remote_translation(code) {
        return Err(SermonRefusal::new(
            "NLT sa do kázne neukladá — vyber iný preklad",
        ));
    }
    translations
        .iter()
        .find(|translation| translation.code.eq_ignore_ascii_case(code.trim()))
        .map(|translation| translation.code.clone())
        .ok_or_else(|| SermonRefusal::new(format!("Preklad {code} nie je nainštalovaný")))
}

impl AppState {
    pub(super) async fn load_sermon(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<SermonSnapshot> {
        Ok(self
            .repository
            .fetch_sermon(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?)
    }

    /// A new, empty sermon (revision 0) with the spec §7.6 defaults: the next
    /// Sunday, "Kázeň D. M. YYYY", the Bible preference's translations.
    pub(crate) async fn create_sermon(
        &self,
        draft: SermonDraft,
        today: NaiveDate,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let started = Instant::now();
        let preferences = self.get_bible_preferences().await?;
        let translations = self.list_bible_translations().await?;
        let default_translation = match draft.default_translation.as_deref() {
            Some(code) => installed(&translations, code, false)?,
            None => preferences
                .main_translation
                .as_deref()
                .and_then(|code| installed(&translations, code, false).ok())
                .or_else(|| {
                    translations
                        .iter()
                        .find(|t| !bible_remote::is_remote_translation(&t.code))
                        .map(|t| t.code.clone())
                })
                .ok_or_else(|| SermonRefusal::new("Žiadny preklad nie je nainštalovaný"))?,
        };
        let secondary = match draft.secondary_translation {
            Some(Some(code)) => Some(installed(&translations, &code, true)?),
            Some(None) => None,
            None => preferences
                .secondary_translation
                .as_deref()
                .and_then(|code| installed(&translations, code, true).ok()),
        };
        let service_date = draft
            .service_date
            .unwrap_or_else(|| default_service_date(today));
        let title = match draft
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(title) => title.to_string(),
            None => default_sermon_title(service_date),
        };
        let sermon = self
            .repository
            .create_sermon(&NewSermon {
                title,
                service_date,
                default_translation_code: default_translation,
                secondary_translation_code: secondary,
                author_email: origin.author.clone(),
            })
            .await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: sermon.id.to_string(),
        });
        log_sermon_write(&SermonLine::new(
            SermonOp::Create,
            sermon.id,
            origin,
            started,
        ));
        self.sermon_detail(sermon.id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(SERMON_NOT_FOUND).into())
    }

    /// Delete the sermon's presentation; the sermon and its items cascade.
    /// Only a sermon: a plain presentation is not found here.
    pub(crate) async fn delete_sermon(
        &self,
        id: BiblePresentationId,
        origin: &SermonOrigin,
    ) -> anyhow::Result<()> {
        let started = Instant::now();
        let snapshot = self.load_sermon(id).await?;
        self.repository.delete_bible_presentation(id).await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        let mut line = SermonLine::new(SermonOp::Delete, id, origin, started);
        line.revision = snapshot.sermon.revision;
        line.items = snapshot.items.len();
        log_sermon_write(&line);
        Ok(())
    }
}
```

`crates/presenter-server/src/state/sermons/guard.rs`:

```rust
//! Sermon slides are generated from the items (#851 spec §8.2): the slide write
//! paths of a Bible presentation (append, edit, delete, reorder; HTTP and the AI
//! tools alike, both call the same `AppState` methods) refuse a sermon's
//! presentation with 409. Renaming (= the sermon title) and deleting it stay
//! allowed.

use presenter_core::BiblePresentationId;
use presenter_persistence::RepositoryError;

use super::super::AppState;

pub(super) const SERMON_SLIDES_REFUSAL: &str =
    "Slajdy kázne sa generujú z jej položiek — upravuje sa v /ui/study";

impl AppState {
    /// 409 (`RepositoryError::Conflict`, central mapping) when the presentation
    /// is a sermon.
    pub(crate) async fn refuse_if_sermon(
        &self,
        presentation_id: BiblePresentationId,
    ) -> anyhow::Result<()> {
        if self.repository.is_sermon(presentation_id).await? {
            return Err(RepositoryError::Conflict(SERMON_SLIDES_REFUSAL).into());
        }
        Ok(())
    }
}
```

`commit.rs` — replace the `SermonLine` block

```rust
/// The fields of one `presenter::bible::sermon` line (spec §13).
struct SermonLine<'a> {
    op: SermonOp,
    sermon_id: BiblePresentationId,
    item_id: Option<Uuid>,
    revision: i64,
    items: usize,
    slides: usize,
    origin: &'a SermonOrigin,
    elapsed: Duration,
    warning: Option<&'a str>,
}
```

with

```rust
/// The fields of one `presenter::bible::sermon` line (spec §13).
pub(super) struct SermonLine<'a> {
    pub(super) op: SermonOp,
    pub(super) sermon_id: BiblePresentationId,
    pub(super) item_id: Option<Uuid>,
    pub(super) revision: i64,
    pub(super) items: usize,
    pub(super) slides: usize,
    pub(super) origin: &'a SermonOrigin,
    pub(super) elapsed: Duration,
    pub(super) warning: Option<&'a str>,
}

impl<'a> SermonLine<'a> {
    /// A line for a write outside `commit_sermon` (create, delete): no item,
    /// revision 0, no items or slides until the caller fills them in.
    pub(super) fn new(
        op: SermonOp,
        sermon_id: BiblePresentationId,
        origin: &'a SermonOrigin,
        started: Instant,
    ) -> Self {
        Self {
            op,
            sermon_id,
            item_id: None,
            revision: 0,
            items: 0,
            slides: 0,
            origin,
            elapsed: started.elapsed(),
            warning: None,
        }
    }
}
```

and `fn log_sermon_write(` with `pub(super) fn log_sermon_write(`.

`mod.rs` — the whole file after this task:

```rust
//! Sermon preparation on the server (#851 spec §7). A write loads the sermon,
//! computes the new item list in memory, generates the slides from the items
//! (`generate.rs`) and commits items + slides in ONE repository transaction
//! guarded by the expected revision (`commit.rs`): a stale write is
//! `RepositoryError::Conflict` → 409 and is never half-applied. Then
//! `LiveEvent::BibleSlidesChanged` is published and one INFO
//! `presenter::bible::sermon` line is logged. Item text is a snapshot, so a
//! Bible re-import never changes a sermon (spec §5.7).

mod commit;
mod generate;
mod guard;
mod view;
mod write;

pub(crate) use view::{SermonDetail, SermonItemView, SermonList, SermonSlideView};
pub(crate) use write::SermonDraft;

use chrono::NaiveDate;

/// Who wrote (spec §13): the Cloudflare Access e-mail when the request carried
/// one, and the client IP by the #793 rule. Both are labels, never
/// permissions (spec §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonOrigin {
    pub(crate) author: Option<String>,
    pub(crate) client_ip: String,
}

impl SermonOrigin {
    /// A write the server makes on its own (the character-limit pass).
    pub(crate) fn system() -> Self {
        Self {
            author: None,
            client_ip: "system".to_string(),
        }
    }
}

/// The `op` field of the `presenter::bible::sermon` line (spec §13). A unit
/// adds its variant together with the write that constructs it (`dead_code`
/// under `-D warnings` otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SermonOp {
    Create,
    Delete,
    Regenerate,
}

impl SermonOp {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Regenerate => "regenerate",
        }
    }
}

/// A sermon request the server refuses with 422 and this Slovak message (an
/// invalid range, the NLT as an item translation, a bad token index …). Decided
/// in the state layer, so it is its own type (repository-error-pattern rule:
/// never forced through `RepositoryError`); `router/bible/sermons.rs` maps it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct SermonRefusal(pub(crate) String);

impl SermonRefusal {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The server-local date: "today" for `/bible/sermons/today`, the default
/// service date and the character-limit pass. The only clock read of the
/// sermon code; everything below it takes the date as a parameter.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests;
```

`state/bible.rs` — the FIRST statement of each of the four slide-write methods: in `append_bible_presentation_slides` (before its `// Note: empty slides are permitted` comment)

```rust
        // #851 §8.2: a sermon's slides are generated from its items.
        self.refuse_if_sermon(id).await?;
```

and `self.refuse_if_sermon(presentation_id).await?;` at the top of `delete_bible_slide`, `reorder_bible_slides` and `update_bible_slide`. The AI tools (`ai/tools/bible_presentation.rs` append + delete) call these same methods and get the same 409.

- [ ] **Step 3: How it is verified.** CI: create takes the server's date (2026-10-14 → 2026-10-18, "Kázeň 18. 10. 2026") and the Bible preferences' translations, refuses the NLT as default translation (422 text); delete refuses a plain presentation (404) and logs one `op="delete"` line with author and client IP; the four slide paths answer `Conflict(SERMON_SLIDES_REFUSAL)` while rename works and becomes the sermon title.

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/state/sermons/ crates/presenter-server/src/state/bible.rs`, message `feat(server): create and delete sermons; sermon slides are read-only elsewhere (#851 P4b)` + Co-Authored-By.

### Task 28: HTTP — `POST /bible/sermons`, `DELETE /bible/sermons/{id}`, `sermonDate`

**Files:**
- Modify: `crates/presenter-server/src/router/bible/sermons.rs` (imports, routes, one request DTO, `double_option`, `origin`, `sermon_error`, two handlers), `sermons/tests.rs` (append)
- Modify: `crates/presenter-server/src/router/bible/presentations.rs:21-35` (the two presentation DTOs) and `:92-111` (their mappers)

**Interfaces:**
- Consumes: `create_sermon`, `delete_sermon`, `SermonDraft`, `SermonOrigin`, `SermonRefusal`; `access_email`; `integrations::extract_client_ip(&HeaderMap, Option<SocketAddr>) -> String` (`pub(super)` in `router::integrations`, visible to `router::bible::sermons`).
- Produces: `fn origin(&HeaderMap, Peer) -> SermonOrigin`; `sermon_error` maps `SermonRefusal` → 422; `CreateSermonRequest { title?, serviceDate?, defaultTranslation?, secondaryTranslation? (null = none) }`; `sermonDate` on `BiblePresentationSummaryDto` / `BiblePresentationDetailDto` (skipped when `None`).

- [ ] **Step 1: Append the router tests** to `router/bible/sermons/tests.rs`:

```rust

// ---- #851 P4b: create, delete, the slide guard, `sermonDate` -------------------

async fn new_sermon(app: &Router) -> Value {
    let body = json!({ "serviceDate": local_today(), "defaultTranslation": "slk-seb" });
    let (status, sermon) = call(app, "POST", "/bible/sermons", Some(body), &[]).await;
    assert_eq!(status, StatusCode::OK, "{sermon}");
    sermon
}

#[tokio::test]
async fn create_records_the_author_and_delete_is_204_then_404() {
    let app = app().await;
    let headers = [
        ("cf-access-authenticated-user-email", "Preacher@Example.org"),
        ("x-forwarded-for", "203.0.113.9"),
    ];
    let body = json!({ "serviceDate": local_today(), "defaultTranslation": "slk-seb",
        "secondaryTranslation": null });

    let (status, created) = call(&app, "POST", "/bible/sermons", Some(body), &headers).await;

    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["authorEmail"], "preacher@example.org");
    assert_eq!(created["revision"], 0);
    assert_eq!(created["items"], json!([]));
    let uri = format!("/bible/sermons/{}", created["id"].as_str().expect("id"));
    let (status, _) = call(&app, "DELETE", &uri, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", &uri, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let nlt = json!({ "defaultTranslation": "eng-nlt" });
    let (status, refused) = call(&app, "POST", "/bible/sermons", Some(nlt), &[]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused["message"],
        "NLT sa do kázne neukladá — vyber iný preklad"
    );
}

#[tokio::test]
async fn a_sermons_slides_refuse_the_presentation_slide_routes() {
    let (app, id) = app_with_sermon().await;
    let (_, sermon) = call(&app, "GET", &format!("/bible/sermons/{id}"), None, &[]).await;
    let slide_id = sermon["slides"][0]["id"]
        .as_str()
        .expect("slide id")
        .to_string();
    let slide_uri = format!("/bible/presentations/{id}/slides/{slide_id}");
    let refused = [
        (
            "POST",
            format!("/bible/presentations/{id}/append"),
            Some(json!({ "slides": [{ "bibleMain": "x", "bibleTranslation": "" }] })),
        ),
        (
            "PATCH",
            slide_uri.clone(),
            Some(json!({ "bibleMain": "x", "bibleTranslation": "",
                "bibleMainReference": "", "bibleTranslationReference": "" })),
        ),
        ("DELETE", slide_uri, None),
        (
            "POST",
            format!("/bible/presentations/{id}/slides/reorder"),
            Some(json!({ "slideIds": [slide_id] })),
        ),
    ];

    for (method, uri, body) in refused {
        let (status, answer) = call(&app, method, &uri, body, &[]).await;
        assert_eq!(status, StatusCode::CONFLICT, "{method} {uri}: {answer}");
        assert_eq!(
            answer["message"],
            "Slajdy kázne sa generujú z jej položiek — upravuje sa v /ui/study"
        );
    }
    let rename = json!({ "name": "Premenovaná" });
    let uri = format!("/bible/presentations/{id}");
    let (status, _) = call(&app, "PATCH", &uri, Some(rename), &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "rename stays allowed");
    let (_, list) = call(&app, "GET", "/bible/presentations", None, &[]).await;
    assert_eq!(list[0]["name"], "Premenovaná");
    assert_eq!(list[0]["sermonDate"], json!(local_today()));
    let (_, detail) = call(&app, "GET", &uri, None, &[]).await;
    assert_eq!(detail["sermonDate"], json!(local_today()));
}
```

- [ ] **Step 2: Implement** in `router/bible/sermons.rs`. Imports: `use axum::http::HeaderMap;` → `use axum::http::{HeaderMap, StatusCode};`, `use serde::Serialize;` → `use serde::{Deserialize, Deserializer, Serialize};`, add `use super::super::integrations::extract_client_ip;` after the `access_email` import, and the `crate::state::sermons` import becomes:

```rust
use crate::state::sermons::{
    local_today, SermonDetail, SermonDraft, SermonItemView, SermonList, SermonOrigin,
    SermonRefusal, SermonSlideView,
};
```

`routes()`:

```rust
        .route("/bible/sermons", get(list_sermons).post(create_sermon))
        .route("/bible/sermons/today", get(todays_sermons))
        .route("/bible/sermons/{id}", get(get_sermon).delete(delete_sermon))
```

Insert before `// ---- Mapping`:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSermonRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    service_date: Option<NaiveDate>,
    #[serde(default)]
    default_translation: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    secondary_translation: Option<Option<String>>,
}

/// serde double-`Option`: an ABSENT field is `None`, an explicit `null` is
/// `Some(None)` ("no secondary line"), a value is `Some(Some(v))`.
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}
```

Replace the P4a `sermon_error` (doc comment included) with:

```rust
/// The author label and client IP of a write (spec §9, §13).
fn origin(headers: &HeaderMap, peer: Peer) -> SermonOrigin {
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    SermonOrigin {
        author: access_email(headers, peer),
        client_ip: extract_client_ip(headers, peer),
    }
}

/// A refused sermon request (spec §12, logged at WARN): `SermonRefusal` → 422
/// with its Slovak message, everything else through the central mapping.
fn sermon_error(err: anyhow::Error) -> AppError {
    tracing::warn!(
        target: "presenter::bible::sermon",
        error = %err,
        "sermon request refused"
    );
    match err.downcast_ref::<SermonRefusal>() {
        Some(refusal) => AppError::unprocessable(refusal.0.clone()),
        None => AppError::from(err),
    }
}
```

Insert before `async fn todays_sermons(`'s `#[instrument(skip_all)]`:

```rust
#[instrument(skip_all)]
async fn create_sermon(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<CreateSermonRequest>,
) -> SermonResult {
    let draft = SermonDraft {
        title: body.title,
        service_date: body.service_date,
        default_translation: body.default_translation,
        secondary_translation: body.secondary_translation,
    };
    let detail = state
        .create_sermon(draft, local_today(), &origin(&headers, peer))
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}
```

Insert before `async fn identity(`'s `#[instrument(skip_all)]`:

```rust
#[instrument(skip_all)]
async fn delete_sermon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<StatusCode, AppError> {
    state
        .delete_sermon(BiblePresentationId::from_uuid(id), &origin(&headers, peer))
        .await
        .map_err(sermon_error)?;
    Ok(StatusCode::NO_CONTENT)
}
```

`router/bible/presentations.rs`: add `use chrono::NaiveDate;`; append to both `BiblePresentationSummaryDto` and `BiblePresentationDetailDto`:

```rust
    /// The service date when the presentation is a sermon (#851 §7.1, §8.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    sermon_date: Option<NaiveDate>,
```

and set it in the two literals: `sermon_date: presentation.sermon_date,` in `bible_presentation_to_detail`, `sermon_date: s.sermon_date,` in `list_bible_presentations` (both structs are only built there — grepped).

- [ ] **Step 3: How it is verified.** CI: create with the Access header + `x-forwarded-for` and no socket peer → `authorEmail` lower case, revision 0, `items: []`; delete → 204, then 404; the NLT as default translation → 422 with the Slovak text; the four presentation slide routes on a sermon → 409 with the guard text; rename → 204 and the list/detail carry `"sermonDate":"<today>"`. (The append route refuses an empty `slides` list with 400 before the state call, so the test sends one slide.)

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router/bible/sermons/tests.rs crates/presenter-server/src/router/bible/presentations.rs`, message `feat(server): create and delete sermons over HTTP, sermonDate on presentations (#851 P4b)` + Co-Authored-By.

### Task 29: Rule section, ship P4b, first live sermon writes on SNV

- [ ] **Step 1: Append** to `.claude/rules/sermon-prep.md`:

```markdown
- **`SermonRefusal(String)`** (state layer, `thiserror`) = 422 with a Slovak message: an invalid
  range, more than 60 verses, the NLT as an item or default translation, an unknown translation
  or book, verses the translation lacks, a bad token index, an empty title. Never forced through
  `RepositoryError` (repository-error-pattern rule).
- **Sermon slides are read-only on the presentation routes** (spec §8.2): `refuse_if_sermon`
  (`guard.rs`) runs FIRST in `append_bible_presentation_slides`, `delete_bible_slide`,
  `reorder_bible_slides`, `update_bible_slide` → `Conflict(SERMON_SLIDES_REFUSAL)` → 409 "Slajdy
  kázne sa generujú z jej položiek — upravuje sa v /ui/study"; the AI tools call the same state
  methods and get the same refusal. Rename (= the sermon title) and delete stay allowed. A new
  slide-write state method calls the guard first.
- `DELETE /bible/sermons/{id}` deletes only a sermon (a plain presentation is 404 there).
- The presentation DTOs carry `sermonDate` (skipped when `None`), so the operator UI (H1) can
  render sermons read-only.
```

Commit — `docs(playbook): sermon create/delete and slide guard rules (#851 P4b) [no-test: docs only]`.

- [ ] **Step 2: Local gates** with `<files>` = `crates/presenter-server/src/state/sermons/write.rs crates/presenter-server/src/state/sermons/guard.rs crates/presenter-server/src/state/sermons/commit.rs crates/presenter-server/src/state/sermons/mod.rs crates/presenter-server/src/state/bible.rs crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router/bible/presentations.rs`. Expected: `state/bible.rs` ≈ 779; `create_sermon` ≈ 65 lines (warn band starts at 80).

- [ ] **Step 3: Ship** with `<V>` = 0.4.324, `<title>` = "Create and delete sermons, slide guard, sermonDate (#851 P4b)".

- [ ] **Step 4: Functional check on SNV** (prod testing is standing-approved; the test sermon is named to sort last and is deleted at once):

```bash
ID=$(curl -fsS -X POST http://10.77.9.205/bible/sermons -H 'content-type: application/json' \
  -d '{"title":"zz-test-851","defaultTranslation":"slk-seb","secondaryTranslation":null}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
curl -fsS http://10.77.9.205/bible/presentations | python3 -c "import json,sys; print([p for p in json.load(sys.stdin) if p['id']=='$ID'])"
curl -fsS -o /dev/null -w '%{http_code}\n' -X DELETE "http://10.77.9.205/bible/sermons/$ID"
ssh -o BatchMode=yes newlevel@presenter.lan "journalctl -u presenter --since '-5 min' --no-pager | grep 'presenter::bible::sermon'"
```

Expected: the presentation row carries `"sermonDate":"<next Sunday>"`; `204`; two INFO "sermon write" lines, `op="create"` and `op="delete"`, `author="-"` (LAN), `client_ip="10.77.8.134"` (T8 for these ops). PP: `GET /bible/sermons` → `{"sermons":[]…}` after the release.

- [ ] **Step 5:** `playbook-review`; comment on #851.

---

## PR unit P4c — add and edit items (v0.4.325)

Spec §7.1 (one contiguous range of one chapter, ≤ 60 verses, NLT never an item translation, the DB text snapshotted), §7.2 (edits validated, indices out of range → 422), §7.5 (`expectedRevision` → 409), §11 `POST /bible/sermons/{id}/items`, `PUT /bible/sermons/{id}/items/{itemId}/edits`, §12.

### Task 30: Post-release version bump to 0.4.325

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.324)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.324 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.325 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.325 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 31: State — snapshot an item, add it, edit it

**Files:**
- Create: `crates/presenter-server/src/state/sermons/snapshot.rs`
- Modify: `crates/presenter-server/src/state/sermons/write.rs` (head, two helpers, `commit_and_view`, two writes), `mod.rs` (module, re-export, two `SermonOp` variants), `tests.rs` (append)

**Interfaces:**
- Consumes: `canonical_book_by_code`, `translation_short_code`, `MAX_ITEM_VERSES`, `normalize_edits` (P1); `commit_sermon`, `SermonChange` (P3); `sermon_detail` (P4a); `load_sermon`, `installed` (P4b).
- Produces: `pub(crate) struct ItemRange { translation: String, book_code: String, chapter: u16, verse_start: u16, verse_end: u16 }` (re-exported); `pub(crate) async fn AppState::snapshot_item(&ItemRange) -> anyhow::Result<SermonItemRecord>` (fresh id, no edits; 422 `SermonRefusal` per the order in `check_range` + coverage); `pub(super) fn check_revision(&SermonSnapshot, i64) -> anyhow::Result<()>`; `pub(super) fn item_mut(&mut [SermonItemRecord], Uuid) -> anyhow::Result<&mut SermonItemRecord>` (404 "sermon item not found"); `pub(super) async fn commit_and_view(SermonChange<'_>) -> anyhow::Result<SermonDetail>`; `pub(crate) async fn add_sermon_item(id, expected_revision: i64, position: Option<usize>, &ItemRange, &SermonOrigin) -> anyhow::Result<SermonDetail>`; `pub(crate) async fn set_sermon_item_edits(id, item_id: Uuid, expected_revision: i64, &ItemEdits, &SermonOrigin) -> anyhow::Result<SermonDetail>`; `SermonOp::{Add, Edit}`.

- [ ] **Step 1: Append the tests** to `tests.rs`:

```rust

// ---- #851 P4c: add and edit items --------------------------------------------

fn range(translation: &str, book_code: &str, verses: std::ops::RangeInclusive<u16>) -> ItemRange {
    ItemRange {
        translation: translation.to_string(),
        book_code: book_code.to_string(),
        chapter: 11,
        verse_start: *verses.start(),
        verse_end: *verses.end(),
    }
}

#[tokio::test]
async fn an_added_item_snapshots_the_text_and_owns_its_slides() {
    let state = hebrews().await;
    let sermon = new_sermon(&state).await;

    let detail = state
        .add_sermon_item(
            sermon.sermon.id,
            0,
            None,
            &range("slk-seb", "HEB", 1..=3),
            &preacher(),
        )
        .await
        .expect("add");

    assert_eq!(detail.sermon.revision, 1);
    let item = &detail.items[0];
    let texts: Vec<&str> = item.record.source.iter().map(|v| v.text.as_str()).collect();
    assert_eq!(texts, SEB);
    assert_eq!(item.record.book_name, "Hebrejom");
    assert_eq!(item.label, "Hebrejom 11:1-3 (SEB)");
    assert!(!item.source_changed);
    assert_eq!(detail.slides.len(), 1);
    assert_eq!(detail.slides[0].item_id, item.record.id);
    assert_eq!(item.slide_ids, [detail.slides[0].slide.id]);
    assert!(item.rendered_text.starts_with("1. Viera je zárukou"));
}

#[tokio::test]
async fn item_refusals_name_the_problem_in_slovak() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let cases = [
        (
            range(NLT_CODE, "HEB", 1..=1),
            "NLT sa do kázne neukladá — vyber iný preklad",
        ),
        (
            range("slk-seb", "HEB", 1..=61),
            "Položka môže mať najviac 60 veršov",
        ),
        (
            range("slk-seb", "HEB", 1..=5),
            "HEB 11:1-5 nie je celé v preklade SEB",
        ),
        (range("slk-seb", "XYZ", 1..=1), "Neznáma kniha XYZ"),
        (
            range("slk-xyz", "HEB", 1..=1),
            "Preklad slk-xyz nie je nainštalovaný",
        ),
        (range("slk-seb", "HEB", 3..=2), "Neplatný rozsah veršov"),
    ];

    for (bad, message) in cases {
        let err = state
            .add_sermon_item(id, 0, None, &bad, &preacher())
            .await
            .expect_err("refused");
        assert_eq!(refusal(&err).as_deref(), Some(message), "{bad:?}");
    }
    let unchanged = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!(unchanged.sermon.revision, 0);
}

#[tokio::test]
async fn edits_are_normalized_and_a_bad_index_is_refused() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let added = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add");
    let item_id = added.items[0].record.id;
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![11, 10, 9, 8, 7],
            emphasized: vec![0, 7],
        },
    );

    let edited = state
        .set_sermon_item_edits(id, item_id, 1, &edits, &preacher())
        .await
        .expect("edit");

    let stored = &edited.items[0];
    assert_eq!(
        stored.record.edits.verse(1),
        Some(&VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: vec![0],
        })
    );
    assert_eq!(stored.label, "Hebrejom 11:1a (SEB)");
    assert_eq!(
        edited.slides[0].slide.main.value(),
        "1. VIERA je zárukou toho, v čo dúfame"
    );
    let mut bad = ItemEdits::default();
    bad.0.insert(
        1,
        VerseEdits {
            deleted: vec![40],
            emphasized: Vec::new(),
        },
    );
    let err = state
        .set_sermon_item_edits(id, item_id, 2, &bad, &preacher())
        .await
        .expect_err("index past the verse");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("Verš 1 nemá slovo s indexom 40")
    );
}

#[tokio::test]
async fn a_stale_revision_is_refused_before_anything_changes() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add");

    let err = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 2..=2), &preacher())
        .await
        .expect_err("stale");

    assert!(matches!(
        err.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::Conflict(
            presenter_persistence::SERMON_CONFLICT
        ))
    ));
    let detail = state
        .sermon_detail(id)
        .await
        .expect("detail")
        .expect("sermon");
    assert_eq!((detail.sermon.revision, detail.items.len()), (1, 1));
}
```

- [ ] **Step 2: Implement** — `crates/presenter-server/src/state/sermons/snapshot.rs`:

```rust
//! Item snapshots (#851 spec §7.1, §7.5): the database text of one verse range
//! when the item is added (or re-snapshotted). Reads go through the `AppState`
//! dispatch in `bible_source.rs` (bible-remote rule); the NLT is never an item
//! translation, so its text is never stored as a sermon source.

use chrono::Utc;
use presenter_core::bible::canonical_book_by_code;
use presenter_core::bible::sermon::{
    translation_short_code, ItemEdits, SourceVerse, MAX_ITEM_VERSES,
};
use presenter_persistence::SermonItemRecord;
use uuid::Uuid;

use super::super::AppState;
use super::SermonRefusal;
use crate::bible_remote;

/// One contiguous verse range of one chapter, as a request names it. A
/// cross-chapter selection is one range per chapter (the client splits it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemRange {
    pub(crate) translation: String,
    pub(crate) book_code: String,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    pub(crate) verse_end: u16,
}

impl AppState {
    /// A new item (fresh id, no edits) holding the database text of `range`;
    /// 422 with a Slovak message for an invalid range, more than 60 verses,
    /// the NLT, an unknown translation or book, or verses the translation lacks.
    pub(crate) async fn snapshot_item(
        &self,
        range: &ItemRange,
    ) -> anyhow::Result<SermonItemRecord> {
        check_range(range)?;
        let translation = self
            .list_bible_translations()
            .await?
            .into_iter()
            .find(|candidate| {
                candidate
                    .code
                    .eq_ignore_ascii_case(range.translation.trim())
            })
            .ok_or_else(|| {
                SermonRefusal::new(format!(
                    "Preklad {} nie je nainštalovaný",
                    range.translation
                ))
            })?;
        let canon = canonical_book_by_code(&range.book_code)
            .ok_or_else(|| SermonRefusal::new(format!("Neznáma kniha {}", range.book_code)))?;
        let rows = self
            .bible_passage_range(
                &translation.code,
                canon.english_name,
                Some(canon.code),
                range.chapter,
                range.verse_start,
                range.verse_end,
            )
            .await?;
        let complete = rows
            .first()
            .is_some_and(|first| first.reference.verse_start == range.verse_start)
            && rows
                .last()
                .is_some_and(|last| last.reference.verse_end == range.verse_end);
        let (Some(first), true) = (rows.first(), complete) else {
            let short = translation_short_code(&translation.code);
            let (chapter, start, end) = (range.chapter, range.verse_start, range.verse_end);
            return Err(SermonRefusal::new(format!(
                "{} {chapter}:{start}-{end} nie je celé v preklade {short}",
                canon.code
            ))
            .into());
        };
        let now = Utc::now();
        Ok(SermonItemRecord {
            id: Uuid::new_v4(),
            translation_code: translation.code.clone(),
            book_code: canon.code.to_string(),
            book_number: canon.number,
            book_name: first.reference.book.clone(),
            chapter: range.chapter,
            verse_start: range.verse_start,
            verse_end: range.verse_end,
            source: rows
                .iter()
                .map(|row| SourceVerse {
                    verse: row.reference.verse_start,
                    text: row.text.clone(),
                })
                .collect(),
            edits: ItemEdits::default(),
            slide_count: 0,
            created_at: now,
            updated_at: now,
        })
    }
}

fn check_range(range: &ItemRange) -> Result<(), SermonRefusal> {
    if range.chapter == 0 || range.verse_start == 0 || range.verse_end < range.verse_start {
        return Err(SermonRefusal::new("Neplatný rozsah veršov"));
    }
    if range.verse_end - range.verse_start >= MAX_ITEM_VERSES {
        return Err(SermonRefusal::new(format!(
            "Položka môže mať najviac {MAX_ITEM_VERSES} veršov"
        )));
    }
    if bible_remote::is_remote_translation(&range.translation) {
        return Err(SermonRefusal::new(
            "NLT sa do kázne neukladá — vyber iný preklad",
        ));
    }
    Ok(())
}
```

`write.rs` — replace everything above `/// \`POST /bible/sermons\`: every field optional` (module doc + imports) with:

```rust
//! The sermon writes of the editor's first steps (#851 spec §7.5, §11): create,
//! add an item, edit an item, delete. Each write loads the sermon, refuses a
//! stale `expected_revision` with 409, computes the new item list in memory and
//! commits it through `commit_sermon` (which re-checks the revision inside the
//! transaction). The answer is the full `SermonDetail`. The item upkeep writes
//! live in `maintain.rs` (P4d).

use std::time::Instant;

use chrono::{NaiveDate, Utc};
use presenter_core::bible::sermon::{
    default_sermon_title, default_service_date, normalize_edits, ItemEdits,
};
use presenter_core::{BiblePresentationId, BibleTranslation};
use presenter_persistence::{
    NewSermon, RepositoryError, SermonItemRecord, SermonSnapshot, SERMON_CONFLICT, SERMON_NOT_FOUND,
};
use uuid::Uuid;

use super::super::AppState;
use super::commit::{log_sermon_write, SermonChange, SermonLine};
use super::snapshot::ItemRange;
use super::view::SermonDetail;
use super::{SermonOp, SermonOrigin, SermonRefusal};
use crate::bible_remote;
use crate::live::LiveEvent;

const ITEM_NOT_FOUND: &str = "sermon item not found";
```

insert after the `SermonDraft` struct (before `/// An installed translation's own code`):

```rust
pub(super) fn check_revision(snapshot: &SermonSnapshot, expected: i64) -> anyhow::Result<()> {
    if snapshot.sermon.revision == expected {
        Ok(())
    } else {
        Err(RepositoryError::Conflict(SERMON_CONFLICT).into())
    }
}

pub(super) fn item_mut(
    items: &mut [SermonItemRecord],
    item_id: Uuid,
) -> anyhow::Result<&mut SermonItemRecord> {
    items
        .iter_mut()
        .find(|item| item.id == item_id)
        .ok_or_else(|| RepositoryError::NotFound(ITEM_NOT_FOUND).into())
}
```

insert inside `impl AppState` after `load_sermon`:

```rust
    /// Commit, then answer the stored sermon with the write's warnings.
    pub(super) async fn commit_and_view(
        &self,
        change: SermonChange<'_>,
    ) -> anyhow::Result<SermonDetail> {
        let id = change.sermon.id;
        let committed = self.commit_sermon(change).await?;
        let mut detail = self
            .sermon_detail(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        detail.warnings = committed.warnings;
        Ok(detail)
    }
```

insert after `create_sermon` (before `/// Delete the sermon's presentation`):

```rust
    /// Add the database text of `range` as an item at `position` (default: the
    /// end).
    pub(crate) async fn add_sermon_item(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        position: Option<usize>,
        range: &ItemRange,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = self.snapshot_item(range).await?;
        let item_id = item.id;
        let position = position
            .unwrap_or(snapshot.items.len())
            .min(snapshot.items.len());
        snapshot.items.insert(position, item);
        self.commit_and_view(SermonChange {
            op: SermonOp::Add,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    /// Replace one item's edits (validated and normalized against its snapshot).
    pub(crate) async fn set_sermon_item_edits(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        edits: &ItemEdits,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = item_mut(&mut snapshot.items, item_id)?;
        item.edits = normalize_edits(&item.source, edits)
            .map_err(|err| SermonRefusal::new(err.to_string()))?;
        item.updated_at = Utc::now();
        self.commit_and_view(SermonChange {
            op: SermonOp::Edit,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }
```

`mod.rs` — the module block and the `SermonOp` enum become:

```rust
mod commit;
mod generate;
mod guard;
mod snapshot;
mod view;
mod write;

pub(crate) use snapshot::ItemRange;
pub(crate) use view::{SermonDetail, SermonItemView, SermonList, SermonSlideView};
pub(crate) use write::SermonDraft;
```

```rust
pub(crate) enum SermonOp {
    Create,
    Add,
    Edit,
    Delete,
    Regenerate,
}
```

with the matching `as_str` arms `Self::Add => "add"`, `Self::Edit => "edit"`.

- [ ] **Step 3: How it is verified.** CI: the add snapshots the three SEB rows, labels "Hebrejom 11:1-3 (SEB)", one slide owned by the item; the six refusals answer their exact Slovak text and leave the revision at 0; edits `[11, 10, 9, 8, 7]` + emphasis `[0, 7]` are stored as `[7, 8, 9, 10, 11]` + `[0]`, label `1a`, slide "1. VIERA je zárukou toho, v čo dúfame"; index 40 → "Verš 1 nemá slovo s indexom 40"; a stale add → `Conflict(SERMON_CONFLICT)` and nothing changes. `check_range` refuses `end < start` before it subtracts (no `u16` underflow).

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/state/sermons/`, message `feat(server): add and edit sermon items (#851 P4c)` + Co-Authored-By.

### Task 32: HTTP — add an item, put its edits; ship P4c

**Files:**
- Modify: `crates/presenter-server/src/router/bible/sermons.rs` (imports, routes, two request DTOs, two handlers), `sermons/tests.rs` (append), `.claude/rules/sermon-prep.md` (append)

**Interfaces:**
- Consumes: `add_sermon_item`, `set_sermon_item_edits`, `ItemRange`.
- Produces: `AddItemRequest { expectedRevision, position?, translation, bookCode, chapter, verseStart, verseEnd }`, `EditsRequest { expectedRevision, verses: ItemEdits }`; routes `POST /bible/sermons/{id}/items`, `PUT /bible/sermons/{id}/items/{item_id}/edits`.

- [ ] **Step 1: Append the router tests**:

```rust

// ---- #851 P4c: add and edit items ----------------------------------------------

#[tokio::test]
async fn add_and_edit_an_item_over_http() {
    let app = app().await;
    let id = new_sermon(&app).await["id"]
        .as_str()
        .expect("id")
        .to_string();
    let items = format!("/bible/sermons/{id}/items");
    let item = json!({ "expectedRevision": 0, "translation": "slk-seb", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });

    let (status, added) = call(&app, "POST", &items, Some(item), &[]).await;
    assert_eq!(status, StatusCode::OK, "{added}");
    assert_eq!(added["items"][0]["label"], "Hebrejom 11:1 (SEB)");
    let item_id = added["items"][0]["id"]
        .as_str()
        .expect("item id")
        .to_string();
    let edits = json!({ "expectedRevision": 1,
        "verses": { "1": { "deleted": [7, 8, 9, 10, 11], "emphasized": [0] } } });
    let uri = format!("/bible/sermons/{id}/items/{item_id}/edits");
    let (status, edited) = call(&app, "PUT", &uri, Some(edits), &[]).await;

    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["items"][0]["label"], "Hebrejom 11:1a (SEB)");
    assert_eq!(
        edited["slides"][0]["mainText"],
        "1. VIERA je zárukou toho, v čo dúfame"
    );
}

#[tokio::test]
async fn item_refusals_are_422_409_and_404() {
    let app = app().await;
    let id = new_sermon(&app).await["id"]
        .as_str()
        .expect("id")
        .to_string();
    let items = format!("/bible/sermons/{id}/items");
    let nlt = json!({ "expectedRevision": 0, "translation": "eng-nlt", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });
    let stale = json!({ "expectedRevision": 7, "translation": "slk-seb", "bookCode": "HEB",
        "chapter": 11, "verseStart": 1, "verseEnd": 1 });

    let (status, body) = call(&app, "POST", &items, Some(nlt), &[]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["message"],
        "NLT sa do kázne neukladá — vyber iný preklad"
    );
    let (status, body) = call(&app, "POST", &items, Some(stale), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], "sermon changed on another device");
    let edits = json!({ "expectedRevision": 0, "verses": {} });
    let missing = format!("/bible/sermons/{id}/items/{}/edits", Uuid::new_v4());
    let (status, _) = call(&app, "PUT", &missing, Some(edits), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Implement** in `router/bible/sermons.rs`: `use axum::routing::get;` → `use axum::routing::{get, post, put};`; add `ItemRange` to the `crate::state::sermons` import (`local_today, ItemRange, SermonDetail, …`); `routes()` gains, after the `/bible/sermons/{id}` route:

```rust
        .route("/bible/sermons/{id}/items", post(add_item))
        .route("/bible/sermons/{id}/items/{item_id}/edits", put(put_edits))
```

Insert before `/// serde double-\`Option\``:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddItemRequest {
    expected_revision: i64,
    #[serde(default)]
    position: Option<usize>,
    translation: String,
    book_code: String,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditsRequest {
    expected_revision: i64,
    verses: ItemEdits,
}
```

Insert before `async fn identity(`'s `#[instrument(skip_all)]`:

```rust
#[instrument(skip_all)]
async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<AddItemRequest>,
) -> SermonResult {
    let range = ItemRange {
        translation: body.translation,
        book_code: body.book_code,
        chapter: body.chapter,
        verse_start: body.verse_start,
        verse_end: body.verse_end,
    };
    let detail = state
        .add_sermon_item(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            body.position,
            &range,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn put_edits(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<EditsRequest>,
) -> SermonResult {
    let detail = state
        .set_sermon_item_edits(
            BiblePresentationId::from_uuid(id),
            item_id,
            body.expected_revision,
            &body.verses,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}
```

- [ ] **Step 3: The rule** — append to `.claude/rules/sermon-prep.md`:

```markdown
- **A write's shape** (`write.rs`, `maintain.rs`): `load_sermon` (404) → `check_revision` (409 BEFORE
  any snapshot read) → compute the new item list in memory → `commit_and_view` (generation +
  the in-transaction revision check + the detail with the write's warnings).
- **`snapshot_item(ItemRange)`** checks, in this order: range sane → ≤ 60 verses (`end - start`
  only after `end ≥ start`, no underflow) → not the NLT → translation installed → book code
  canonical → the rows cover exactly `start..=end` (first `verse_start`, last `verse_end`).
  One chapter per item: the client splits a cross-chapter selection.
```

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router/bible/sermons/tests.rs .claude/rules/sermon-prep.md`, message `feat(server): add and edit sermon items over HTTP (#851 P4c)` + Co-Authored-By.

- [ ] **Step 5: Local gates** with `<files>` = `crates/presenter-server/src/state/sermons/snapshot.rs crates/presenter-server/src/state/sermons/write.rs crates/presenter-server/src/state/sermons/mod.rs crates/presenter-server/src/router/bible/sermons.rs`.

- [ ] **Step 6: Ship** with `<V>` = 0.4.325, `<title>` = "Add and edit sermon items (#851 P4c)".

- [ ] **Step 7: Functional check on SNV** — the August-sermon shape in miniature (T5 at the API level; the UI comes in E1–E3):

```bash
B=http://10.77.9.205/bible/sermons
ID=$(curl -fsS -X POST $B -H 'content-type: application/json' \
  -d '{"title":"zz-test-851","defaultTranslation":"slk-seb","secondaryTranslation":null}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
ITEM=$(curl -fsS -X POST $B/$ID/items -H 'content-type: application/json' \
  -d '{"expectedRevision":0,"translation":"slk-seb","bookCode":"DEU","chapter":28,"verseStart":20,"verseEnd":20}' | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["items"][0]["id"]); print(d["items"][0]["source"][0]["text"], file=sys.stderr)')
```

Read the printed Dt 28:20 text and strike from the token after "podnikať," to the end (on the 2026-10-14 SEB text: 31 tokens, "podnikať," is token 17 — re-check against the printed text): `curl -fsS -X PUT $B/$ID/items/$ITEM/edits -H 'content-type: application/json' -d '{"expectedRevision":1,"verses":{"20":{"deleted":[18,19,20,21,22,23,24,25,26,27,28,29,30],"emphasized":[]}}}'`. Expected: `items[0].label` = "Deuteronómium 28:20a (SEB)", `slides[0].mainText` starts "20. " and ends "a čo budeš podnikať" (the comma trimmed, no period added), `slides[0].mainReference` = the same label. Then `curl -fsS -X DELETE $B/$ID` (204) and `journalctl … | grep presenter::bible::sermon` → `create`, `add`, `edit`, `delete` lines. PP: `GET /bible/sermons` after the release.

- [ ] **Step 8:** `playbook-review`; comment on #851 with the Dt 28:20 result.

---

## PR unit P4d — item upkeep, reorder, update, regenerate; docs; milestone M1 (v0.4.326)

Spec §7.5 (translation change re-snapshots and clears the edits; "Obnoviť text" keeps the edits of unchanged verses; reorder with a stale set → 409), §11 (`PATCH /bible/sermons/{id}`, `PATCH`/`DELETE /bible/sermons/{id}/items/{itemId}`, `POST …/items/reorder`, `POST …/regenerate`), §2 T6–T8.

### Task 33: Post-release version bump to 0.4.326

**Files:**
- Modify: `Cargo.toml:15` (`[workspace.package] version`), `Cargo.lock` (the 7 workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Check the versions** (expected main = dev = 0.4.325)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
```

If main is below 0.4.325 (the previous unit is not released yet), STOP: this unit starts after that release. If `DEV` is already above `MAIN`, skip Steps 2-3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # 0.4.326 expected; Cargo.toml 1 line, Cargo.lock 7 entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to 0.4.326 [no-test: version bump only]` + blank line + the Co-Authored-By line.

### Task 34: State — translation change, refresh, delete item, reorder, update, regenerate

**Files:**
- Create: `crates/presenter-server/src/state/sermons/maintain.rs`
- Modify: `crates/presenter-server/src/state/sermons/mod.rs` (module, re-export, five `SermonOp` variants), `tests.rs` (append)

**Interfaces:**
- Consumes: `retain_unchanged_edits` (P1); `check_revision`, `item_mut`, `installed`, `load_sermon`, `commit_and_view`, `snapshot_item`, `ItemRange` (P4b/P4c).
- Produces: `pub(crate) struct SermonPatch { title: Option<String>, service_date: Option<NaiveDate>, secondary_translation: Option<Option<String>> }` (Default, re-exported); `pub(crate)` `AppState::change_sermon_item_translation(id, item_id, expected_revision, translation: &str, &SermonOrigin)`, `refresh_sermon_item_source(id, item_id, expected_revision, &SermonOrigin)`, `delete_sermon_item(id, item_id, expected_revision, &SermonOrigin)`, `reorder_sermon_items(id, expected_revision, item_ids: &[Uuid], &SermonOrigin)`, `update_sermon(id, expected_revision, SermonPatch, &SermonOrigin)`, `regenerate_sermon(id, expected_revision, &SermonOrigin)` — each `-> anyhow::Result<SermonDetail>`; `SermonOp::{Translation, Refresh, DeleteItem, Reorder, Update}`.

- [ ] **Step 1: Append the tests** to `tests.rs`:

```rust

// ---- #851 P4d: item upkeep, reorder, update, regenerate ----------------------

/// A sermon with HEB 11:1 (verse 1 cut to `1a`) and HEB 11:2-3; revision 3.
async fn sermon_with_two_items(state: &AppState) -> SermonDetail {
    let id = new_sermon(state).await.sermon.id;
    let first = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=1), &preacher())
        .await
        .expect("add first");
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![7, 8, 9, 10, 11],
            emphasized: Vec::new(),
        },
    );
    state
        .set_sermon_item_edits(id, first.items[0].record.id, 1, &edits, &preacher())
        .await
        .expect("edit");
    state
        .add_sermon_item(id, 2, None, &range("slk-seb", "HEB", 2..=3), &preacher())
        .await
        .expect("add second")
}

#[tokio::test]
async fn changing_the_translation_resnapshots_and_clears_the_edits() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let item_id = sermon.items[0].record.id;

    let changed = state
        .change_sermon_item_translation(sermon.sermon.id, item_id, 3, "eng-kjv", &preacher())
        .await
        .expect("translation");

    let item = &changed.items[0];
    assert_eq!(
        item.record.id, item_id,
        "the item keeps its id and its slide ids"
    );
    assert_eq!(item.record.translation_code, "eng-kjv");
    assert_eq!(item.record.book_name, "Hebrews");
    assert_eq!(item.record.edits, ItemEdits::default());
    assert_eq!(item.label, "Hebrews 11:1 (KJV)");
    assert_eq!(item.record.source[0].text, KJV[0]);
}

#[tokio::test]
async fn refreshing_keeps_the_edits_of_unchanged_verses_only() {
    let state = hebrews().await;
    let id = new_sermon(&state).await.sermon.id;
    let added = state
        .add_sermon_item(id, 0, None, &range("slk-seb", "HEB", 1..=2), &preacher())
        .await
        .expect("add");
    let item_id = added.items[0].record.id;
    let mut edits = ItemEdits::default();
    edits.0.insert(
        1,
        VerseEdits {
            deleted: vec![11],
            emphasized: Vec::new(),
        },
    );
    edits.0.insert(
        2,
        VerseEdits {
            deleted: vec![0],
            emphasized: Vec::new(),
        },
    );
    state
        .set_sermon_item_edits(id, item_id, 1, &edits, &preacher())
        .await
        .expect("edit");
    seed(
        &state,
        "slk-seb",
        "sk",
        "Hebrejom",
        &[SEB[0], "Iné znenie druhého verša.", SEB[2]],
    )
    .await;

    let refreshed = state
        .refresh_sermon_item_source(id, item_id, 2, &preacher())
        .await
        .expect("refresh");

    let item = &refreshed.items[0];
    assert!(!item.source_changed);
    assert_eq!(item.record.source[1].text, "Iné znenie druhého verša.");
    assert_eq!(item.record.edits.0.keys().copied().collect::<Vec<_>>(), [1]);
}

#[tokio::test]
async fn deleting_and_reordering_items() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let id = sermon.sermon.id;
    let (first, second) = (sermon.items[0].record.id, sermon.items[1].record.id);

    let reordered = state
        .reorder_sermon_items(id, 3, &[second, first], &preacher())
        .await
        .expect("reorder");
    let order: Vec<_> = reordered.items.iter().map(|item| item.record.id).collect();
    assert_eq!(order, [second, first]);
    assert_eq!(
        reordered.slides[0].item_id, second,
        "slides follow the items"
    );

    let stale_set = state
        .reorder_sermon_items(id, 4, &[second], &preacher())
        .await
        .expect_err("a different item set is a stale view");
    assert!(matches!(
        stale_set.downcast_ref::<presenter_persistence::RepositoryError>(),
        Some(presenter_persistence::RepositoryError::Conflict(_))
    ));

    let deleted = state
        .delete_sermon_item(id, second, 4, &preacher())
        .await
        .expect("delete item");
    let left: Vec<_> = deleted.items.iter().map(|item| item.record.id).collect();
    assert_eq!(left, [first]);
    assert_eq!(deleted.slides.len(), 1);
}

#[tokio::test]
async fn update_and_regenerate() {
    let state = hebrews().await;
    let sermon = sermon_with_two_items(&state).await;
    let id = sermon.sermon.id;
    let patch = SermonPatch {
        title: Some("  Viera  ".to_string()),
        service_date: Some(date("2026-10-25")),
        secondary_translation: Some(Some("eng-kjv".to_string())),
    };

    let updated = state
        .update_sermon(id, 3, patch, &preacher())
        .await
        .expect("update");

    assert_eq!(updated.sermon.title, "Viera");
    assert_eq!(updated.sermon.service_date, date("2026-10-25"));
    let presentation = state
        .bible_presentation_detail(id)
        .await
        .expect("detail")
        .expect("presentation");
    assert_eq!(presentation.name, "Viera");
    assert_eq!(
        presentation.slides[0].secondary_reference,
        "Hebrews 11:1 (KJV)"
    );
    let empty = SermonPatch {
        title: Some("  ".to_string()),
        ..SermonPatch::default()
    };
    let err = state
        .update_sermon(id, 4, empty, &preacher())
        .await
        .expect_err("empty title");
    assert_eq!(
        refusal(&err).as_deref(),
        Some("Názov kázne nesmie byť prázdny")
    );
    let regenerated = state
        .regenerate_sermon(id, 4, &preacher())
        .await
        .expect("regenerate");
    assert_eq!(regenerated.sermon.revision, 5);
}
```

- [ ] **Step 2: Implement** — `crates/presenter-server/src/state/sermons/maintain.rs`:

```rust
//! The item upkeep writes (#851 spec §7.5, §11): change an item's translation,
//! refresh its text, delete it, reorder the items, change the sermon's title,
//! date or secondary translation, regenerate the slides. Same shape as
//! `write.rs`: load, 409 on a stale revision, compute, `commit_and_view`.

use presenter_core::bible::sermon::{retain_unchanged_edits, ItemEdits};
use presenter_core::BiblePresentationId;
use presenter_persistence::{RepositoryError, SermonItemRecord, SERMON_CONFLICT};
use uuid::Uuid;

use super::super::AppState;
use super::commit::SermonChange;
use super::snapshot::ItemRange;
use super::view::SermonDetail;
use super::write::{check_revision, installed, item_mut};
use super::{SermonOp, SermonOrigin, SermonRefusal};

/// `PATCH /bible/sermons/{id}`: `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SermonPatch {
    pub(crate) title: Option<String>,
    pub(crate) service_date: Option<NaiveDate>,
    pub(crate) secondary_translation: Option<Option<String>>,
}

fn non_empty_title(title: &str) -> Result<String, SermonRefusal> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(SermonRefusal::new("Názov kázne nesmie byť prázdny"));
    }
    Ok(trimmed.to_string())
}

impl AppState {
    /// Re-snapshot one item in another translation; its edits are cleared
    /// (the editor confirms first, spec §7.5).
    pub(crate) async fn change_sermon_item_translation(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        translation: &str,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        self.resnapshot_item(id, item_id, expected_revision, Some(translation), origin)
            .await
    }

    /// "Obnoviť text": re-snapshot one item; the edits of verses whose text is
    /// unchanged stay, the rest are cleared.
    pub(crate) async fn refresh_sermon_item_source(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        self.resnapshot_item(id, item_id, expected_revision, None, origin)
            .await
    }

    async fn resnapshot_item(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        translation: Option<&str>,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = item_mut(&mut snapshot.items, item_id)?;
        let range = ItemRange {
            translation: translation
                .unwrap_or(item.translation_code.as_str())
                .to_string(),
            book_code: item.book_code.clone(),
            chapter: item.chapter,
            verse_start: item.verse_start,
            verse_end: item.verse_end,
        };
        let fresh = self.snapshot_item(&range).await?;
        let edits = match translation {
            Some(_) => ItemEdits::default(),
            None => retain_unchanged_edits(&item.source, &fresh.source, &item.edits),
        };
        *item = SermonItemRecord {
            id: item.id,
            edits,
            created_at: item.created_at,
            ..fresh
        };
        let op = match translation {
            Some(_) => SermonOp::Translation,
            None => SermonOp::Refresh,
        };
        self.commit_and_view(SermonChange {
            op,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    pub(crate) async fn delete_sermon_item(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        item_mut(&mut snapshot.items, item_id)?;
        snapshot.items.retain(|item| item.id != item_id);
        self.commit_and_view(SermonChange {
            op: SermonOp::DeleteItem,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    /// The items in the order of `item_ids`, which must be exactly the stored
    /// set (a different set is a stale view: 409, spec §7.5).
    pub(crate) async fn reorder_sermon_items(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        item_ids: &[Uuid],
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let mut reordered = Vec::with_capacity(snapshot.items.len());
        for item_id in item_ids {
            let Some(index) = snapshot.items.iter().position(|item| item.id == *item_id) else {
                return Err(RepositoryError::Conflict(SERMON_CONFLICT).into());
            };
            reordered.push(snapshot.items.swap_remove(index));
        }
        if !snapshot.items.is_empty() {
            return Err(RepositoryError::Conflict(SERMON_CONFLICT).into());
        }
        self.commit_and_view(SermonChange {
            op: SermonOp::Reorder,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: reordered,
            item_id: None,
        })
        .await
    }

    /// Title (= the presentation name), service date, secondary translation.
    pub(crate) async fn update_sermon(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        patch: SermonPatch,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        if let Some(title) = patch.title.as_deref() {
            snapshot.sermon.title = non_empty_title(title)?;
        }
        if let Some(date) = patch.service_date {
            snapshot.sermon.service_date = date;
        }
        if let Some(secondary) = patch.secondary_translation {
            let translations = self.list_bible_translations().await?;
            snapshot.sermon.secondary_translation_code = match secondary {
                Some(code) => Some(installed(&translations, &code, true)?),
                None => None,
            };
        }
        self.commit_and_view(SermonChange {
            op: SermonOp::Update,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }

    /// `POST /bible/sermons/{id}/regenerate`: the same items, the slides
    /// generated again (retries the secondary line after an NLT failure).
    pub(crate) async fn regenerate_sermon(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        self.commit_and_view(SermonChange {
            op: SermonOp::Regenerate,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }
}
```

`mod.rs` — the whole file after this task (the final state of the module root):

```rust
//! Sermon preparation on the server (#851 spec §7). A write loads the sermon,
//! computes the new item list in memory, generates the slides from the items
//! (`generate.rs`) and commits items + slides in ONE repository transaction
//! guarded by the expected revision (`commit.rs`): a stale write is
//! `RepositoryError::Conflict` → 409 and is never half-applied. Then
//! `LiveEvent::BibleSlidesChanged` is published and one INFO
//! `presenter::bible::sermon` line is logged. Item text is a snapshot, so a
//! Bible re-import never changes a sermon (spec §5.7).

mod commit;
mod generate;
mod guard;
mod maintain;
mod snapshot;
mod view;
mod write;

pub(crate) use maintain::SermonPatch;
pub(crate) use snapshot::ItemRange;
pub(crate) use view::{SermonDetail, SermonItemView, SermonList, SermonSlideView};
pub(crate) use write::SermonDraft;

use chrono::NaiveDate;

/// Who wrote (spec §13): the Cloudflare Access e-mail when the request carried
/// one, and the client IP by the #793 rule. Both are labels, never
/// permissions (spec §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonOrigin {
    pub(crate) author: Option<String>,
    pub(crate) client_ip: String,
}

impl SermonOrigin {
    /// A write the server makes on its own (the character-limit pass).
    pub(crate) fn system() -> Self {
        Self {
            author: None,
            client_ip: "system".to_string(),
        }
    }
}

/// The `op` field of the `presenter::bible::sermon` line (spec §13). A unit
/// adds its variant together with the write that constructs it (`dead_code`
/// under `-D warnings` otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SermonOp {
    Create,
    Add,
    Edit,
    Translation,
    Refresh,
    DeleteItem,
    Reorder,
    Update,
    Delete,
    Regenerate,
}

impl SermonOp {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Translation => "translation",
            Self::Refresh => "refresh",
            Self::DeleteItem => "delete_item",
            Self::Reorder => "reorder",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Regenerate => "regenerate",
        }
    }
}

/// A sermon request the server refuses with 422 and this Slovak message (an
/// invalid range, the NLT as an item translation, a bad token index …). Decided
/// in the state layer, so it is its own type (repository-error-pattern rule:
/// never forced through `RepositoryError`); `router/bible/sermons.rs` maps it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct SermonRefusal(pub(crate) String);

impl SermonRefusal {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The server-local date: "today" for `/bible/sermons/today`, the default
/// service date and the character-limit pass. The only clock read of the
/// sermon code; everything below it takes the date as a parameter.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 3: How it is verified.** CI: a translation change keeps the item id, re-snapshots in KJV ("Hebrews 11:1 (KJV)", the KJV text) and clears the edits; a refresh after a re-import keeps verse 1's edit and drops verse 2's (its text changed), `sourceChanged` false again; a reorder moves the slides with the items, a different id set is a 409, a delete leaves one item and one slide; an update trims the title, renames the presentation, adds the KJV line ("Hebrews 11:1 (KJV)"), an empty title is 422, regenerate bumps the revision.

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/state/sermons/`, message `feat(server): sermon item upkeep, reorder, update and regenerate (#851 P4d)` + Co-Authored-By.

### Task 35: HTTP — the upkeep routes

**Files:**
- Modify: `crates/presenter-server/src/router/bible/sermons.rs` (imports, routes, four request DTOs, five handlers), `sermons/tests.rs` (append)

**Interfaces:**
- Consumes: Task 34's methods, `SermonPatch`.
- Produces: `UpdateSermonRequest`, `PatchItemRequest { expectedRevision, translation?, refreshSource }`, `ReorderRequest { expectedRevision, itemIds }`, `RevisionRequest`, `RevisionQuery` (`expectedRevision`, alias `expected_revision`); routes `PATCH /bible/sermons/{id}`, `PATCH`/`DELETE /bible/sermons/{id}/items/{item_id}`, `POST /bible/sermons/{id}/items/reorder`, `POST /bible/sermons/{id}/regenerate`.

- [ ] **Step 1: Append the router test**:

```rust

// ---- #851 P4d: item upkeep, reorder, update, regenerate ---------------------------

#[tokio::test]
async fn upkeep_routes_answer_the_whole_sermon() {
    let (app, id) = app_with_sermon().await;
    let (_, sermon) = call(&app, "GET", &format!("/bible/sermons/{id}"), None, &[]).await;
    let item_id = sermon["items"][0]["id"]
        .as_str()
        .expect("item id")
        .to_string();
    let item_uri = format!("/bible/sermons/{id}/items/{item_id}");

    let refresh = json!({ "expectedRevision": 2, "refreshSource": true });
    let (status, refreshed) = call(&app, "PATCH", &item_uri, Some(refresh), &[]).await;
    assert_eq!(
        (status, refreshed["revision"].clone()),
        (StatusCode::OK, json!(3))
    );
    assert_eq!(
        refreshed["items"][0]["edits"]["1"]["emphasized"],
        json!([0])
    );
    let neither = json!({ "expectedRevision": 3 });
    let (status, _) = call(&app, "PATCH", &item_uri, Some(neither), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let reorder = json!({ "expectedRevision": 3, "itemIds": [item_id] });
    let uri = format!("/bible/sermons/{id}/items/reorder");
    let (status, reordered) = call(&app, "POST", &uri, Some(reorder), &[]).await;
    assert_eq!(
        (status, reordered["revision"].clone()),
        (StatusCode::OK, json!(4))
    );
    let rename = json!({ "expectedRevision": 4, "title": "Viera", "secondaryTranslation": null });
    let (status, renamed) = call(
        &app,
        "PATCH",
        &format!("/bible/sermons/{id}"),
        Some(rename),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["title"], "Viera");
    let regenerate = json!({ "expectedRevision": 5 });
    let uri = format!("/bible/sermons/{id}/regenerate");
    let (status, regenerated) = call(&app, "POST", &uri, Some(regenerate), &[]).await;
    assert_eq!(
        (status, regenerated["revision"].clone()),
        (StatusCode::OK, json!(6))
    );
    let delete = format!("{item_uri}?expectedRevision=6");
    let (status, emptied) = call(&app, "DELETE", &delete, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{emptied}");
    assert_eq!(
        (emptied["items"].clone(), emptied["slides"].clone()),
        (json!([]), json!([]))
    );
}
```

- [ ] **Step 2: Implement** in `router/bible/sermons.rs`: `use axum::extract::{ConnectInfo, Path, State};` → `use axum::extract::{ConnectInfo, Path, Query, State};`; `use axum::routing::{get, post, put};` → `use axum::routing::{get, patch, post, put};`; add `SermonPatch` to the `crate::state::sermons` import (after `SermonOrigin`). `routes()` becomes:

```rust
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/bible/sermons", get(list_sermons).post(create_sermon))
        .route("/bible/sermons/today", get(todays_sermons))
        .route(
            "/bible/sermons/{id}",
            get(get_sermon).patch(update_sermon).delete(delete_sermon),
        )
        .route("/bible/sermons/{id}/items", post(add_item))
        .route("/bible/sermons/{id}/items/reorder", post(reorder_items))
        .route(
            "/bible/sermons/{id}/items/{item_id}",
            patch(patch_item).delete(delete_item),
        )
        .route("/bible/sermons/{id}/items/{item_id}/edits", put(put_edits))
        .route("/bible/sermons/{id}/regenerate", post(regenerate))
        .route("/bible/study/identity", get(identity))
}
```

(matchit: the static `reorder` segment wins over `{item_id}`; a PATCH on `/items/reorder` is a 405, never a translation change.) Insert before `/// serde double-\`Option\``:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSermonRequest {
    expected_revision: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    service_date: Option<NaiveDate>,
    #[serde(default, deserialize_with = "double_option")]
    secondary_translation: Option<Option<String>>,
}

/// Either `{expectedRevision, translation}` or `{expectedRevision,
/// refreshSource: true}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchItemRequest {
    expected_revision: i64,
    #[serde(default)]
    translation: Option<String>,
    #[serde(default)]
    refresh_source: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderRequest {
    expected_revision: i64,
    item_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevisionRequest {
    expected_revision: i64,
}

/// `?expectedRevision=` as spec §11 writes it; `expected_revision` (the
/// snake_case query convention) is accepted too.
#[derive(Debug, Deserialize)]
struct RevisionQuery {
    #[serde(rename = "expectedRevision", alias = "expected_revision")]
    expected_revision: i64,
}
```

Insert before `async fn identity(`'s `#[instrument(skip_all)]`:

```rust
#[instrument(skip_all)]
async fn update_sermon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<UpdateSermonRequest>,
) -> SermonResult {
    let patch = SermonPatch {
        title: body.title,
        service_date: body.service_date,
        secondary_translation: body.secondary_translation,
    };
    let detail = state
        .update_sermon(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            patch,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn patch_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<PatchItemRequest>,
) -> SermonResult {
    let id = BiblePresentationId::from_uuid(id);
    let origin = origin(&headers, peer);
    let revision = body.expected_revision;
    let result = match (body.refresh_source, body.translation.as_deref()) {
        (true, _) => {
            state
                .refresh_sermon_item_source(id, item_id, revision, &origin)
                .await
        }
        (false, Some(translation)) => {
            state
                .change_sermon_item_translation(id, item_id, revision, translation, &origin)
                .await
        }
        (false, None) => {
            return Err(AppError::bad_request_message(
                "translation or refreshSource is required",
            ))
        }
    };
    Ok(Json(detail_dto(result.map_err(sermon_error)?)))
}

#[instrument(skip_all)]
async fn delete_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<RevisionQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> SermonResult {
    let detail = state
        .delete_sermon_item(
            BiblePresentationId::from_uuid(id),
            item_id,
            query.expected_revision,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn reorder_items(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<ReorderRequest>,
) -> SermonResult {
    let detail = state
        .reorder_sermon_items(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            &body.item_ids,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn regenerate(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<RevisionRequest>,
) -> SermonResult {
    let detail = state
        .regenerate_sermon(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}
```

- [ ] **Step 3: How it is verified.** CI: on the `app_with_sermon` sermon (revision 2): refresh → 3 (the emphasis on "Viera" survives, its verse is unchanged), neither field → 400, reorder of the full set → 4, title + `secondaryTranslation: null` → 5, regenerate → 6, delete item `?expectedRevision=6` → 7 with no items and no slides.

- [ ] **Step 4: Commit** — paths `crates/presenter-server/src/router/bible/sermons.rs crates/presenter-server/src/router/bible/sermons/tests.rs`, message `feat(server): sermon upkeep routes (#851 P4d)` + Co-Authored-By.

### Task 36: Docs, rule, ship P4d, verify milestone M1

**Files:**
- Modify: `docs/architecture.md:218-231` (Key HTTP Endpoints table), `docs/functional-needs.md:7-10` (Bible section), `.claude/rules/sermon-prep.md` (append)

- [ ] **Step 1: Docs.** `docs/architecture.md`, append to the Key HTTP Endpoints table:

```markdown
| `/bible/sermons` | Sermons: list (with the server's `today` + default service date), create (#851) |
| `/bible/sermons/today` | Today's sermons in full (operator console, tablet, part C) |
| `/bible/sermons/{id}` | One sermon: items, computed labels, generated slides; PATCH / DELETE |
| `/bible/sermons/{id}/items…` | Add, edit (strike + emphasize), refresh, change translation, reorder, delete items |
| `/bible/study/identity` | The Cloudflare Access e-mail (an author label, never a permission) |
```

`docs/functional-needs.md`, append to `## Bible`:

```markdown
- Sermon preparation (#851): the preacher builds the sermon's verse list on his own phone or tablet; each item is one verse range of one chapter, edited only by striking words and emphasizing them (no rewording); labels such as "Deuteronómium 28:20a (SEB)" are computed; the sermon IS a Bible presentation, so its generated slides play on the walls, the tablet and the console without retyping.
- Clean verse text: no section headings, footnotes, paragraph marks or translator's notes inside a verse; bracketed words that belong to a translation stay.
```

- [ ] **Step 2: The rule** — append to `.claude/rules/sermon-prep.md`:

```markdown
- `PATCH …/items/{itemId}`: `refreshSource: true` wins over `translation`; neither → 400. A
  translation change re-snapshots and CLEARS the item's edits (the editor confirms first); a
  refresh keeps the edits of verses whose text is unchanged (`retain_unchanged_edits`). Both
  keep the item id (and so its slide ids).
- `POST …/items/reorder` needs EXACTLY the stored item set; a different set (missing, extra,
  duplicate id) is a stale view → `Conflict(SERMON_CONFLICT)` → 409, never a partial reorder.
- `POST …/regenerate` = the same items, the slides generated again (retries the secondary line
  after an NLT failure).
```

Commit — paths `docs/architecture.md docs/functional-needs.md .claude/rules/sermon-prep.md`, message `docs(#851): sermon API in the architecture and functional docs; upkeep rules (P4d) [no-test: docs only]`.

- [ ] **Step 3: Local gates** with `<files>` = `crates/presenter-server/src/state/sermons/maintain.rs crates/presenter-server/src/state/sermons/mod.rs crates/presenter-server/src/router/bible/sermons.rs`. Expected: `router/bible/sermons.rs` ≈ 400 production lines, `maintain.rs` ≈ 190, `resnapshot_item` ≈ 45 lines.

- [ ] **Step 4: Ship** with `<V>` = 0.4.326, `<title>` = "Sermon item upkeep, reorder, update, regenerate (#851 P4d) — M1".

- [ ] **Step 5: Milestone M1 on SNV** — one scripted pass with a test sermon (deleted at the end), then the journal:

```bash
B=http://10.77.9.205/bible/sermons
J='content-type: application/json'
ID=$(curl -fsS -X POST $B -H "$J" -d '{"title":"zz-test-851","defaultTranslation":"slk-seb","secondaryTranslation":null}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
R=$(curl -fsS -X POST $B/$ID/items -H "$J" -d '{"expectedRevision":0,"translation":"slk-seb","bookCode":"HEB","chapter":11,"verseStart":1,"verseEnd":1}')
ITEM=$(echo "$R" | python3 -c 'import json,sys; print(json.load(sys.stdin)["items"][0]["id"])')
# T7: a second device writes first; the stale write is a 409 and changes nothing
curl -fsS -X PUT $B/$ID/items/$ITEM/edits -H "$J" -d '{"expectedRevision":1,"verses":{"1":{"deleted":[7,8,9,10,11],"emphasized":[0]}}}' > /dev/null
curl -s -o /dev/null -w '%{http_code}\n' -X PUT $B/$ID/items/$ITEM/edits -H "$J" -d '{"expectedRevision":1,"verses":{}}'
curl -fsS $B/$ID | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["revision"], d["items"][0]["label"], d["slides"][0]["mainText"])'
curl -fsS -X POST $B/$ID/items/reorder -H "$J" -d "{\"expectedRevision\":2,\"itemIds\":[\"$ITEM\"]}" > /dev/null
curl -fsS -X PATCH $B/$ID -H "$J" -d '{"expectedRevision":3,"secondaryTranslation":"eng-kjv"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["slides"][0]["secondaryReference"])'
curl -fsS -X POST $B/$ID/regenerate -H "$J" -d '{"expectedRevision":4}' > /dev/null
curl -fsS -X PATCH $B/$ID/items/$ITEM -H "$J" -d '{"expectedRevision":5,"refreshSource":true}' > /dev/null
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE $B/$ID
ssh -o BatchMode=yes newlevel@presenter.lan "journalctl -u presenter --since '-5 min' --no-pager | grep 'presenter::bible::sermon'"
```

Expected: `409`; `2 Hebrejom 11:1a (SEB) 1. VIERA je zárukou toho, v čo dúfame` (the SEB Heb 11:1 text the spec quotes; if D1 changed its spacing the token indices still hold — it has no note); `Hebrews 11:1 (KJV)`; `204`; INFO lines `create`, `add`, `edit`, `reorder`, `update`, `regenerate`, `refresh`, `delete` (T8) plus one WARN "sermon request refused" for the 409. T6 is pinned by `a_reimport_leaves_slides_sermons_and_items_byte_identical` (P2b). PP: `GET /bible/sermons` → `[]`-list after the release.

- [ ] **Step 6: The Access header** (spec §9 [unverified]): dev2 is inside the church egress (Access bypass → no header), so it cannot be checked from here. Leave it [unverified]; comment on #851 that the first sermon write from outside the church shows `author="<e-mail>"` in the journal or not, and that nothing depends on it.

- [ ] **Step 7:** `playbook-review`; comment on #851: **M1 reached** — D1 on SNV/PP (T1 numbers), the sermon API on SNV/PP (T6 test, T7 409, T8 lines), the measured unit sizes, and the spec gaps below that need an owner decision (warnings after reload, UI DTO field, `prepared_references` unit).

---

## Self-review against the spec

**§5.1 Import fixes (D1)** — every row: ROH `<TS>` dropped (case-insensitive), KJV `\f`/`\x` dropped with their text, SEB notes removed with the leading whitespace before tag stripping, whitespace before `. , ; : ! ?` dropped for every MySword module after it, bare `[…]` kept, a leading `¶` dropped (Tasks 2–3); `PARSER_VERSION` 2 (Task 3); `.claude/rules/bible-import.md` with the listed paths (Task 4). §14 D1 tests: SEB note → no stray space, Mk 16:20 shorter ending + "[Amen.]" kept, ROH `<TS>` dropped, ROH "[a pôstom]" kept, KJV GEN 2:14 ends "Euphrates.", JHN 3:16 starts "For God", `strips_strongs_concordance_markup_from_usfm` untouched and green. T1 is checked on SNV and PP with read-only SQL (Task 4). Addition: the glued `\s1` heading lines (Spec gaps 1). D1 adds no schema and no field to `bible_passage` / `BiblePassage`, so the §5.2 side-table decision (`bible_passage_markup`) is untouched for D2a/D2b.

**§7.1 Data model (P2a)** — both tables exactly as specified (raw SQL), no FK from items to translations, the cascade chain presentation → sermon → items, the date index and the order index; entities in a new file (gap 3); `sermon_date` on `BiblePresentationSummary` / `BiblePresentation` (4 literals updated) and on the server's presentation DTOs (P4b, gap 4); ≤ 60 verses and "NLT is never an item translation" (`snapshot_item`, P4c; create refuses the NLT as default translation, P4b).

**§7.2 Editing model (P1)** — tokens, `deleted` / `emphasized`, struck beats emphasized, index validation → 422 (`EditError` → `SermonRefusal`). The editor gestures (`state/sermon_edit.rs`: mode toggle, long press, parenthesis expansion, cut to end / from start, undo) are UI (E2), not M1.

**§7.3 Rendering and labels (P1)** — every table row has a test; the mockup example and Dt 28:20 shape; main label `"{book_name} {chapter}:{suffix} ({SHORT})"`; line format `"{verse}. {text}"` unchanged.

**§7.4 Slide generation (P2a + P3)** — `compose_bible_slides_labelled` + thin wrapper, existing composer tests unchanged (Task 13); `state/sermons/generate.rs` with the item labels and `get_bible_preferences().character_limit`, #828 rules (Task 19 tests); secondary = full kept verses, secondary book, plain numbers, NLT failure → warning (Task 19); stable v5 ids (Task 19); metadata = today's `BibleSlideMetadata`, no new field; written in the sermon write transaction, then `BibleSlidesChanged` (Task 20); character-limit regeneration for `service_date ≥ today` with one retry, logged (Task 20). `state/sermons/snapshot.rs` lands in P4c, where its first caller is.

**§7.5 Writes, revisions, concurrency (P2b–P4d)** — `expectedRevision` on every write; ONE transaction in `save_sermon_change` (revision check, items with `slide_count`, slides, revision + `updated_at`); stale revision and stale reorder set → `Conflict` → 409; `sourceChanged` on reads; refresh keeps unchanged-verse edits; a translation change re-snapshots and clears the edits.

**§8.2 Server guards (P4b)** — `refuse_if_sermon` first in append / delete / reorder / update slide, 409 with the spec's Slovak text, the AI tools covered through the same methods, rename and delete allowed. The read-only Prepared-tab component is UI (H1).

**§9 Access and identity (P4a/P4b)** — `access_email` with the loopback rule, trimming, lower case, `@`, ≤ 254; stored as `author_email` on create, logged on every write; `GET /bible/study/identity`. The live header check stays [unverified] (Task 36 Step 6).

**§11 API (P4a–P4d)** — every sermon route with its body and answer shape (`SermonDetail` fields exactly as listed), snake_case query / camelCase JSON; writes answer the full sermon; `DELETE /bible/sermons/{id}` deletes the presentation. `/bible/search` scope, `/bible/study/*` reads are D4.

**§12 Errors** — 404 / 422 / 409 / NLT warning as tabled; every failed sermon request logs WARN with its cause (`sermon_error`).

**§13 Observability** — one INFO `presenter::bible::sermon` line per write with all listed fields (`op` values create, add, edit, translation, refresh, delete_item, reorder, update, delete, regenerate). The import INFO counts per markup kind are D2b.

**§14 Test plan** — presenter-core (P1), D1, persistence incl. T6 (P2a/P2b; the "reorder with a different id set" case moved to the state, gap 8), server router + state tests, NLT failure via `NltClient::for_test`, guards, `/today` by date parameter (the clock is `local_today()` only), `access_email`. UI host tests and Playwright specs belong to the S/E/H units. Mutation sweep after P1 (Task 9).

**§15 Rollout** — D1 first, then P1–P4 as nine version-bumped PRs with measured sizes (gap 2); rule files land with their units (`bible-import.md` D1, `sermon-prep.md` P1 + every later unit, the composer section P2a); docs with P4d.

## Spec gaps found while planning

1. **§5.1 misses a sixth defect.** KJV paragraph lines between verses are appended to the previous verse: 35 verses — PSA 119's 21 acrostic headings ("…utterly. ב BETH.") and 14 epistle subscriptions, Romans to Hebrews (HEB 13:25 = "Grace be with you all. Amen. Written to the Hebrews from Italy by Timothy." on dev today). D1 fixes it (same defect class, and it must precede D2a's KJV content digest); D2a should map those lines to `Heading` segments rather than drop them.
2. **§15 sizes are ≈ 2.1× low.** Measured (rustfmt-formatted, non-blank, non-comment, tests included): D1 ≈ 390, P1 ≈ 636, P2 ≈ 1,100, P3 ≈ 720, P4 ≈ 2,200 — ≈ 5,050 lines vs ≈ 2,900 estimated; the DTO layer and SeaORM's verbosity dominate. Re-cut into D1, P1, P2a, P2b, P3, P4a–P4d (nine PRs to M1). P3 stays ≈ 720: generation's only production caller is the commit path, whose only P3 caller is the character-limit pass — any split leaves dead code at a PR head (`-D warnings`). If the owner prefers fewer release cycles, P2a+P2b (≈ 1,100) and P4b–P4d (≈ 1,540) can each ship as one PR by waiving the cap.
3. **§7.1 "entities in `entities.rs`"**: that file is at 927 production lines (not in the §4 near-cap list); the sermon entities go to `entities/sermon.rs`, re-exported as `entities::{bible_sermon, bible_sermon_item}`.
4. **§7.1 `sermon_date` wiring**: the spec adds it to the core types and the UI DTOs in P2 but never mentions the server's router DTOs (`BiblePresentationSummaryDto` / `BiblePresentationDetailDto`), without which it never reaches the wire. The plan adds it there in P4b and leaves the UI DTOs to H1, their first consumer (§15 calls P1–P4 "server only").
5. **§11 query casing**: the intro says query parameters are snake_case, the table writes `DELETE …?expectedRevision=`. The plan accepts both (`#[serde(rename = "expectedRevision", alias = "expected_revision")]`).
6. **§12 status choice**: "TargetNotFound → 422 or bad_request_message → 400" is open-ended; the plan uses one state-layer `SermonRefusal` → 422 for every refusal the state decides (repository-error-pattern rule: a state refusal gets its own type, not `RepositoryError`), and 400 only for malformed requests decided in the router.
7. **§7.5/§12 warnings after a reload**: `SermonDetail.warnings` (NLT failed) exists only on the write answer; a `GET` cannot know it, so after a reload the editor loses the "Obnoviť slajdy" prompt unless it infers it (secondary set + a kept-verse slide without secondary text). Persisting it would need a column. Decide in E2/E3.
8. **§14 test placement**: "reorder with a different id set → Conflict" is listed under persistence, but the plan's repository takes the full new item list (simplest correct transaction), so the stale-set check and its test live in `reorder_sermon_items` (state, Task 34).
9. **§8.4 `prepared_references(date)`** is assigned to no §15 unit; with no consumer before H1 / part C it would be dead code in M1 — left to H1 (`GET /bible/sermons/today` already exposes the same data).
10. **§7.3 small decisions made explicit**: an end cut trims `, ; : – —` also when the verse gets a trailing ellipsis (decided default 4 says "an end cut removes …"); a wholly struck item's suffix names the whole source range so the card keeps a label (the table says "—").
11. **§9 Access e-mail [unverified] stays unverified in M1**: dev2 sits behind the church egress bypass (no header), so only the preacher's first write from home can confirm it.
12. **T1 numbers**: dev's DB has 548 SEB rows matching `GLOB '* [.,;:!?]*'` (spec: 490), plus 5 ROH and 3 SEVP rows. After D1: SEB 0, ROH 0 (simulated on the archives); SEVP keeps its 3 (the rule is MySword-only per §5.1).
13. **T8 on dev**: dev2's `presenter-dev.service.d/gst-debug.conf` filter names `presenter::bible::show` but not `presenter::bible::sermon`, so the sermon lines are checked on SNV with a test sermon that is deleted at once (Tasks 29, 32, 36).
