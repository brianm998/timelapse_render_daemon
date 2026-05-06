#!/bin/bash

# Builds the timelapse-render CLI for Linux and packages it as a .deb file.
#
# Prerequisites (Debian/Ubuntu):
#   apt install dpkg-dev
#   # Swift toolchain from https://www.swift.org/download/
#
# Produces a .deb at cli/.build/timelapse-render_<version>_<arch>.deb

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR"

VERSION="$("$REPO_ROOT/TimelapseRenderCore/version.sh")"
echo "==> Building timelapse-render CLI v${VERSION} for Linux"

# ── 1. CLI ───────────────────────────────────────────────────────────────────
echo "==> Building (release configuration, static stdlib)..."
cd "$REPO_ROOT/cli"
# -static-stdlib links the Swift standard library into the binary so the
# target machine does not need a matching Swift toolchain installed.
swift build -c release -Xswiftc -static-stdlib

BINARY="$REPO_ROOT/cli/.build/release/timelapse-render"
strip "$BINARY"

echo "==> Build complete: $BINARY"

# ── 2. .deb package ──────────────────────────────────────────────────────────
echo "==> Creating .deb package..."

case "$(uname -m)" in
    x86_64)  DPKG_ARCH="amd64" ;;
    aarch64) DPKG_ARCH="arm64" ;;
    *)       DPKG_ARCH="$(uname -m)" ;;
esac

PKG_STEM="timelapse-render_${VERSION}_${DPKG_ARCH}"
PKG_DIR="$REPO_ROOT/cli/.build/${PKG_STEM}"
DEB_FILE="$REPO_ROOT/cli/.build/${PKG_STEM}.deb"

rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR/DEBIAN"
mkdir -p "$PKG_DIR/usr/local/bin"

cp "$BINARY" "$PKG_DIR/usr/local/bin/timelapse-render"
chmod 755 "$PKG_DIR/usr/local/bin/timelapse-render"

cat > "$PKG_DIR/DEBIAN/control" << EOF
Package: timelapse-render
Version: ${VERSION}
Architecture: ${DPKG_ARCH}
Maintainer: Brian Martin
Depends: libc6 (>= 2.17), ffmpeg, libimage-exiftool-perl
Section: video
Priority: optional
Description: Daemon that watches image-sequence directories and renders timelapse videos.
 Watches a configurable set of source paths for image sequences ready to be
 rendered by ffmpeg, dispatches per-variant render jobs, and produces output
 videos with exif metadata extracted from the source raws.
EOF

# --root-owner-group requires dpkg >= 1.19.1 (Ubuntu 18.10+).
dpkg-deb --build --root-owner-group "$PKG_DIR" "$DEB_FILE"
rm -rf "$PKG_DIR"

echo ""
echo "==> Package: $DEB_FILE"
echo "    Install:  sudo dpkg -i $DEB_FILE"
echo "    Remove:   sudo dpkg -r timelapse-render"
