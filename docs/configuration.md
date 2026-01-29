# Configuration Reference

All environment variables and feature flags for Presenter.

## Environment Variables

### Server

| Variable           | Default                 | Description              |
| ------------------ | ----------------------- | ------------------------ |
| `PRESENTER_PORT`   | `80`                    | HTTP server port         |
| `PRESENTER_DB_URL` | `sqlite://presenter.db` | SQLite connection string |
| `RUST_LOG`         | `info,tower_http=debug` | Tracing filter           |

### Companion Integration

| Variable                      | Default | Description                      |
| ----------------------------- | ------- | -------------------------------- |
| `PRESENTER_COMPANION_ENABLED` | `0`     | Enable Companion WebSocket (0/1) |
| `PRESENTER_COMPANION_PORT`    | `18175` | Companion listener port          |
| `PRESENTER_COMPANION_TOKEN`   | (none)  | Shared secret for authentication |

### Stage Display

| Variable                          | Default | Description                    |
| --------------------------------- | ------- | ------------------------------ |
| `PRESENTER_HEARTBEAT_INTERVAL_MS` | `1500`  | Stage heartbeat frequency (ms) |

### Android Stage Launchers

| Variable                    | Default      | Description            |
| --------------------------- | ------------ | ---------------------- |
| `PRESENTER_ANDROID_ADB_BIN` | `adb` (PATH) | Custom adb binary path |

### Data & Backup

| Variable                 | Default                  | Description                |
| ------------------------ | ------------------------ | -------------------------- |
| `PRESENTER_LIBRARY_ROOT` | `../presenter-libraries` | ProPresenter import source |

## Feature Flags

Managed via Settings UI (`/ui/settings`):

### Companion WebSocket

- **Enable/Disable**: Toggle WebSocket server
- **Port**: Configure listener port
- **Token**: Set authentication secret

### Android Stage Launchers

- **Device Roster**: Configure ADB endpoints
- **Health Reporting**: Per-device status

## Runtime Profiles

| Environment | Port    | Database                          | Service                 | Notes                     |
| ----------- | ------- | --------------------------------- | ----------------------- | ------------------------- |
| Production  | 80      | `/opt/presenter/presenter.db`     | `presenter.service`     | Release build from `main` |
| Dev deploy  | 8080    | `/opt/presenter-dev/presenter.db` | `presenter-dev.service` | Release build from `dev`  |
| Local dev   | 80      | `presenter_dev.db` (cwd)          | N/A                     | `cargo run`               |
| E2E testing | dynamic | temp DB                           | N/A                     | Playwright tests          |

## CI/CD Variables

GitHub Actions repository variables:

| Variable        | Purpose                                   |
| --------------- | ----------------------------------------- |
| `RUNNER_LABEL`  | Custom runner label (e.g., `self-hosted`) |
| `CODECOV_TOKEN` | Codecov upload token (secret)             |

## Related

- [Runbook](ops/runbook.md) - Operational procedures
- [Settings README](settings/README.md) - Feature flag details
