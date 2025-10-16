# Design: Bible core module split

## Current state
- `crates/presenter-core/src/bible.rs` contains >1,000 LOC covering:
  - Reference parsing/formatting utilities (`BibleReference`, `BiblePassageRange`).
  - Canon metadata, canonical lookups, and accent-folding helpers.
  - Translation ingestion structs (`BibleTranslation`, `IngestionBatch`) and validation.
  - Search tokenisation helpers shared with library lookups.
  - Embedded default-spec data and tests.
- Tests live inline, which hides intended responsibilities and increases compile times.

## Target structure
```
crates/presenter-core/src/bible/
├── mod.rs              # Public facade + re-exports; <200 LOC
├── reference.rs        # Reference structs, validation, formatting
├── canonical.rs        # Metadata, canonical order, accent folding
├── translation.rs      # Translation structs, default specs, ingestion validation
├── search.rs           # Tokenisation helpers used by search APIs
└── tests/
    ├── mod.rs          # Shared fixtures/helpers
    └── *.rs            # Split test modules by concern (reference, ingestion, search)
```

### Module responsibilities
- `reference.rs`: `BibleReference`, `BiblePassage`, range parsing, fmt/display.
- `canonical.rs`: `BOOK_ALIASES`, canonical lookup maps, normalisation.
- `translation.rs`: `BibleTranslation`, ingestion batch handling, default translation specs (moved to constants / lazy statics as needed).
- `search.rs`: `fold_query`, `query_tokens`, other search helpers currently in bible.rs.

`mod.rs` exposes the same API via `pub use` to avoid widespread downstream changes. Internal-only helpers become `pub(crate)` within the module tree, reducing accidental coupling.

## Compatibility
- Re-export types so existing imports (`use presenter_core::bible::{BibleReference, BibleTranslation, fold_query}`) continue to resolve.
- Update crate feature declarations or `pub use` in `lib.rs` if the module path changes.
- Ensure serde derives continue to function (keep module paths stable for generated names).

## Testing
- Move inline tests into `tests/` submodules to keep production modules slim.
- Cover each module with unit tests equivalent to the existing suite.
- Add integration-style tests (if helpful) to verify the re-exports compile; optionally add a smoke test in `presenter-server` or `presenter-persistence` if current coverage misses regression holes.

## Risks & Mitigations
- **Risk:** Missed re-export causing compilation error downstream. *Mitigation:* `cargo check` across workspace, add compile-time tests referencing the facade.
- **Risk:** Module split introduces cyclic dependencies. *Mitigation:* Keep helper functions in `canonical.rs` or `search.rs`, avoid referencing `translation.rs` from `reference.rs`.
- **Risk:** Test relocation removes coverage. *Mitigation:* Mirror current assertions verbatim; ensure `cargo test -p presenter-core` passes.

## Follow-up considerations
- After refactor, evaluate moving ingestion logic closer to persistence if needed, but outside the scope of this change.
- Once module boundaries stabilize, consider deriving documentation comments for external use (API docs).
