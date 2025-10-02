**/merged-prepare-new**

1. Confirm the PR has been merged (`gh pr view --json state` should report `MERGED`).
2. For each issue referenced by the PR body or commits, ensure it is closed; close lingering ones with a comment referencing the PR.
3. `git fetch origin main` → `git switch main` → `git pull --ff-only` to sync the local main branch.
4. Remove local branches whose remotes disappeared (`git branch -vv` then `git branch -d/-D <name>` as needed).
5. Fetch open issues (`gh issue list --state open --json number,title,labels,createdAt`) and present a detailed grouped report by subsystem (intercom, ndi-display, ndi-capture, pipewire, web/UX, build/image, etc.), including number, title, labels, and age.
