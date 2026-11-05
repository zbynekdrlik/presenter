#!/usr/bin/env bash
# Installs apt packages on a GitHub-hosted runner (#850, 2026-11-05).
#
# A mirror sometimes stalls: apt normally takes about a minute, but three runs
# on 2026-11-05 hung until the step's 10-minute limit. Each attempt is bounded,
# and a stalled attempt is retried once. Without -qq the log shows which
# mirror stalled.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: apt-install.sh <package>..." >&2
  exit 2
fi

APT=(sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30)

attempt() {
  timeout 150 "${APT[@]}" update || return 1
  timeout 240 "${APT[@]}" install -y -q "$@" || return 1
}

for n in 1 2; do
  started=$(date +%s)
  if attempt "$@"; then
    echo "apt-install: attempt $n done in $(( $(date +%s) - started )) s"
    exit 0
  fi
  echo "::warning::apt-install: attempt $n failed after $(( $(date +%s) - started )) s"
done
echo "::error::apt-install: both attempts failed" >&2
exit 1
