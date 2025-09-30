# ADR 0002: Bible Source Pipeline

## Status
Accepted – 27 September 2025

## Context
Presenter needs offline Bible content for scripture slides and tablet workflows. The scraping pipeline must deliver reliable verse segmentation, support multilingual translations, and integrate cleanly with our Rust domain model. We also require a distribution-friendly format (no dynamic HTML parsing) to keep ingestion deterministic and testable.

## Decision
Adopt a mixed-source ingestion pipeline: pull the King James Version (KJV) from eBible's USFM archive (`eng-kjv_usfm.zip`), load the Slovenský ekumenický preklad from the MySword SQLite bundle, and ingest the Slovenský Roháčkov as well as the Slovenský evanjelický translations from Obohu's SQLite exports. This gives the team a complete multilingual baseline while the remaining Czech sources are evaluated under issue #5.

## Rationale
- eBible maintains USFM exports for many translations, allowing us to parse structured verse markers instead of brittle HTML scraping.
- USFM maps directly to our ingestion model: markers for book (`\id`), chapter (`\c`), and verse (`\v`) align with `BibleReference` construction.
- Starting with KJV provides a legally straightforward baseline because it is in the public domain, letting us validate performance and schema decisions before adding licensed Slovak/Czech texts.

## Consequences
- The default release now seeds four translations (KJV, Slovenský ekumenický, Roháčkov, Slovenský evanjelický) so every operator workflow can exercise real Slovak content end-to-end.
- Our default translation spec ships with multiple providers: USFM for KJV, MySword SQLite for SEB (with Slovak book-name overrides), and Obohu SQLite archives for ROH/SEVP.
- Integration tests rely on synthetic archives for each format (USFM, MySword, Obohu) so we can keep the pipeline deterministic; production ingestion still requires outbound HTTPS access to download/source the live archives.
- A dedicated reset script (`scripts/dev/refresh-dev-data.sh`) now nukes the SQLite database and re-runs both the ProPresenter import and Bible ingestion so developers can recover from failed runs quickly.

## Follow-up
- Extend coverage with the remaining Czech translations (issue #5) once licensing is clarified.
- Extend the parser to handle richer USFM markers (poetry, footnotes) and confirm formatting for stage displays.
- Document ingestion tooling (CLI/API) so operators can refresh translations without redeploying the application.
