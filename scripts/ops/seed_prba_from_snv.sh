#!/usr/bin/env bash
# One-time seed of the PRBA instance on dev1 from a copy of SNV (#918).
#
# Run ONCE, by hand, from dev2 (it reaches SNV over the LAN and dev1 over tailscale).
# Owner 2026-11-08: SNV data is copied only at the start; afterwards PRBA edits its own,
# so this refuses to run when PRBA already has a database — it never re-seeds.
#
# SNV is production: its DB is only read (`sqlite3 -readonly … .backup`), the temporary
# backup file on SNV is removed again.
set -euo pipefail

SNV="${SNV:-newlevel@presenter.lan}"
PRBA="${PRBA:-newlevel@100.104.8.125}"
PRBA_DIR="${PRBA_DIR:-/opt/presenter-prba}"
PRBA_USER="${PRBA_USER:-presenter-prba}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ssh "$PRBA" "test -e $PRBA_DIR/presenter.db"; then
  echo "PRBA DB exists ($PRBA_DIR/presenter.db) — refusing to re-seed" >&2
  exit 1
fi
ssh "$PRBA" "test -d $PRBA_DIR" || { echo "$PRBA_DIR missing on PRBA host — create it first" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
REMOTE_TMP="/tmp/prba-seed-$$.db"

echo "== backup SNV DB (read-only)"
ssh "$SNV" "sqlite3 -readonly /opt/presenter/presenter.db '.backup $REMOTE_TMP'"
scp -q "$SNV:$REMOTE_TMP" "$WORK/presenter.db"
ssh "$SNV" "rm -f $REMOTE_TMP"

echo "== clear SNV devices and site settings"
python3 -I "$HERE/prba_seed_clean.py" "$WORK/presenter.db"

echo "== install the DB on PRBA"
scp -q "$WORK/presenter.db" "$PRBA:$REMOTE_TMP"
ssh "$PRBA" "sudo install -o $PRBA_USER -g $PRBA_USER -m 640 $REMOTE_TMP $PRBA_DIR/presenter.db && rm -f $REMOTE_TMP"

echo "== copy the SNV libraries folder"
mkdir -p "$WORK/libraries"
rsync -a "$SNV:/opt/presenter/libraries/" "$WORK/libraries/"
ssh "$PRBA" "mkdir -p /tmp/prba-libraries-$$"
rsync -a "$WORK/libraries/" "$PRBA:/tmp/prba-libraries-$$/"
ssh "$PRBA" "sudo rm -rf $PRBA_DIR/libraries && sudo mv /tmp/prba-libraries-$$ $PRBA_DIR/libraries && sudo chown -R $PRBA_USER:$PRBA_USER $PRBA_DIR/libraries"

echo "== done: $(ssh "$PRBA" "sudo ls -l $PRBA_DIR/presenter.db")"
