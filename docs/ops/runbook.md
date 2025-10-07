# Presenter Operations Runbook

This runbook captures the day-to-day commands for operating the Presenter environments on the Intel N100 controller. All paths assume the repository lives at `/home/newlevel/marek/presenter`; adjust if you relocate the checkout.

## Host Bootstrap

For any fresh workstation or controller, run the provisioning helper before touching the services:

```bash
cd /home/newlevel/devel/presenter/presenter-dev1   # or any checkout in /home/newlevel/devel/presenter
./scripts/ops/bootstrap-host.sh
```

The script installs build essentials, browser/runtime libraries, the Rust toolchain, and Playwright browsers. Re-run it after OS upgrades or when bringing additional dev folders online so every env stays consistent.

## Android Stage Launchers

- Install the Android platform tools on every controller (and CI runner that executes `verify-and-refresh`):
  ```bash
  sudo apt install android-tools-adb
  ```
- Create a shared key directory so all runtimes reuse the same trusted keypair:
  ```bash
  mkdir -p ~/.config/presenter/adb
  cp ~/.android/adbkey ~/.config/presenter/adb/  # after first successful pairing
  cp ~/.android/adbkey.pub ~/.config/presenter/adb/
  ```
  All docker stacks mount this folder at `/root/.android`, so when one environment authorizes a TV the
  others piggyback on the same keys automatically.
- If `adb` lives outside `$PATH`, export `PRESENTER_ANDROID_ADB_BIN=/custom/path/adb` before starting
  Presenter. The service falls back to the first `adb` on `$PATH` otherwise.
- Pair each Android TV for wireless debugging (`adb pair HOST:PORT PIN`, then
  `adb connect HOST:5555`). Presenter automatically retries the connection every 20 s and relaunches
  Fully Kiosk (`com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity`) once connected.
- Manage the device roster under **Settings → Android Stage Launchers**; status badges surface last
  launch, last attempt, and the most recent ADB error so operators can intervene before pre-service.

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

## OSC Bridge Quickstart

- Run `sudo -E ./scripts/dev/verify-and-refresh.sh --force` before testing a branch demo; the helper seeds an _Ableton Demo_ playlist with the first five imported presentations and leaves the OSC bridge enabled with the `/note` pattern bound to notes 18/19/20.
- AbleSet's local status API defaults to port 80. If AbleSet picked a different HTTP port (it will auto-increment when 80 is busy), check the AbleSet status card in Settings → Ableton Bridge and update the host/port fields accordingly.
- AbleSet listens for OSC control on UDP 39051 by default; keep the bridge pointed there unless you've changed it in AbleSet’s preferences.
- Each demo publishes the OSC listener on a deterministic high port. Check **Settings → OSC Bridge** or `curl http://127.0.0.1:<demo-port>/integrations/osc/status` to read the `hostPort` field. For example, the `presenter-dev2` stack listens on UDP `10.77.9.21:18532`.
- In Ableton, load the Max for Live OSC/MIDI bridge, set the target host to the controller's IP (e.g., `10.77.9.21`) and the port from the status API, and keep the default `/note` address. The plugin should emit zero-based velocity values so playlist 0, presentation 0, and slide indices line up.
- A 75 ms suppression window prevents tight Ableton loops from re-triggering the same slide; if you need intentional repeats, stagger cues or adjust the loop length accordingly.
- The operator header exposes `Automation` and `Follow UI` toggles along with the currently playing song and slide index. Leave *Follow UI* off to keep the interface steady while AbleSet still drives Resolume; re-enable it when you want the library/slide lists to follow AbleSet cues in real time.
- The verify helper restarts the gateway after each demo refresh so the landing page advertises the correct OSC port and timestamp per branch.


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

## Companion Automation Checklist

- Generate the packaged Companion profile by running `node ops/companion/generate-profile.mjs`, then import the resulting file under `ops/companion/generated/` into your Companion workspace (v3.2+). It provisions the Presenter module with the default timer/stage/Bible pages.
- Install or refresh the native Companion module by copying `ops/companion/presenter/` into `/companion/modules/user/presenter/` on the Companion host, running `npm install --prefix /companion/modules/user/presenter`, and restarting the container. The module handles the websocket handshake, variables feed, and HH:MM countdown conversion automatically.
- Refresh the module tarball before deployments: run `scripts/companion/package-module.sh` to produce `ops/companion/releases/presenter-companion-ws-0.5.0.tgz` and `latest.json`. The companion-docker build in `nl-infrastructure` pulls those artefacts during image creation.
- In Presenter, enable “Companion WebSocket” under **Settings → Companion** and assign a unique port for each environment. Update the module’s host/port accordingly and paste the shared secret if `PRESENTER_COMPANION_TOKEN` is set. Invalid or missing tokens immediately close the socket with code `4001`—Companion should display an alert.
- Buttons ship with feedback bindings for `stage_current_main`, `stage_next_main`, `timer_countdown_state`, and both `timer_countdown_remaining_hhmm` / `timer_countdown_remaining_seconds`. Keep those variables intact when cloning buttons so colour/state logic continues to work.
- Enable Companion’s “Run Macro on Connection State” option and tie the provided `macro_presenter_alert` to the Presenter module instance so operators see a red panic indicator whenever `live_ws_connected` flips to false.
- After modifying Presenter commands or adding new cues, run `npm run test:playwright -- --grep @companion` to exercise the simulated Companion session and verify acks/variable updates still flow end-to-end.

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

## Ableton Live OSC bridge

Load Ableton's Connection Kit “OSC Send” Max for Live device on the automation track, point it at the Presenter controller (`presenter.lan` on port 39051 by default), and store a default preset so the host/port survive reopening the Live Set. Step-by-step setup guidance lives in [docs/ops/ableton-osc-device.md](ableton-osc-device.md).

Pre‑release defaults (intentional)
- OSC address pattern is fixed to `/note`.
- Velocity mode is fixed to zero‑based, i.e. velocity values 0,1,2… map directly to slide indices. This matches our E2E tests and library mappings.
- These defaults simplify the first tuned production build; custom patterns/modes can be added in a later iteration if needed.
