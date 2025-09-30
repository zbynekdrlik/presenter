#!/usr/bin/env bash
set -euo pipefail

if [[ $(id -u) -ne 0 ]]; then
  exec sudo --preserve-env=PATH "$0" "$@"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
UNIT_SOURCE="$REPO_ROOT/ops/systemd/presenter@.service"
UNIT_TARGET="/etc/systemd/system/presenter@.service"
BACKUP_UNIT_SOURCE="$REPO_ROOT/ops/systemd/presenter-backup@.service"
BACKUP_UNIT_TARGET="/etc/systemd/system/presenter-backup@.service"
BACKUP_TIMER_SOURCE="$REPO_ROOT/ops/systemd/presenter-backup@.timer"
BACKUP_TIMER_TARGET="/etc/systemd/system/presenter-backup@.timer"

if [[ ! -f "$UNIT_SOURCE" ]]; then
  echo "Missing systemd unit template at $UNIT_SOURCE" >&2
  exit 70
fi

install -D -m 0644 "$UNIT_SOURCE" "$UNIT_TARGET"
install -D -m 0644 "$BACKUP_UNIT_SOURCE" "$BACKUP_UNIT_TARGET"
install -D -m 0644 "$BACKUP_TIMER_SOURCE" "$BACKUP_TIMER_TARGET"

systemctl daemon-reload

environments=("${@:-dev}")
for env in "${environments[@]}"; do
  case "$env" in
    dev|test|prod)
      systemctl enable --now "presenter@$env.service"
      if [[ "$env" == "prod" ]]; then
        systemctl enable --now "presenter-backup@$env.timer"
      fi
      ;;
    *)
      echo "Skipping unknown environment '$env'" >&2
      ;;
  esac
done

systemctl status "presenter@${environments[0]}.service" --no-pager || true
