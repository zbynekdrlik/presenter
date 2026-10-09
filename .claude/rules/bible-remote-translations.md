---
paths:
  - "crates/presenter-server/src/bible_remote/**"
  - "crates/presenter-server/src/state/bible_source.rs"
  - "crates/presenter-server/src/state/bible_source/**"
  - "crates/presenter-server/src/state/bible.rs"
  - "crates/presenter-server/src/state/bible_manager.rs"
  - "tests/e2e/bible-nlt-translation.spec.ts"
---

# Remote Bible translations — the NLT from Tyndale's API (#826)

## Why it is remote

Every other translation is a local file ingested into `bible_passages`. The New
Living Translation (`eng-nlt`, Tyndale House) is copyrighted and this repo is
PUBLIC, so it can never be a committed file. The owner (non-commercial church
use) chose Tyndale's own NLT API, `https://api.nlt.to`, which exists so apps can
DISPLAY NLT text: passages are fetched on demand and cached in memory only.

- **Never bulk-copy the NLT** (no "download it all into SQLite", no scraping of
  BibleGateway & co.) and **never commit NLT text** beyond the few-verse parser
  fixtures in `bible_remote/fixtures/` (three samples, ~10 verses, attribution
  in their header). The parser and dispatch tests may read those fixtures;
  every other test (wiremock pages, the E2E mock) uses synthetic words in the
  API's markup.
- Tyndale's attribution rule is the `(NLT)` after every quotation — the
  existing `(CODE)` reference suffix (`translation_short_code("eng-nlt")`).

## One dispatch — `state/bible_source.rs`

Every verse and book/chapter read goes through `AppState`:
`list_bible_translations`, `bible_passage_range`, `find_bible_passage`,
`bible_book_chapter_summaries` / `list_bible_books`,
`search_bible_passages_cross`. `bible_remote::is_remote_translation(code)`
picks the API, everything else the repository. `generate_bible_slides`,
`trigger_bible_passage`, the `/bible/*` router, the Companion trigger and the
AI tools all call these methods.

- **Never call `self.repository.bible_passage_range` / `find_bible_passage` /
  `bible_book_chapter_summaries` / `list_bible_translations` from a new call
  site** — the NLT would silently be missing there. Grep for them after any
  Bible change; only `bible_source.rs` may use them.
- The NLT has NO DB rows. Its books, chapters and verse counts come from
  `eng-kjv` (`structure_source`), and so does every NLT passage's book NAME
  (`structure_book_name`, KJV's chapter 1:1 row: "Psalms", "Song of Songs").
  That is what makes the #824 secondary label read `1 John 1:1-3 (NLT)`.
- `eng-nlt` is listed only when `eng-kjv` is installed, and AFTER the installed
  translations — index-based E2E specs (`selectOption({ index: N })`, `nth(1)`)
  keep their meaning.
- Search never covers the NLT (no local text): `Some("eng-nlt")` → empty, no
  request.
- `find_bible_passage` matches single verses only (NLT passages are single
  verses) — a multi-verse reference is `None` without a request.

## The client — `bible_remote/client.rs`

`GET {PRESENTER_NLT_API_URL}/api/passages?ref=<Book>.<ch>.<start>-<end>&version=NLT&key=<PRESENTER_NLT_API_KEY>`

- Limits: anonymous `TEST` key ≤50 verses/request, ≤500 requests/day; a
  registered key ≤500 verses, ≤5000 requests. Ranges go out in ≤50-verse chunks
  (`MAX_VERSES_PER_REQUEST`), a fetch is capped at 200 verses
  (`MAX_VERSES_PER_FETCH`, Psalm 119 = 176), 10 s timeout per request.
- **The API CLAMPS instead of failing**: `1Jn.1.12-15` answers verse 10 (the
  chapter's last), a missing chapter answers the book's last chapter. Every page
  is filtered to the requested chapter + range, and a chunk with nothing in range
  ends the fetch (the chapter is over).
- An unknown book abbreviation answers 200 with an EMPTY page → `NoVerses`.
  Book abbreviations (`books.rs`) were checked live: the API wants `1Thes`/`2Thes`
  and `1Jn`/`2Jn`/`3Jn`, not the OSIS `1Thess`/`1John`.
- Cache: bounded LRU of chunks (256), keyed by (book_code, chapter, start, end),
  in memory only; a failed request is never cached.
- Fail fast: after a `Timeout`/`Network` failure the client answers
  `RemoteBibleError::Backoff` for 30 s (`UNREACHABLE_BACKOFF`) without asking the
  API, so with the venue's internet down each load degrades at once instead of
  waiting out the 10 s timeout. Any API answer ends the pause; HTTP-status
  failures (`Status`, `NoVerses`) never start it. The cache lock is a
  `std::sync::Mutex` that must never be held across an `.await`.
- The request URL carries the key: never log the URL; transport errors go through
  `describe()` (`reqwest::Error::without_url`).
- Logging: every request at INFO (ref, verses, status, elapsed_ms), cache
  hit/miss at DEBUG, failures at WARN with the error.

## Parser — `bible_remote/parse.rs`

One `<verse_export ch= vn=>` = one verse. Text starts after
`<span class="vn">N</span>`; everything before it (chapter heading, `h3`/`h4`
section headings, Psalm 119 Hebrew letters, `p.psa-title`) is dropped. Footnote
bodies `span.tn` nest spans → cut at the MATCHING `</span>` (balanced scan, not
a regex). `span.sc` (small-caps `Lord`) → `LORD`, as eng-kjv writes it. Block
tags (`p`, poetry lines) → a space, inline tags → nothing, entities decoded,
whitespace collapsed. No HTML-parser crate: the markup is flat and generated.

## Errors → HTTP

`RemoteBibleError` (`bible_remote/mod.rs`) is mapped in the router's central
`From<anyhow::Error> for AppError`: `Timeout`/`Network`/`Backoff` → 503,
`Status`/`NoVerses` → 502, with the Slovak "NLT nedostupné — API/internet: …" message. The UI's
`resolve_slides` uses `post_json_detail`, so the "Failed to load passage" toast
shows that message.

That hard failure is only for the NLT as MAIN translation (nothing to show).
An unavailable SECONDARY translation (any `RemoteBibleError`: unreachable, or an
unusable answer such as a bad key / spent daily quota) never fails a load — a
remembered NLT secondary would otherwise kill every Bible load of a service the
moment the internet drops. (The first load after the drop still waits for the
request timeout; the fail-fast pause makes the next ones immediate.)

- `generate_bible_slides` (`secondary_verse_lookup`) catches a
  `RemoteBibleError` from the secondary fetch, composes main-only slides and
  returns `GeneratedBibleSlides::secondary_warning`; `/bible/resolve` sends it as
  `warning` and the Bible page shows it as an error toast on every load
  (`load_passage`). Any OTHER secondary error still fails the load.
- The legacy `trigger_bible_passage` logs an unreachable secondary and leaves it
  out — the main passage still goes on air.

## Chapters the NLT numbers longer than eng-kjv

The NLT's verse counts are eng-kjv's, except the chapters in
`bible_remote::NLT_LONGER_CHAPTERS` (`nlt_structure`, applied by
`bible_book_chapter_summaries` for `eng-nlt`): 3 John 1 has 15 verses in the
NLT (14 in the KJV) and Revelation 12 has 18 (17) — both checked live. Without
the override a whole-chapter load with the NLT as MAIN (resolve takes
`verse_end` from the structure) would drop that last verse. If another such
chapter turns up, add it to the table (check the KJV count on prod read-only
and the NLT with one API request); the NLT as SECONDARY follows the main
translation's range and is unaffected either way.

## Env + tests

- `PRESENTER_NLT_API_KEY` (default `TEST`), `PRESENTER_NLT_API_URL` (default
  `https://api.nlt.to`).
- Rust tests point a client at wiremock via `NltClient::for_test` +
  `AppState::set_test_nlt_client` — never via env (process-global, parallel
  tests race).
- `tests/e2e/support.ts` defaults `PRESENTER_NLT_API_URL` to a dead loopback
  (`http://127.0.0.1:1`); `bible-nlt-translation.spec.ts` overrides it with its
  own mock server before `startTestServer`.
