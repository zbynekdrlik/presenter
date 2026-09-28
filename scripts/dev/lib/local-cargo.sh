# shellcheck shell=bash
# Tier-0 local-compile guard (#802) — SOURCE this file, never execute it.
#
# Presenter compiles Rust in CI only. A repo script may compile locally ONLY
# when the caller explicitly opted in with PRESENTER_ALLOW_LOCAL_CARGO=1
# (Tier-1/2 boxes). Otherwise it must use a prebuilt binary (the CI
# `build-artifacts` artifact) or stop with an actionable message. The guard
# `scripts/ci/test_no_local_cargo_fallback.py` fails CI when a compiling cargo
# call in scripts/dev or scripts/ops is not behind require_local_cargo /
# local_cargo_allowed.

# True when the caller explicitly opted in to local compilation.
local_cargo_allowed() {
  [[ "${PRESENTER_ALLOW_LOCAL_CARGO:-}" == "1" ]]
}

# local_cargo_refuse <tag> <reason> [extra hint line...] — print the recipe to
# stderr and exit 1 (exits the whole script, or the enclosing subshell).
local_cargo_refuse() {
  local tag="$1"
  local reason="$2"
  shift 2
  local line
  {
    echo "[${tag}] ERROR: ${reason}"
    echo "[${tag}] Presenter is Tier-0: Rust compiles in CI only, scripts never compile locally without an explicit opt-in (#802)."
    for line in "$@"; do
      echo "[${tag}] ${line}"
    done
    echo "[${tag}] Prebuilt binaries come from CI — download the build artifact for your commit into target/release/:"
    echo "[${tag}]   gh run download <run-id> -n build-artifacts -D _artifacts"
    echo "[${tag}]   mkdir -p target/release && cp _artifacts/* target/release/ && chmod +x target/release/*"
    echo "[${tag}] (find <run-id> with: gh run list -w pipeline.yml -b dev -L 5)"
    echo "[${tag}] Explicit local-compile opt-in (Tier-1/2 boxes only): PRESENTER_ALLOW_LOCAL_CARGO=1"
  } >&2
  exit 1
}

# require_local_cargo <tag> <what> [extra hint line...] — return 0 when opted
# in (logging that a local compile follows), otherwise refuse and exit 1.
require_local_cargo() {
  local tag="$1"
  local what="$2"
  shift 2
  if local_cargo_allowed; then
    echo "[${tag}] PRESENTER_ALLOW_LOCAL_CARGO=1 — compiling locally: ${what} (Tier-0 opt-in)" >&2
    return 0
  fi
  local_cargo_refuse "$tag" "refusing to compile locally: ${what}" "$@"
}
