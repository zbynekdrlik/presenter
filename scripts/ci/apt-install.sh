#!/usr/bin/env bash
# Installs apt packages on a GitHub-hosted runner (#850, 2026-11-05).
#
# A mirror sometimes stalls: apt normally takes about a minute, but three runs
# on 2026-11-05 hung until the step's 10-minute limit. Each attempt is bounded,
# and a stalled attempt is retried once. Without -qq the log shows which
# mirror stalled. Two download attempts plus one install (2 x (90 + 170) + 300 s)
# fit in the calling steps' 15-minute limit (2026-11-23: was 10 minutes with the
# install inside each bounded attempt, see download() below).
#
# A slow mirror is not cured by retrying it: on 2026-11-05 the runners' Azure
# mirror sent ~40 kB/s on both attempts. The retry therefore drops the Azure
# mirror from the runner's mirror list (/etc/apt/apt-mirrors.txt, read by the
# `mirror+file:` sources) and fetches from a list of independent mirrors.
#
# Recommends are not installed (2026-11-23): the gstreamer/-dev set pulled
# 316 MB with recommends (gdal, ROCm libamdhip64, …) and two Clippy runs that
# day ran out of both attempts on a slow mirror. Every package a job needs is
# named explicitly in its call, so a missing recommend is a missing name there.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: apt-install.sh <package>..." >&2
  exit 2
fi

MIRRORS=/etc/apt/apt-mirrors.txt

use_main_archive() {
  if [ -f "$MIRRORS" ]; then
    echo "apt-install: mirror list was:"
    cat "$MIRRORS"
    # Several mirrors, tried in priority order by apt's `mirror+file:` method:
    # on 2026-11-23 archive.ubuntu.com answered InRelease in 15 s and the EC2
    # runner mirror with 503, while kernel.org / Azure / MIT answered < 1 s.
    printf '%s\tpriority:%s\n' \
      http://mirrors.edge.kernel.org/ubuntu/ 1 \
      http://azure.archive.ubuntu.com/ubuntu/ 2 \
      http://mirrors.mit.edu/ubuntu/ 3 \
      http://archive.ubuntu.com/ubuntu/ 4 \
      | sudo tee "$MIRRORS" >/dev/null
    echo "apt-install: retrying from kernel.org / Azure / MIT / archive.ubuntu.com"
  else
    echo "apt-install: no $MIRRORS, retrying the configured sources"
  fi
}

APT=(sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30)

# The composite action .github/actions/apt-install restores the job's earlier
# .deb downloads into APT_ARCHIVES (actions/cache) and saves them after the job;
# apt then fetches only what changed. Unset (a direct call): apt's own cache.
if [ -n "${APT_ARCHIVES:-}" ]; then
  mkdir -p "$APT_ARCHIVES/partial"
  APT+=(-o "Dir::Cache::Archives=$APT_ARCHIVES/")
fi

count_debs() {
  find "$APT_ARCHIVES" -maxdepth 1 -name '*.deb' | wc -l
}

# apt runs as root; the cache is saved by the runner user, who must read it.
# Also after a failed install: the .debs it did fetch are saved for the next
# run (the action saves whenever this run added any, APT_NEW_DEBS_OUTPUT).
# shellcheck disable=SC2317 # invoked through the EXIT trap
own_archives() {
  if [ -n "${APT_ARCHIVES:-}" ]; then
    sudo rm -rf "$APT_ARCHIVES/partial" "$APT_ARCHIVES/lock"
    sudo chown -R "$(id -u):$(id -g)" "$APT_ARCHIVES"
    if [ -n "${APT_NEW_DEBS_OUTPUT:-}" ]; then
      echo "new_debs=$(( $(count_debs) - DEBS_BEFORE ))" >> "$APT_NEW_DEBS_OUTPUT"
    fi
  fi
}
if [ -n "${APT_ARCHIVES:-}" ]; then
  DEBS_BEFORE=$(count_debs)
  trap own_archives EXIT
fi

# Only the download talks to a mirror, so only the download is bounded and
# retried. On 2026-11-23 (run 45190029022) the 170 s bound killed apt-get
# while dpkg was unpacking; the orphaned dpkg kept its lock and the retry
# failed at once ("dpkg was interrupted"). The install runs from the
# downloaded .debs, once, with its own bound.
download() {
  timeout 90 "${APT[@]}" update || return 1
  timeout 170 "${APT[@]}" install -y -q --no-install-recommends --download-only "$@" || return 1
}

fetched=0
for n in 1 2; do
  started=$(date +%s)
  if [ "$n" -gt 1 ]; then
    use_main_archive
  fi
  if download "$@"; then
    echo "apt-install: download attempt $n done in $(( $(date +%s) - started )) s"
    fetched=1
    break
  fi
  echo "::warning::apt-install: download attempt $n failed after $(( $(date +%s) - started )) s"
done
if [ "$fetched" -ne 1 ]; then
  echo "::error::apt-install: both download attempts failed" >&2
  exit 1
fi

started=$(date +%s)
if ! timeout 300 "${APT[@]}" -o DPkg::Lock::Timeout=60 install -y -q --no-install-recommends "$@"; then
  echo "::error::apt-install: install from the downloaded packages failed after $(( $(date +%s) - started )) s" >&2
  exit 1
fi
echo "apt-install: install done in $(( $(date +%s) - started )) s"
