**/quality review**

Run the recurring Quality & Architecture Review (Issue #41 baseline) for the current branch.

Commands to run:
- `scripts/dev/quality-check.sh --against origin/main` (advisory)
- `scripts/dev/quality-check.sh --strict --against origin/main` (enforced before merge)
- `sudo -E ./scripts/dev/verify-and-refresh.sh` (full tests + demo/gateway refresh)

Then ensure your PR description contains the Definition of Done from:
- `docs/issues/41-recurring-quality-architecture-review.md`

If you introduce non-trivial architecture changes, add/update an ADR under `docs/adr/` and link it.

