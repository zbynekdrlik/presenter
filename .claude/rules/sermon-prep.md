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
- **Everything the wall shows derives from `render_item(source, edits)`.** Once sermons are
  stored (P2b/P3), their slide text is a cache that every sermon write regenerates; never treat
  it as the truth.
- **Label rules (§7.3, decided defaults 3–5):** letters only on the EDGES of a run of
  consecutive kept verses (`b` = start cut on the run's first verse, `a` = end cut on its last;
  `16b-18a`); a cut the label cannot express (an inner verse, the inner edge of a run's first or
  last verse, one verse cut at both ends) reads `… ` / ` …`; an end cut trims a trailing
  `, ; : – —` (also before a trailing ellipsis) and never adds a period; no trailing ellipsis
  after `. ! ?`; a gap inside a verse reads ` … `; a wholly struck verse splits the label
  (`5-7, 9`); every token struck → no verses and the suffix names the whole source range (the
  editor still labels the card); emphasis = Unicode upper case ("vieru" → "VIERU").
- **`translation_short_code`** (core `label.rs`) is the shared "(SEB)" formatter; new code calls
  it, never a copy. Three older copies remain today: the live composer's private
  `translation_short_code` in `state/slides/compose.rs` (P2a switches the composer to the core
  function), an inline `short_code` in `ai/tools/bible.rs`, and `resolume/handlers.rs`'s own.
- **End cuts:** a trailing `, ; : – —` goes even when glued to the word, but a hyphen only as a
  standalone spaced ` - ` token (ROH and SEVP write the dash that way); a hyphen inside or at the
  end of a word ("Bar-Jona", "nad- a podzemné") always stays. A sentence end followed only by
  closing quotes or brackets (`.)`, `.”`, `!“`, `?»`, `.’`, `.]`) gets no trailing ellipsis.
- `MAX_ITEM_VERSES` = 60. `default_service_date(today)` = the next Sunday (today on a Sunday),
  `default_sermon_title(date)` = "Kázeň D. M. YYYY" — computed from the SERVER's date (the
  `state::sermons::local_today` helper arrives in P3), never the phone's.
- Tests are plain `#[test]` fns with inline `assert_eq!` (the quality job's assertion check reads
  `#[test]` bodies; a helper-only body fails it).
- After P1 merges: `/mutation-sweep` (`gh workflow run mutation-full.yml --ref main`); a survivor
  under `bible/sermon/` is P1 work, killed on `dev` with a test, never a follow-up ticket.

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
