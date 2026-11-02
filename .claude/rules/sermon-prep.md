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
  - "crates/presenter-ui/src/pages/study/sermon*.rs"
  - "crates/presenter-ui/src/pages/study/add_to_sermon.rs"
  - "crates/presenter-ui/src/state/sermon_*.rs"
  - "crates/presenter-ui/src/api/sermons.rs"
  - "tests/e2e/sermon-*.spec.ts"
  - "crates/presenter-ui/src/pages/bible_console/prepared.rs"
  - "crates/presenter-ui/src/state/bible_console_prepared.rs"
  - "crates/presenter-ui/src/pages/bible_prepared_sermon.rs"
  - "crates/presenter-ui/src/pages/tablet/sermon_select.rs"
  - "tests/e2e/sermon-handoff.spec.ts"
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
- **A write's shape** (`write.rs`, `maintain.rs`): `load_sermon` (404) → `check_revision` (409 BEFORE
  any snapshot read) → compute the new item list in memory → `commit_and_view` (generation +
  the in-transaction revision check + the detail with the write's warnings).
- **`snapshot_item(ItemRange)`** checks, in this order: range sane → ≤ 60 verses (`end - start`
  only after `end ≥ start`, no underflow) → not the NLT → translation installed → book code
  canonical → the rows cover exactly `start..=end` (first `verse_start`, last `verse_end`).
  One chapter per item: the client splits a cross-chapter selection.
- `PATCH …/items/{itemId}`: `refreshSource: true` wins over `translation`; neither → 400. A
  translation change re-snapshots and CLEARS the item's edits (the editor confirms first); a
  refresh keeps the edits of verses whose text is unchanged (`retain_unchanged_edits`). Both
  keep the item id (and so its slide ids).
- `POST …/items/reorder` needs EXACTLY the stored item set; a different set (missing, extra,
  duplicate id) is a stale view → `Conflict(SERMON_CONFLICT)` → 409, never a partial reorder.
- `POST …/regenerate` = the same items, the slides generated again (retries the secondary line
  after an NLT failure).

## Secondary line and outages (review of the server stack, 2026-11-01)

- Prod's Bible preference has the NLT as secondary, so its paths are the normal case. Lookups run
  concurrently over the item's FULL range (stable remote cache key), kept verses are filtered after.
- An outage never takes a stored line: `stored_secondary` reads the sermon's slides back
  (`"{verse}. {text}"` lines, book from the stored label) and fills what the lookup lost; the
  warning stays only when a kept verse is still missing. The server's own pass
  (`origin.is_system()`) then refuses to store: a person's write stores what it has, and
  `SermonDetail.secondaryMissing` carries the state across a reload and to other devices.
- A secondary equal to the item's translation adds no line (the full verse would bring struck words back).
- `rename_bible_presentation` bumps a sermon's revision in the same transaction: a write loaded
  before a rename gets its 409 and never restores the old title.
- `delete_bible_presentation` on a sermon goes through `delete_sermon` (the `delete` log line).
- Sermons do not sync between SNV and PP: they live on the site where they were written.

## The study page's sermon views (E1, E2, `pages/study/{sermons,add_to_sermon,sermon_editor,sermon_item}.rs`)

- Views: `StudyView::{Sermons, Sermon { id }}`. The add sheet `s=pridat` never comes from a URL: the
  passage it asks about lives only in `Study::pending_passage`.
- `study.activeSermon` (localStorage) = where "Pridať do kázne" goes. A 404 forgets it and asks.
- **All writes share the sermon's revision, so they run one at a time** (`Editor::busy`): an edit 300 ms
  after the last tap, a structural write through `write()`, which first waits for the edits
  (`settle_saves`). `Editor::apply` keeps a dirty item's local taps when the server's answer arrives.
- **The page owns the editors** (`editor_for`: one per opened sermon, created under `Study::owner`). A view's
  cleanup must not reach the queue: it cancelled the 300 ms timer (a tap lost) and disposed the values an
  answer on its way reads (`get_value` / `with_untracked` on a disposed value PANICS in release; `set` /
  `update` do not). `online`, `pagehide`, `visibilitychange` and `beforeunload` are page-wide (`install`).
- Every write goes through `take_turn` / `end_turn`, Späť included; `end_turn` flushes, or a tap made
  during a write waits for the next tap. `apply` ignores a LOWER revision (a slow GET would take back a
  saved edit). A 404 on a save leaves the queue and reloads; only a sermon 404 is `Missing`.
- Späť after a delete re-adds through the server, which snapshots TODAY's text: `retain_unchanged` keeps
  only the edits of verses still reading the same, restored as a normal edit (it retries like one). A
  failed Späť keeps Späť on screen; a `tell` during its 5 s waits in `Study::later` for the toast to end.
- **E3:** Náhľad and Poradie are sheets with URLs (`s=nahlad`, `s=poradie`) over the sermon view (`open_editor`).
  The order is `Editor::order` (`OrderDraft { base, ids }`), not the sheet's: sent once as the sheet LEAVES
  (`Owner::on_cleanup` + `send_order`), whichever way it closes, kept after a failure for
  `sermon-order-retry`. **The socket keeps the revision fresh, so a revision check cannot catch a stale
  order: `send_order` compares the stored order with `base` and refuses by itself.** HOTOVO goes only when
  everything is stored and names what was refused (`Editor::refused`). A drag = pointer events on a `touch-action: none` handle, the pure order
  logic is the plate list's (`nameplates::order`, generic). The socket (`ws::passive`, surface `study`) opens
  with the first sermon, never for plain reading; a sermon's id IS its presentation's. A test that needs a
  409 must race the write itself (`page.route` on the PUT): a REST write beforehand reaches the open editor
  through the socket first. An event during a write of ours waits for it (`Editor::behind`); a failed
  background reload keeps the sermon on screen. Accepted: a closing tab sends one PUT (the next item's
  needs the first's new revision), the socket stays once opened.
- **Edits go as a `keepalive` fetch with a 15 s abort** (`start_keepalive`): the browser cancels a plain fetch
  as a tab closes. It answers `ApiError::Status`, so a refused edit shows no server text.
- 409 on an edit: same item, same text → re-sent once; anything else is not replayed (spec §7.6).
- The words of a card share one listener (`token_at`), never one per token: a chapter has thousands.
- A new sermon takes the translation being read as its default; the NLT never (`snapshot_translation`).
- `sermon_error`: only `ApiError::Refused` shows the server's text, so sermon calls use the `*_refusal`
  helpers.
- **A refusal lives until HOTOVO has named it (or the sermon is opened again):** a later landed write never clears `Editor::refused`, or HOTOVO would say "Kázeň je uložená" over a change that was thrown away.
- **Hand-off (H1):** "Pripravené kazateľom" is `pages/bible_console/prepared.rs` + pure `state/bible_console_prepared.rs`. A pick (`Console::prepared`) replaces `target`: `target_key` = `prepared:<sermon>:<revision>`, the preview holds ALL the sermon's slides and a step only moves its index; no `/bible/resolve`, stored text is the truth even after a translation switch. ZOBRAZIŤ goes through `/bible/presentations/{id}/slides/{slide}/trigger` (log `source=prepared`). Typing and an old-inputs load let go of the pick; a refresh follows the slide id. A sermon keeps ids and count when it is cut, so the tablet compares the open sermon's slides on its 10 s poll.
