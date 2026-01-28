#!/usr/bin/env bash
# Presenter - One-line installer
# Usage: curl -fsSL https://raw.githubusercontent.com/zbynekdrlik/presenter/main/scripts/install.sh | bash
#
# Options (via environment variables):
#   PRESENTER_VERSION  - Specific version to install (default: latest)
#   PRESENTER_DIR      - Installation directory (default: /usr/local/bin)
#   PRESENTER_DATA_DIR - Data directory (default: ~/.presenter)

set -euo pipefail

REPO="zbynekdrlik/presenter"
BINARY_NAME="presenter-server"
DEFAULT_INSTALL_DIR="/usr/local/bin"
DEFAULT_DATA_DIR="$HOME/.presenter"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1" >&2; exit 1; }

# Detect OS and architecture
detect_platform() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"

    case "$os" in
        linux) os="linux" ;;
        darwin) os="darwin" ;;
        *) error "Unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64) arch="x64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) error "Unsupported architecture: $arch" ;;
    esac

    echo "${os}-${arch}"
}

# Get latest version from GitHub API
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | \
        grep '"tag_name":' | sed -E 's/.*"v?([^"]+)".*/\1/'
}

# Download and install
install_presenter() {
    local version="${PRESENTER_VERSION:-$(get_latest_version)}"
    local install_dir="${PRESENTER_DIR:-$DEFAULT_INSTALL_DIR}"
    local data_dir="${PRESENTER_DATA_DIR:-$DEFAULT_DATA_DIR}"
    local platform
    platform="$(detect_platform)"

    info "Installing Presenter v${version} for ${platform}"

    # Create temp directory
    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap "rm -rf '$tmp_dir'" EXIT

    # Download release
    local download_url="https://github.com/${REPO}/releases/download/v${version}/presenter-${version}-${platform}.tar.gz"
    info "Downloading from: ${download_url}"

    if ! curl -fsSL "$download_url" -o "${tmp_dir}/presenter.tar.gz"; then
        error "Failed to download release. Check if version ${version} exists."
    fi

    # Extract
    info "Extracting..."
    tar -xzf "${tmp_dir}/presenter.tar.gz" -C "$tmp_dir"

    # Install binary
    info "Installing to ${install_dir}..."
    if [ -w "$install_dir" ]; then
        cp "${tmp_dir}/${BINARY_NAME}" "${install_dir}/"
        chmod +x "${install_dir}/${BINARY_NAME}"
    else
        warn "Need sudo to install to ${install_dir}"
        sudo cp "${tmp_dir}/${BINARY_NAME}" "${install_dir}/"
        sudo chmod +x "${install_dir}/${BINARY_NAME}"
    fi

    # Create data directory
    mkdir -p "$data_dir"

    info "Installation complete!"
    echo ""
    echo "  Binary installed: ${install_dir}/${BINARY_NAME}"
    echo "  Data directory:   ${data_dir}"
    echo ""
    echo "  To start Presenter:"
    echo "    ${BINARY_NAME}"
    echo ""
    echo "  Or with custom settings:"
    echo "    PRESENTER_PORT=8080 PRESENTER_DB_URL=sqlite://${data_dir}/presenter.db ${BINARY_NAME}"
    echo ""
}

# Run installer
install_presenter
