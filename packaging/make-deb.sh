#!/usr/bin/env bash
# Build a .deb package for netshare-gui on Ubuntu/Debian.
# Run from the repo root after `cargo build --release -p netshare-gui`.
set -euo pipefail

BINARY="netshare/target/release/netshare-gui"
VERSION="${NETSHARE_VERSION:-0.1.0}"
ARCH="amd64"
PKG="netshare_${VERSION}_${ARCH}"
DIST="dist"

if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: Binary not found at $BINARY — run cargo build --release first."
    exit 1
fi

mkdir -p "$DIST"
ROOT="$DIST/$PKG"
rm -rf "$ROOT"

# ── Package directory structure ───────────────────────────────────────────────
mkdir -p "$ROOT/DEBIAN"
mkdir -p "$ROOT/usr/bin"
mkdir -p "$ROOT/usr/share/applications"
mkdir -p "$ROOT/usr/share/icons/hicolor/256x256/apps"

# Binary
cp "$BINARY" "$ROOT/usr/bin/netshare"
chmod 755 "$ROOT/usr/bin/netshare"

# Desktop entry
cat > "$ROOT/usr/share/applications/netshare.desktop" <<'EOF'
[Desktop Entry]
Name=NetShare
Comment=Share keyboard, mouse, and audio across LAN
Exec=netshare
Icon=netshare
Terminal=false
Type=Application
Categories=Network;Utility;
EOF

# Icon (copy if present, otherwise skip)
if [[ -f "netshare/assets/icon-256.png" ]]; then
    cp "netshare/assets/icon-256.png" "$ROOT/usr/share/icons/hicolor/256x256/apps/netshare.png"
fi

# Control file
cat > "$ROOT/DEBIAN/control" <<EOF
Package: netshare
Version: $VERSION
Section: net
Priority: optional
Architecture: $ARCH
Depends: libasound2 (>= 1.0), libssl3 | libssl1.1, libx11-6, libxcb1
Maintainer: NetShare <noreply@netshare.local>
Description: LAN keyboard/mouse/audio sharing
 NetShare lets you control multiple computers over your local network
 using a single keyboard, mouse, and share audio between them.
 Seamless cursor hand-off works like Synergy/Barrier.
EOF

# Post-install: add user to input group so uinput/evdev works without sudo
cat > "$ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    CURRENT_USER="${SUDO_USER:-$USER}"
    if [ -n "$CURRENT_USER" ] && id "$CURRENT_USER" >/dev/null 2>&1; then
        usermod -aG input "$CURRENT_USER" 2>/dev/null || true
        echo "NetShare: added $CURRENT_USER to 'input' group."
        echo "         Please log out and back in for input access to take effect."
    fi
fi
EOF
chmod 755 "$ROOT/DEBIAN/postinst"

# Build .deb
dpkg-deb --build --root-owner-group "$ROOT" "$DIST/${PKG}.deb"
echo "Built: $DIST/${PKG}.deb"
