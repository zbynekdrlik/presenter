# Placeholder Detection Gate — Design Spec

**Issue:** #200
**Date:** 2026-04-06

## Problem

Placeholder text like `"coming soon"` can slip into production code and survive CI undetected. This happened with the Settings page which displayed `"Settings interface — coming soon."` for months.

## Solution

Add a placeholder detection check to the existing `scripts/dev/quality-check.sh` script. The check uses ripgrep to scan production Rust/TypeScript source files for known placeholder patterns and fails the build in `--strict` mode.

## Patterns Detected (case-insensitive)

| Pattern | Rationale |
|---------|-----------|
| `coming soon` | Always indicates unfinished UI |
| `not implemented` | Always indicates stub code |
| `TODO` | Unfinished work marker (comments excluded) |
| `FIXME` | Known-broken marker (comments excluded) |
| `HACK` | Workaround marker (comments excluded) |

**Not detected:** The word `placeholder` is excluded despite the issue mentioning it. The codebase has 30+ legitimate uses (HTML `placeholder=` attributes, CSS `::placeholder`, proto enum values, variable names like `placeholder_bible_reference`). Filtering these reliably adds complexity without proportional value.

## Exclusion Rules

The check skips:

| Exclusion | Reason |
|-----------|--------|
| `**/tests.rs`, `**/tests/*.rs`, `tests/e2e/**` | Test fixtures may legitimately reference these patterns |
| `**/vendor/**` | Third-party proto files we don't control |
| `*.css` | CSS uses `/* */` comments which break line-based heuristics |
| `*/presenter-migration/src/*.rs` | Migration files are declarative |
| `docs/**`, `*.md` | Documentation and specs legitimately discuss these patterns |
| Lines matching `^\s*//`, `^\s*\*`, `^\s*#` | Comment lines — TODOs in comments are acceptable if tracked as issues |
| `scripts/dev/quality-check.sh` | The script itself contains the pattern strings |

## Integration Point

New check #10 in `scripts/dev/quality-check.sh`, after the existing dependency/security check (#9). Uses the existing `fail()` function for hard failures in `--strict` mode. Output shows each match with file:line for easy navigation.

## Verification

1. Run `./scripts/dev/quality-check.sh --strict` — must pass with no false positives
2. Temporarily insert `"coming soon"` in a Rust source file — must fail
3. CI pipeline runs the script automatically on every push to `dev`
