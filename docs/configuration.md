# Configuration Reference

All environment variables and feature flags for Presenter.

## Environment Variables

### Server

| Variable | Default | Description |
|----------|---------|-------------|
| `PRESENTER_PORT` | `80` | HTTP server port |
| `PRESENTER_DB_URL` | `sqlite://presenter.db` | SQLite connection string |
| `RUST_LOG` | `info,tower_http=debug` | Tracing filter |

### Companion Integration

| Variable | Default | Description |
|----------|---------|-------------|
| `PRESENTER_COMPANION_ENABLED` | `0` | Enable Companion WebSocket (0/1) |
| `PRESENTER_COMPANION_PORT` | `18175` | Companion listener port |
| `PRESENTER_COMPANION_TOKEN` | (none) | Shared secret for authentication |

### Stage Display

| Variable | Default | Description |
|----------|---------|-------------|
| `PRESENTER_HEARTBEAT_INTERVAL_MS` | `1500` | Stage heartbeat frequency (ms) |

### Android Stage Launchers

| Variable | Default | Description |
|----------|---------|-------------|
| `PRESENTER_ANDROID_ADB_BIN` | `adb` (PATH) | Custom adb binary path |

### Data & Backup

| Variable | Default | Description |
|----------|---------|-------------|
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

| Environment | Port | Database | Notes |
|-------------|------|----------|-------|
| development | 80 | `var/data/dev/presenter.db` | Auto-refresh, debug logs |
| testing | 8081 | `var/data/test/presenter.db` | Mirrors production config |
| production | 8080 | `var/data/prod/presenter.db` | Release build only |

## CI/CD Variables

GitHub Actions repository variables:

| Variable | Purpose |
|----------|---------|
| `RUNNER_LABEL` | Custom runner label (e.g., `self-hosted`) |
| `CODECOV_TOKEN` | Codecov upload token (secret) |

## Related

- [Runbook](ops/runbook.md) - Operational procedures
- [Settings README](settings/README.md) - Feature flag details
