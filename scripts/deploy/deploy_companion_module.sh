#!/usr/bin/env bash
set -euo pipefail
# Deploy the presenter Companion module to ONE production host (#733, #850).
#
# usage: deploy_companion_module.sh <ssh-alias> <label>
#   ssh-alias  the job's alias for the host (deploy.yml / release.yml: deploy-target)
#   label      SNV / PP, for the log lines
#
# WHY it lives in the release workflows: Companion runs only on the production hosts
# (companion-snv = 10.77.9.205, companion-pp.lan). The module used to be deployed by
# pipeline.yml on every push to dev, which stopped and restarted the operators'
# Companion ~50 min after any dev push (2026-10-14: a PP event at 18:00) and ran
# unreleased plugin code next to the released server. Now deploy.yml (SNV) and
# release.yml (PP) run this after the server deploy; check_deploy_workflows.py
# invariant 5 keeps it out of any dev-push workflow.
#
# #733: the module is a Companion DEV module (--extra-module-path
# /opt/companion-module-dev, which both hosts launch with), so the connection pins the
# STABLE version id "dev" instead of a concrete version. A concrete pin breaks with
# "missing version" on every version bump. The re-pin is idempotent. Companion is
# stopped for the module swap + config-DB write and is ALWAYS started again (an EXIT
# trap from the stop on); the step fails loudly on any error.

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <ssh-alias> <label>" >&2
  exit 2
fi
HOST="$1"
LABEL="$2"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODULE_DIR="/opt/companion-module-dev/presenter"
OLD_STORE_DIR="/home/companion/.config/companion-nodejs/modules/presenter"
CONFIG_DIR="/home/companion/.config/companion-nodejs"

STAGE_DIR=$(ssh "$HOST" "mktemp -d /tmp/companion-module-XXXXXX")
echo "Deploying the Companion module to $LABEL (staging: $STAGE_DIR)..."
scp -r "$ROOT"/ops/companion/presenter/* "$HOST:$STAGE_DIR/"
REPIN_REMOTE=$(ssh "$HOST" "mktemp /tmp/repin-presenter-XXXXXX.py")
scp "$ROOT/ops/companion/repin-presenter-connection.py" "$HOST:$REPIN_REMOTE"
ssh "$HOST" "cd $STAGE_DIR && npm install --omit=dev"

# From the stop on, ANY exit (a failed ssh, a swap error) starts Companion again: the
# operators' buttons are never left dead. The module dir may then be a partial state -
# the step still fails loudly.
STOP_ISSUED=0
start_companion_on_exit() {
  if [ "$STOP_ISSUED" -eq 1 ]; then
    echo "::error::Companion deploy on $LABEL stopped early - starting Companion again"
    ssh "$HOST" "sudo systemctl start companion" || true
  fi
}
trap start_companion_on_exit EXIT
STOP_ISSUED=1
ssh "$HOST" "sudo systemctl stop companion"
ssh "$HOST" "sudo rm -rf $MODULE_DIR && sudo mkdir -p $(dirname "$MODULE_DIR") && sudo mv $STAGE_DIR $MODULE_DIR && sudo chown -R companion:companion $MODULE_DIR" \
  || { echo "::error::Companion module swap failed on $LABEL"; exit 1; }
ssh "$HOST" "sudo rm -rf $OLD_STORE_DIR" || true
REPIN_RC=0
ssh "$HOST" "sudo python3 $REPIN_REMOTE --config-dir $CONFIG_DIR --build-file /opt/companion/BUILD --expect-connection" || REPIN_RC=$?
ssh "$HOST" "rm -f $REPIN_REMOTE" || true
ssh "$HOST" "sudo systemctl start companion"
STOP_ISSUED=0
sleep 5
ssh "$HOST" "systemctl is-active companion" || { echo "::error::Companion failed to restart on $LABEL"; exit 1; }
if [ "$REPIN_RC" -ne 0 ]; then
  echo "::error::presenter connection re-pin failed on $LABEL (rc=$REPIN_RC)"
  exit 1
fi
VERSION=$(node -e "console.log(require('$ROOT/ops/companion/presenter/package.json').version)")
echo "Companion module $VERSION deployed to $LABEL (dev module + connection pinned to 'dev')"
