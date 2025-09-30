#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DATA_ROOT="${PRESENTER_DATA_ROOT:-$REPO_ROOT/var/data}"
BACKUP_ROOT="${PRESENTER_BACKUP_ROOT:-$REPO_ROOT/var/backups}"

usage() {
  cat <<USAGE
Usage: $(basename "$0") <command> [args]
Commands:
  backup <env>              Create a timestamped SQLite backup for the given environment (dev|test|prod).
  list <env>                List available backups for the environment.
  restore <env> <backup>    Restore the provided backup file into the environment database (service must be stopped).

Environment variables:
  PRESENTER_DATA_ROOT       Override the data directory (default: $REPO_ROOT/var/data)
  PRESENTER_BACKUP_ROOT     Override the backup directory (default: $REPO_ROOT/var/backups)
USAGE
}

command="${1:-}"; shift || true
if [[ -z "$command" ]]; then
  usage >&2
  exit 64
fi

case "$command" in
  backup|list|restore)
    ;;
  -h|--help|help)
    usage
    exit 0
    ;;
  *)
    echo "Unknown command '$command'" >&2
    usage >&2
    exit 65
    ;;
esac

env="${1:-}"; shift || true
if [[ -z "$env" ]]; then
  echo "Missing environment (dev|test|prod)." >&2
  usage >&2
  exit 66
fi

case "$env" in
  dev)
    db_file="presenter_dev.db"
    ;;
  test)
    db_file="presenter_test.db"
    ;;
  prod)
    db_file="presenter_prod.db"
    ;;
  *)
    echo "Unknown environment '$env'." >&2
    exit 67
    ;;
esac

env_data_dir="$DATA_ROOT/$env"
mkdir -p "$env_data_dir"

db_path="$env_data_dir/$db_file"
backup_dir="$BACKUP_ROOT/$env"
mkdir -p "$backup_dir"

case "$command" in
  backup)
    timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
    backup_path="$backup_dir/${db_file%.db}-$timestamp.db"
    if [[ ! -f "$db_path" ]]; then
      echo "Database $db_path does not exist; nothing to backup." >&2
      exit 68
    fi
    sqlite3 "$db_path" ".backup '$backup_path'"
    echo "Created backup: $backup_path"
    ;;
  list)
    if ls "$backup_dir"/*.db >/dev/null 2>&1; then
      ls -1t "$backup_dir"/*.db
    else
      echo "No backups found in $backup_dir" >&2
    fi
    ;;
  restore)
    backup_path="${1:-}"
    if [[ -z "$backup_path" ]]; then
      echo "Provide a backup file to restore." >&2
      exit 69
    fi
    if [[ ! -f "$backup_path" ]]; then
      echo "Backup file '$backup_path' not found." >&2
      exit 70
    fi
    if command -v systemctl >/dev/null 2>&1; then
      if systemctl is-active --quiet "presenter@$env.service"; then
        echo "presenter@$env.service is currently running; stop it before restoring." >&2
        exit 71
      fi
    fi
    echo "Restoring $backup_path -> $db_path"
    sqlite3 "$db_path" "PRAGMA journal_mode=OFF;" >/dev/null 2>&1 || true
    install -D -m 0640 "$backup_path" "$db_path"
    echo "Restore complete. Restart the presenter@$env.service if it is running."
    ;;
esac
