**/quality-review**

Quality checks are run automatically by GitHub Actions on every push.

To check CI status:
```bash
gh run list
gh run watch
```

For local debugging (optional, CI does this):
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
npm run test:playwright
```

If CI fails, check the logs:
```bash
gh run view <run-id> --log-failed
```
