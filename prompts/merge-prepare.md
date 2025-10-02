**/merge prepare**

1. Review every change in the branch; remove abandoned experiments, debug logs, and stray files.
2. `git status` → commit anything pending (`git add -A && git commit -m "..."`).
3. Audit added files; delete or relocate anything that was only for temporary debugging or planning.
4. Re-run through all Markdown docs (`rg --files -g"*.md" | xargs $EDITOR`) to confirm each file is up to date; edit only where behaviour or instructions have changed.
5. Update the pull request—create it if none exists, or edit the draft (`gh pr create ...` / `gh pr edit --title "..." --body-file ...`) with an accurate summary and the latest test evidence.
6. Push the branch (`git push`).
