**/merge prepare**

1. Review every change in the branch; remove abandoned experiments, debug logs, and stray files.
2. `git status` → commit anything pending (`git add -A && git commit -m "..."`).
3. Audit added files; delete or relocate anything that was only for temporary debugging or planning.
4. Ensure the branch reflects the latest `main` (run `/sync-with-main` and resolve any conflicts).
5. Re-run through all Markdown docs (`rg --files -g"*.md" | xargs $EDITOR`) to confirm each file is up to date; edit only where behaviour or instructions have changed.
6. Update the pull request—create it if none exists, or edit the draft (`gh pr create ...` / `gh pr edit --title "..." --body-file ...`) with an accurate summary and the latest test evidence.
7. Push the branch (`git push`).

8. Tests must be comprehensive, with E2E as the priority:
   - Run `cargo test` and `npm run test:playwright` (no flake retries).
   - Fail the review if any `.only` or `.skip` is present: `rg -n "\.only\(|test\.only|describe\.only|it\.skip|test\.skip|describe\.skip" tests/e2e` must output nothing.
   - For integration features, assert E2E covers UI + side effects: AbleSet status (mock `/api/setlist`), OSC `/note` reception, and stage snapshot updates.
   - Re-run `sudo -E ./scripts/dev/verify-and-refresh.sh` and confirm the demo + gateway validations pass.
