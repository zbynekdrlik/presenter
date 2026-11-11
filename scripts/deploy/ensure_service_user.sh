#!/usr/bin/env bash
set -euo pipefail
# Ensure the system user a Presenter site runs as (#929). Runs ON the host, as the deploy
# user (passwordless sudo), from deploy-site.yml's "Ensure the service user" step:
#
#   ssh deploy-target "bash -s -- <service_user> <deploy_dir> <hardware>" < ensure_service_user.sh
#
# Why its own user: dcontroller's dante-guard lets a Dante app onto the Dante leg by the
# socket's uid (nft `meta skuid`, dstack two-networks.md §3). Running presenter as the admin
# login would let everything that login runs onto the leg.
#
# Idempotent:
#   * creates a system user + group with home = the deploy dir (systemd sets HOME from it, and
#     it is the unit's only ReadWritePaths); an existing account is never modified;
#   * on a hardware site, seeds <deploy_dir>/.android/adbkey from the deploy user's own key when
#     the service user has none: the stage TVs authorized THAT key, and an adb server the
#     service starts after a reboot would otherwise generate a new one — every TV would then
#     ask "Allow USB debugging?" on screen and the stage launcher would stop working.
# Ownership of the deploy dir itself is handed over in the swap step (service stopped).

SERVICE_USER="${1:?service user}"
DEPLOY_DIR="${2:?deploy dir}"
HARDWARE="${3:?hardware true|false}"
# Overridable only by the shell guard (tests/deploy/service-user.test.sh), never in a deploy.
SUDO="${SUDO_CMD:-sudo}"
DEPLOY_HOME="${DEPLOY_USER_HOME:-$HOME}"

case "$SERVICE_USER" in *[!a-z0-9-]* | '') echo "::error::bad service user '$SERVICE_USER'" >&2; exit 1 ;; esac
case "$DEPLOY_DIR" in /*) ;; *) echo "::error::deploy dir must be absolute: '$DEPLOY_DIR'" >&2; exit 1 ;; esac

if entry=$(getent passwd "$SERVICE_USER"); then
  # An existing account is never modified: on PRBA it is also the deploy's SSH login, whose
  # authorized_keys live in its /home — moving that home would lock the deploy out.
  home=$(printf '%s' "$entry" | cut -d: -f6)
  echo "Service user $SERVICE_USER exists (uid $(printf '%s' "$entry" | cut -d: -f3), home $home)"
else
  $SUDO useradd --system --user-group --home-dir "$DEPLOY_DIR" --no-create-home \
    --shell /usr/sbin/nologin "$SERVICE_USER"
  echo "Service user $SERVICE_USER created"
  home="$DEPLOY_DIR"
fi

if [ "$HARDWARE" != "true" ]; then
  echo "No devices: no adb key to seed"
  exit 0
fi

# The service's adb (and so its key) lives in $HOME/.android; the unit lets it write only the
# deploy dir, so a device site's service user must be homed there.
if [ "$home" != "$DEPLOY_DIR" ]; then
  echo "::error::service user $SERVICE_USER has home $home, a device site needs $DEPLOY_DIR (adb key + ReadWritePaths)" >&2
  exit 1
fi
KEY_DIR="$DEPLOY_DIR/.android"
if $SUDO test -s "$KEY_DIR/adbkey"; then
  echo "adb key of $SERVICE_USER already present"
elif [ -s "$DEPLOY_HOME/.android/adbkey" ]; then
  $SUDO install -d -m 0700 -o "$SERVICE_USER" -g "$SERVICE_USER" "$KEY_DIR"
  $SUDO install -m 0600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DEPLOY_HOME/.android/adbkey" "$KEY_DIR/adbkey"
  if [ -s "$DEPLOY_HOME/.android/adbkey.pub" ]; then
    $SUDO install -m 0644 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DEPLOY_HOME/.android/adbkey.pub" "$KEY_DIR/adbkey.pub"
  fi
  echo "adb key seeded for $SERVICE_USER from the deploy user's key (the TVs keep trusting it)"
else
  # A site whose TVs never authorized any key gets a fresh one on first use — loud, not fatal.
  echo "::warning::no adb key at $DEPLOY_HOME/.android/adbkey to seed — the stage TVs will ask to authorize a new key"
fi
