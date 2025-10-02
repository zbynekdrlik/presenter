**/sync-with-main**

1. `git fetch origin`
2. If `origin/main` has new commits compared to `main`, run `git switch main && git pull --ff-only`.
3. Switch back to the current feature branch (`git switch -`) and merge main in carefully (`git merge main` or `git rebase main` per branch policy), resolving conflicts if needed.
