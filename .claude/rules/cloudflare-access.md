---
paths:
  - "ops/cloudflare/**"
---

# Cloudflare Access for the public Presenter hosts (#850 §9)

- **The repo is PUBLIC.** Only `ops/cloudflare/presenter-access.example.json` (placeholders) is
  committed; `.gitignore` refuses any other `ops/cloudflare/*.json`. The real config (account id,
  e-mails, hosts, on-site bypass IPs) is `~/.config/presenter/presenter-access.json` (0600) on dev2.
  The Cloudflare token stays in the vault file `cloudflare-presenter-access` and reaches the script
  only through `secret exec` (the vault hook refuses hand reads/writes under the vault dir, so the
  non-secret config does not live there).
- **Run — dry run first, always:**
  `python3 ~/devel/airuleset/airuleset.py secret exec --file <vault>/cloudflare-presenter-access --env CF_TOKEN -- python3 ops/cloudflare/presenter_access.py --config ~/.config/presenter/presenter-access.json`
  The live state is applied, so a clean run prints four `ok` lines. Add `--apply` only when the dry
  run shows exactly the intended `create` / `update_*` lines, then dry-run again (4× `ok`).
- **Adding a person** = one more e-mail in `allowed_emails` → the dry run shows
  `update_policy … changed=include` for both hosts → `--apply`.
- The script never deletes: a policy the config does not name is printed as `extra_policy` and kept.
- `update_app` (a session-duration change) PUTs the existing app with its policy ids kept —
  **[unverified]** against the live API (the 2026-10-13 apps were created, never updated). The first
  real `update_app` must be followed by a dry run that prints `ok`.
- `desired_apps()` / `plan()` are pure and unit-tested in `test_presenter_access.py` (CI `quality` job).
