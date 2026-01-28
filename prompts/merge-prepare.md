**/merge-prepare**

Prepare branch for merge to main. GitHub Actions handles all testing.

Steps:
1. `git status` → commit any pending changes.
2. Ensure branch is up to date with main:
   ```bash
   git fetch origin main
   git merge origin/main
   ```
3. Push to dev and wait for CI:
   ```bash
   git push origin dev
   gh run watch
   ```
4. Once CI is green, test locally:
   ```bash
   cargo run -p presenter-server
   # Open http://localhost:80 and verify functionality
   ```
5. Create/update PR:
   ```bash
   gh pr create --base main --head dev --title "..." --body "..."
   # Or edit existing: gh pr edit
   ```
6. Wait for user approval to merge.
