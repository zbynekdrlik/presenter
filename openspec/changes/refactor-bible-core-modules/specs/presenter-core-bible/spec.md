## ADDED Requirements
### Requirement: Bible core is modularised
The Bible domain in `presenter-core` MUST be split into focused submodules so no single source file exceeds the 800-line quality gate threshold and responsibilities stay isolated.

#### Scenario: Bible reference helpers live in dedicated module
- **GIVEN** the workspace is built from a clean checkout
- **WHEN** the crate `presenter-core` is compiled
- **THEN** `crates/presenter-core/src/bible/reference.rs` contains the reference parsing/formatting logic previously in `bible.rs`
- **AND** `cargo fmt` keeps each module under the 800-line guideline

#### Scenario: Bible ingestion helpers live in dedicated module
- **GIVEN** the workspace is built from a clean checkout
- **WHEN** the crate `presenter-core` is compiled
- **THEN** `crates/presenter-core/src/bible/translation.rs` exposes `BibleTranslation`, ingestion batch types, and default translation specs via the `bible` facade
- **AND** downstream crates compile without changing their public imports

### Requirement: Bible facade preserves public API
The `bible` module MUST continue exporting the same public types and helpers so other crates compile without source changes.

#### Scenario: Core facade re-exports existing symbols
- **GIVEN** `cargo check` runs for the entire workspace
- **WHEN** the new module layout is in place
- **THEN** code that previously referenced `presenter_core::bible::BibleTranslation` (and other existing types/functions) compiles without edits
- **AND** no compilation errors report missing items from the `bible` module
