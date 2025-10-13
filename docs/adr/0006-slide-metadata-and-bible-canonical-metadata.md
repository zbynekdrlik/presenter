# ADR 0006: Persist Slide Metadata JSON and Canonical Bible Metadata

Date: 2025-10-11

Status: Accepted

## Context

The Bible operator workflow needs slide metadata that round‑trips through
presentations: custom reference labels edited in the UI and canonical book
identifiers used by external systems (gateway, stage, OSC/Resolume).

Previously, slide metadata was not persisted with the presentation slides, and
Bible passages returned by persistence sometimes lacked canonical `bookCode` and
`bookNumber`, leading to missing fields in E2E expectations and downstream
integrations.

## Decision

- Persist slide metadata as JSON in the `slides` table (`metadata_json` column)
  and hydrate it on read. This preserves UI‑edited fields (e.g., main/translation
  reference labels) across presentation saves and reloads.
- When mapping `bible_passages` to domain `BibleReference`, include canonical
  `book_code` and `book_number` when stored; otherwise derive from the canonical
  map by book name during ingestion. Slide metadata emitted by the server
  includes `bookCode`/`bookNumber` so operators and integrations receive
  consistent identifiers.

## Consequences

- The initial migration already contains `slides.metadata_json`; no new
  migrations are added (pre‑release policy). Entities and repositories were
  updated to read/write this column.
- E2E “operator manages Bible workflow end‑to‑end” now passes, and presentation
  detail reflects custom reference labels edited in the UI.
- Downstream consumers may rely on `bookCode`/`bookNumber` being present in
  Bible slide metadata going forward.

## Alternatives Considered

- Reconstruct reference labels at render time only: rejected because UI edits
  would be lost once slides are persisted.
- Store per‑field columns: rejected for flexibility; metadata evolves across
  workflows and JSON keeps schema stable while remaining queryable if needed.

## Testing

- Added/updated unit tests in persistence to assert canonicalization.
- Existing Playwright E2E for the Bible operator flow passes without flake.
