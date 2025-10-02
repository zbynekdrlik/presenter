#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SOURCE_DIR="${REPO_ROOT}/prompts"
TARGET_DIR="${HOME}/.codex/prompts"

if [[ ! -d "${SOURCE_DIR}" ]]; then
  echo "[prompts] source directory not found: ${SOURCE_DIR}" >&2
  exit 1
fi

mkdir -p "${TARGET_DIR}"

shopt -s nullglob
count=0
for file in "${SOURCE_DIR}"/*.md; do
  dest="${TARGET_DIR}/$(basename "${file}")"
  install -m 0644 "${file}" "${dest}"
  count=$((count + 1))
done
shopt -u nullglob

echo "[prompts] Published ${count} prompt file(s) to ${TARGET_DIR}"
