#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -eq 0 ]]; then
  echo "[bootstrap] Run this script as the normal project user (it will sudo as needed)." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

APT_PACKAGES=(
  build-essential
  pkg-config
  libssl-dev
  protobuf-compiler
  # Playwright/browser runtime dependencies
  libasound2t64
  libatk-bridge2.0-0t64
  libatk1.0-0t64
  libatspi2.0-0t64
  libcairo2
  libcups2t64
  libdbus-1-3
  libdrm2
  libgbm1
  libglib2.0-0t64
  libgtk-3-0t64
  libnspr4
  libnss3
  libpango-1.0-0
  libx11-6
  libxcb1
  libxcomposite1
  libxdamage1
  libxext6
  libxfixes3
  libxkbcommon0
  libxrandr2
  libx11-xcb1
  libxcursor1
  libxi6
  libxrender1
  libxinerama1
  libxshmfence1
  libxss1
  libxtst6
  libxcb-shm0
  libpangocairo-1.0-0
  libgdk-pixbuf-2.0-0
  libatk1.0-data
  libfontconfig1
  libfreetype6
  gstreamer1.0-libav
  gstreamer1.0-plugins-bad
  gstreamer1.0-plugins-base
  gstreamer1.0-plugins-good
  libicu74
  libatomic1
  libenchant-2-2
  libepoxy0
  libevent-2.1-7t64
  libflite1
  libgles2
  libgstreamer-gl1.0-0
  libgstreamer-plugins-bad1.0-0
  libgstreamer-plugins-base1.0-0
  libgstreamer1.0-0
  libgtk-4-1
  libharfbuzz-icu0
  libharfbuzz0b
  libhyphen0
  libjpeg-turbo8
  liblcms2-2
  libmanette-0.2-0
  libopus0
  libpng16-16t64
  libsecret-1-0
  libvpx9
  libwayland-client0
  libwayland-egl1
  libwayland-server0
  libwebp7
  libwebpdemux2
  libwoff1
  libxml2
  libxslt1.1
  libx264-164
  libavif16
  xvfb
  xdg-utils
  fonts-noto-color-emoji
  fonts-unifont
  fonts-liberation
  fonts-freefont-ttf
  fonts-wqy-zenhei
  fonts-ipafont-gothic
  fonts-tlwg-loma-otf
)

printf '[bootstrap] Updating apt metadata...\n'
sudo apt-get update

printf '[bootstrap] Installing base packages...\n'
sudo apt-get install -y "${APT_PACKAGES[@]}"

# Rust toolchain
if ! command -v rustup >/dev/null 2>&1; then
  printf '[bootstrap] Installing Rust toolchain via rustup...\n'
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
rustup install stable >/dev/null 2>&1 || true
rustup default stable >/dev/null 2>&1 || true

# Node & npm are expected from NodeSource, but fail fast if missing.
if ! command -v node >/dev/null 2>&1; then
  echo "[bootstrap] Node.js is not installed. Install Node 22.x (NodeSource apt) before rerunning." >&2
  exit 1
fi

printf '[bootstrap] Installing Playwright browsers and system deps...\n'
NODE_OPTIONS="${NODE_OPTIONS:-}" npx --yes playwright install --with-deps

printf '[bootstrap] Host provisioning complete.\n'
