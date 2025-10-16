# Refactor presenter-core Bible modules

## Summary
Break `crates/presenter-core/src/bible.rs` into purpose-focused submodules so the Bible domain is easier to maintain and respects the 800-line soft limit from the quality gate (GitHub issue #74).

## Motivation
- The current monolithic file is 1,009 lines, repeatedly tripping Issue #41 warnings and making code reviews slow.
- Bible domain concerns (reference math, translation metadata, ingestion helpers, search utilities) are tightly coupled, hiding invariants and test seams.
- Upcoming Bible UI and API work will need clearer boundaries to reuse logic without copy/paste.

## Scope
- Introduce a `crates/presenter-core/src/bible/` module directory with submodules for references, canonical metadata, ingestion, and search helpers.
- Maintain the existing public API by re-exporting types/functions from a small `bible/mod.rs`, keeping downstream crates stable.
- Update unit tests and imports to follow the new module structure.
- Document the new structure in `docs/` if necessary.

## Out of Scope
- Changing Bible feature behaviour (UI rendering, repository APIs, data ingest formats).
- Adjusting database schemas or seed data.
- Refactoring unrelated presenter-core modules.
