#!/usr/bin/env bash
# One-shot install script for Ubuntu 22.04 / 24.04 (x86_64).
# Run with: bash install-ubuntu.sh
# Builds from source and installs the netshare binary + udev rule.
set -euo pipefail

# ── Dependency check ──────────────────────────────────────────────────────────
echo "[1/5] Installing build dependencies..."
sudo apt-get update -qq
sudo apt-get install -y --no-install-recommends \
    build-essential \
    curl \
    git \
    pkg-config \
    libasound2-dev \
    libssl-dev \
    libx11-dev \
    libxcb1-dev \
    libxrandr-dev \
    libxi-dev \
    libxtst-dev \
    libxdo-dev \
    libglib2.0-dev \
    libgtk-3-dev

# ── Install Rust if needed ────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
    echo "[2/5] Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck disable=SC1090
    source "$HOME/.cargo/env"
else
    echo "[2/5] Rust already installed ($(cargo --version))."
fi

# ── Build ─────────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "[3/5] Building NetShare (release)..."
cd "$REPO_ROOT/netshare"
cargo build --release -p netshare-gui

# ── Install binary ────────────────────────────────────────────────────────────
echo "[4/5] Installing to /usr/local/bin/netshare..."
sudo install -m 755 target/release/netshare-gui /usr/local/bin/netshare

# Desktop entry
sudo install -d /usr/local/share/applications
sudo tee /usr/local/share/applications/netshare.desktop >/dev/null <<'EOF'
[Desktop Entry]
Name=NetShare
Comment=Share keyboard, mouse, and audio across LAN
Exec=netshare
Icon=netshare
Terminal=false
Type=Application
Categories=Network;Utility;
EOF

# ── Input group ───────────────────────────────────────────────────────────────
echo "[5/5] Adding $USER to 'input' group (required for evdev/uinput)..."
sudo usermod -aG input "$USER"

echo ""
echo "Installation complete!"
echo ""
echo "  IMPORTANT: Log out and back in for group membership to take effect,"
echo "  then launch NetShare from your app menu or run: netshare"
echo ""
