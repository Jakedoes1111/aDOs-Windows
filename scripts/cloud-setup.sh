#!/usr/bin/env bash
# One-time setup for Ubuntu 24.04 (a fresh VM, container or cloud session): build packages, the UI test rig, Servo.
# Note: it appends two environment lines to ~/.bashrc.
# Proven by the cloud trial on 13 Nov 2026: cold `cargo build -p ados` 5m40s on 4 cores / 15 GB.
# Usage: scripts/cloud-setup.sh   (idempotent; safe to re-run)
set -euo pipefail
cd "$(dirname "$0")/.."
export DEBIAN_FRONTEND=noninteractive
SUDO=$([ "$(id -u)" = 0 ] && echo "" || echo sudo)
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq --no-install-recommends \
  build-essential clang libclang-dev pkg-config cmake \
  libfontconfig1-dev libfreetype6-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libgbm-dev \
  libasound2-dev libssl-dev libdbus-1-dev \
  xvfb xfwm4 xdotool scrot x11-utils mesa-vulkan-drivers libvulkan1 \
  fonts-dejavu-core fonts-jetbrains-mono \
  libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 libgstreamer-plugins-bad1.0-0 \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav gstreamer1.0-gl \
  python3 >/dev/null
# Servo nightly from GitHub releases.
S="$HOME/Apps/servo/servo/servoshell"
if [ ! -x "$S" ]; then
  mkdir -p "$HOME/Apps/servo"
  curl -sSL -m 300 https://github.com/servo/servo-nightly-builds/releases/download/2026-11-17/servo-x86_64-linux-gnu.tar.gz \
    | tar xz -C "$HOME/Apps/servo"
fi
"$S" --version
# Use every core, and the distro library folder.
grep -q CARGO_BUILD_JOBS ~/.bashrc 2>/dev/null || echo 'export CARGO_BUILD_JOBS=$(nproc) LIBRARY_PATH=/usr/lib/x86_64-linux-gnu' >> ~/.bashrc
echo "cloud setup done"
