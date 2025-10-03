#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REPO_PARENT="$(cd "${REPO_ROOT}/.." && pwd)"
SHARED_LIB_ROOT="${PRESENTER_LIBRARY_ROOT:-$REPO_PARENT/presenter-libraries}"

# ensure rustup toolchain is available when running under systemd where PATH may exclude ~/.cargo/bin
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi
ENVIRONMENT="${1:-}" # dev | test | prod
if [[ -z "$ENVIRONMENT" ]]; then
  echo "Usage: $(basename "$0") <dev|test|prod>" >&2
  exit 64
fi

if [[ "$ENVIRONMENT" != "dev" && "$ENVIRONMENT" != "test" && "$ENVIRONMENT" != "prod" ]]; then
  echo "Unknown environment '$ENVIRONMENT'. Expected dev, test, or prod." >&2
  exit 65
fi
shift

export PRESENTER_ENVIRONMENT="$ENVIRONMENT"
export PRESENTER_DATA_ROOT="${PRESENTER_DATA_ROOT:-$REPO_ROOT/var/data}"
mkdir -p "$PRESENTER_DATA_ROOT/$ENVIRONMENT"

case "$ENVIRONMENT" in
  dev)
    export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$PRESENTER_DATA_ROOT/dev/presenter_dev.db}"
    export PRESENTER_PORT="${PRESENTER_PORT:-80}"
    exec "$REPO_ROOT/scripts/dev/watch-demo.sh" "$@"
    ;;
  test)
    export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$PRESENTER_DATA_ROOT/test/presenter_test.db}"
    export PRESENTER_PORT="${PRESENTER_PORT:-8081}"
    if [[ "${PRESENTER_RESET_DB:-1}" == "1" ]]; then
      "$REPO_ROOT/scripts/dev/refresh-dev-data.sh" "$SHARED_LIB_ROOT"
    fi
    exec cargo run -p presenter-server "$@"
    ;;
  prod)
    export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$PRESENTER_DATA_ROOT/prod/presenter_prod.db}"
    export PRESENTER_PORT="${PRESENTER_PORT:-8080}"
    export RUST_LOG="${RUST_LOG:-info}"
    exec cargo run --release -p presenter-server "$@"
    ;;
  *)
    echo "Unhandled environment '$ENVIRONMENT'" >&2
    exit 66
    ;;
 esac
