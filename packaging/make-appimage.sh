#!/usr/bin/env bash
# Build an AppImage for netshare-gui.
# Run from the repo root after `cargo build --release -p netshare-gui`.
# Requires: fuse / libfuse2, patchelf, wget (all installed by CI).
set -euo pipefail

BINARY="netshare/target/release/netshare-gui"
VERSION="${NETSHARE_VERSION:-0.1.0}"
DIST="dist"
APPDIR="$DIST/NetShare.AppDir"

if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: Binary not found at $BINARY — run cargo build --release first."
    exit 1
fi

mkdir -p "$DIST"

# Download linuxdeploy if not cached
LINUXDEPLOY="$DIST/linuxdeploy-x86_64.AppImage"
if [[ ! -f "$LINUXDEPLOY" ]]; then
    echo "Downloading linuxdeploy..."
    wget -q -O "$LINUXDEPLOY" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
    chmod +x "$LINUXDEPLOY"
fi

# ── Build AppDir ──────────────────────────────────────────────────────────────
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp "$BINARY" "$APPDIR/usr/bin/netshare"
chmod 755 "$APPDIR/usr/bin/netshare"

cat > "$APPDIR/usr/share/applications/netshare.desktop" <<'EOF'
[Desktop Entry]
Name=NetShare
Comment=Share keyboard, mouse, and audio across LAN
Exec=netshare
Icon=netshare
Terminal=false
Type=Application
Categories=Network;Utility;
EOF

# Icon
if [[ -f "netshare/assets/icon-256.png" ]]; then
    cp "netshare/assets/icon-256.png" \
       "$APPDIR/usr/share/icons/hicolor/256x256/apps/netshare.png"
else
    # Create a minimal placeholder PNG (1x1 pixel) so linuxdeploy doesn't fail
    printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\x0f\x00\x00\x01\x01\x00\x05\x18\xd8N\x00\x00\x00\x00IEND\xaeB`\x82' \
        > "$APPDIR/usr/share/icons/hicolor/256x256/apps/netshare.png"
fi

# ── Run linuxdeploy to bundle shared libraries ────────────────────────────────
# FUSE_AVAILABLE=0 tells linuxdeploy to extract rather than mount (works in CI)
ARCH=x86_64 \
FUSE_AVAILABLE=0 \
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --output appimage

# linuxdeploy writes NetShare-x86_64.AppImage in the current directory
GENERATED="NetShare-x86_64.AppImage"
if [[ -f "$GENERATED" ]]; then
    mv "$GENERATED" "$DIST/NetShare-${VERSION}-x86_64.AppImage"
    echo "Built: $DIST/NetShare-${VERSION}-x86_64.AppImage"
else
    echo "WARNING: AppImage not found at expected path — check linuxdeploy output above."
fi
