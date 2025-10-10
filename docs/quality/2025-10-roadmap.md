# Quality Roadmap — October 2025

We hardened the review gate (#41) and opened the following workstreams to bring the codebase back under policy:

| Area | Issue | Summary |
| --- | --- | --- |
| Persistence repository | #44 | Split the 1.9k-line repository module into focused submodules. |
| App state & settings UI | #45 | Modularise the server state and operator/settings views to keep files under 800 lines. |
| Bible crate | #46 | Break `presenter-bible` into self-contained modules with documented APIs. |
| Clippy baseline | #47 | Run `cargo clippy -D warnings` and fix every diagnostic. |
| Licence policy | #48 | Add repo-level `cargo-deny.toml` and resolve licence/duplicate warnings. |
| Security advisories | #49 | Upgrade or replace crates flagged by `cargo audit`. |

## Interim scaffolding

- `cargo-deny.toml` and `audit.toml` are now checked in with minimal policy; these will be tightened as the above issues land.
- The quality script remains in advisory mode for these areas to avoid blocking current work.

## Next steps

1. Tackle #44 first — once persistence is modular, clippy output and size caps become manageable.
2. Execute #45/#46 in parallel feature branches (both touch large areas, so coordinate merges).
3. When the structural refactors land, enable `cargo clippy -D warnings` in CI and remove the advisory warnings from `scripts/dev/quality-check.sh`.
4. Finalise #48/#49 by upgrading dependencies and shrinking the ignore lists.

Document updates tied to these issues should reference this roadmap until the backlog is cleared.
