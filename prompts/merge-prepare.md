**/merge prepare**

1. Review every change in the branch; remove abandoned experiments, debug logs, and stray files.
2. `git status` → commit anything pending (`git add -A && git commit -m "..."`).
3. Audit added files; delete or relocate anything that was only for temporary debugging or planning.
4. Ensure the branch reflects the latest `main` (run `/sync-with-main` and resolve any conflicts).
5. Re-run through all Markdown docs (`rg --files -g"*.md" | xargs -n1 -I{} ${EDITOR:-vi} "{}"`) to confirm each file is up to date; edit only where behaviour or instructions have changed.
6. Update the pull request—create it if none exists, or edit the draft (`gh pr create ...` / `gh pr edit --title "..." --body-file ...`) with an accurate summary and the latest test evidence.
7. Push the branch (`git push`).

8. Tests must be comprehensive, with E2E as priority:
   - Run `cargo test` and `npm run test:playwright` (no flake retries).
   - Reject focused/skipped tests: `rg -n "\.only\(|describe\.only|it\.only|test\.only|\.skip\(" tests/e2e` must return nothing.
   - Ensure E2E covers UI plus side effects for ALL workflows changed in this branch (use mocks where needed). Examples: Resolume, Ableton/AbleSet/OSC, Android Stage, Companion, gateway, stage layouts, Bible imports.
   - Re-run `sudo -E ./scripts/dev/verify-and-refresh.sh` and confirm demo + gateway validations pass.

9. Quality & Architecture Review (Issue #41 rules):
   - Run `scripts/dev/quality-check.sh --strict --against origin/main` and resolve all reported failures. Warnings should be triaged in the PR notes with follow-up TODOs.
   - Ensure the PR description includes the “Definition of Done” checklist from `docs/issues/41-recurring-quality-architecture-review.md` and each item is checked.
   - If architectural changes were introduced, add/update an ADR under `docs/adr/` and link it in the PR.
