# Presenter Operations Runbook

This runbook captures the day-to-day commands for operating the Presenter environments on the Intel N100 controller. All paths assume the repository lives at `/home/newlevel/marek/presenter`; adjust if you relocate the checkout.

## Host Bootstrap

For any fresh workstation or controller, run the provisioning helper before touching the services:

```bash
cd /home/newlevel/devel/presenter/presenter-dev1   # or any checkout in /home/newlevel/devel/presenter
./scripts/ops/bootstrap-host.sh
```

The script installs build essentials, browser/runtime libraries, the Rust toolchain, and Playwright browsers. Re-run it after OS upgrades or when bringing additional dev folders online so every env stays consistent.

## Environment Matrix

| Environment | Service Name             | Port | Database Path                                      | Purpose |
|-------------|--------------------------|------|----------------------------------------------------|---------|
| Development | `presenter@dev.service`  | 80   | `var/data/dev/presenter_dev.db`                    | Auto-refreshing instance for rapid iteration; rebuilds data on every file change. |
| Testing     | `presenter@test.service` | 8081 | `var/data/test/presenter_test.db`                  | Stable validation surface mirroring production configuration. |
| Production  | `presenter@prod.service` | 8080 | `var/data/prod/presenter_prod.db` (override as needed) | Release build promoted after user sign-off; paired with nightly backups. |

All environments bind to `0.0.0.0`. If you need different ports or database locations, set `PRESENTER_PORT`, `PRESENTER_DB_URL`, or `PRESENTER_DATA_ROOT` in the environment before starting the service (or adjust the systemd unit overrides).

## Installing/Updating Systemd Units

```bash
sudo ./scripts/ops/setup-systemd.sh dev test      # install and start dev + test
sudo ./scripts/ops/setup-systemd.sh prod          # install production + backup timer
```

The helper copies these unit templates into `/etc/systemd/system/`:

- `ops/systemd/presenter@.service` – templated runtime entrypoint using `scripts/ops/run-env.sh`.
- `ops/systemd/presenter-backup@.service` and `.timer` – nightly backups (enabled automatically for production).

Re-run the script after repository updates to propagate service changes; it calls `systemctl daemon-reload` for you.

## Lifecycle Commands

```bash
# Development
systemctl status presenter@dev.service
sudo systemctl restart presenter@dev.service
sudo systemctl stop presenter@dev.service
journalctl -u presenter@dev.service -f

# Testing
sudo systemctl restart presenter@test.service
journalctl -u presenter@test.service -n 200

# Production
sudo systemctl restart presenter@prod.service
journalctl -u presenter@prod.service -f
sudo systemctl status presenter-backup@prod.timer
```

The development service uses `cargo watch` to rebuild automatically. Testing and production run plain `cargo run`/`cargo run --release`; restart them after pulling new commits or promoting a build.

To start the development instance without systemd (useful on laptops), run `./scripts/dev/start-demo-server.sh`. It spawns the same `run-env.sh dev` process in the background and records its PID under `.presenter-demo.pid`.

## Data Refresh & Backups

- Manual refresh of any environment:
  ```bash
  PRESENTER_DB_URL=sqlite://var/data/test/presenter_test.db ./scripts/dev/refresh-dev-data.sh
  ```
  (The dev service performs this automatically before each rebuild.)

- Snapshot backups:
  ```bash
  ./scripts/ops/db-maintenance.sh backup dev
  ./scripts/ops/db-maintenance.sh list prod
  sudo ./scripts/ops/db-maintenance.sh restore prod var/backups/prod/presenter_prod-20250928T020000Z.db
  ```

  The restore command refuses to run while `presenter@<env>.service` is active; stop the service first, restore the backup, then restart the service.

- Nightly backups: `presenter-backup@prod.timer` executes at 02:30 UTC with a ±10 minute randomized delay to stagger filesystem load. Adjust the schedule by editing `ops/systemd/presenter-backup@.timer` and rerunning the setup script.

Set `PRESENTER_BACKUP_ROOT` (e.g., `/var/lib/presenter/backups`) before calling the backup script or installing the systemd units if you prefer off-repo storage.

## Promotion Workflow

1. Merge the feature branch into `main` after review per AGENTS.md.
2. Pull `main` on the controller and restart the **testing** service:
   ```bash
   git switch main && git pull --ff-only
   sudo systemctl restart presenter@test.service
   ```
3. Run `npm run test:playwright` and `cargo test` locally or via CI; inspect logs.
4. Once the user validates the testing instance, promote to production:
   ```bash
   sudo systemctl stop presenter@prod.service
   sudo systemctl restart presenter@prod.service
   ```
5. Confirm health checks (`/healthz`) and stage display latency (<100 ms) from a client machine.

## Troubleshooting Checklist

- **Service fails to start on port 80:** ensure the unit runs as a user with `CAP_NET_BIND_SERVICE`. The templated unit already applies this capability; if you override it, re-add `AmbientCapabilities=CAP_NET_BIND_SERVICE`.
- **Auto-refresh not picking up changes:** verify `cargo-watch` is installed (`/home/newlevel/.cargo/bin/cargo-watch`). Reinstall via `cargo install cargo-watch --locked` if necessary, then restart `presenter@dev.service`.
- **Backup timer reports failures:** inspect `journalctl -u presenter-backup@prod.service` to see the underlying `sqlite3` command output. The most common cause is a missing database file; ensure the production service has run at least once to create it.
- **Restores revert immediately:** confirm systemd is stopped; `db-maintenance.sh restore` aborts if the service is running, but manual copies may still be overwritten by dev auto-refresh. Use `systemctl list-units 'presenter@*.service'` to ensure only the desired environment is active.

## File Structure Summary

```
var/
  data/         # Per-environment SQLite files managed by run-env.sh
  backups/      # Timestamped backups grouped by environment
scripts/ops/
  run-env.sh    # Entry point used by systemd template
  setup-systemd.sh
  db-maintenance.sh
ops/systemd/
  presenter@.service
  presenter-backup@.service
  presenter-backup@.timer
```

Keep this document up to date whenever the deployment process changes.
