**/sync-with-main**

Keep dev branch in sync with main:

```bash
git fetch origin main
git merge origin/main
# Resolve any conflicts
git push origin dev
gh run watch  # Wait for CI
```
