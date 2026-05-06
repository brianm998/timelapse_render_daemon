#!/bin/bash

# Builds the timelapse-render CLI for Windows and packages it as a .zip file.
#
# Prerequisites:
#   - Swift toolchain (https://swift.org/install/windows)
#   - Git for Windows (provides this bash, sed, etc.)
#   - ffmpeg.exe and exiftool.exe must be available at runtime (not bundled).
#
# Run this script from Git Bash, not from PowerShell or cmd.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR"

VERSION="$("$REPO_ROOT/TimelapseRenderCore/version.sh")"
echo "==> Building timelapse-render CLI v${VERSION} for Windows"

# ── 1. CLI ────────────────────────────────────────────────────────────────────
echo "==> Building (release configuration)..."
cd "$REPO_ROOT/cli"
# No -static-stdlib on Windows; Swift runtime DLLs are bundled in the zip instead.
swift build -c release

BINARY="$REPO_ROOT/cli/.build/release/timelapse-render.exe"
[ -f "$BINARY" ] || { echo "ERROR: built binary not found"; exit 1; }

echo "==> Build complete: $BINARY"

# ── 2. .zip package ──────────────────────────────────────────────────────────
echo "==> Creating .zip package..."

case "$(uname -m)" in
    x86_64)  WIN_ARCH="x64" ;;
    aarch64) WIN_ARCH="arm64" ;;
    *)       WIN_ARCH="$(uname -m)" ;;
esac

PKG_STEM="timelapse-render_${VERSION}_windows_${WIN_ARCH}"
PKG_DIR="$REPO_ROOT/cli/.build/${PKG_STEM}"
ZIP_FILE="$REPO_ROOT/cli/.build/${PKG_STEM}.zip"

rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR"
cp "$BINARY" "$PKG_DIR/"

# Bundle Swift runtime DLLs so the binary runs without a Swift installation.
SWIFT_BIN="$(dirname "$(command -v swift.exe 2>/dev/null || command -v swift)")"
echo "==> Bundling Swift runtime DLLs from: $SWIFT_BIN"
DLL_COUNT=0
for dll in \
    "$SWIFT_BIN"/swift*.dll \
    "$SWIFT_BIN"/BlocksRuntime.dll \
    "$SWIFT_BIN"/dispatch.dll \
    "$SWIFT_BIN"/msvcp*.dll \
    "$SWIFT_BIN"/vcruntime*.dll \
    "$SWIFT_BIN"/ucrtbase*.dll; do
    if [ -f "$dll" ]; then
        cp "$dll" "$PKG_DIR/"
        DLL_COUNT=$((DLL_COUNT + 1))
    fi
done
echo "    bundled $DLL_COUNT DLL(s)"

cd "$(dirname "$ZIP_FILE")"
zip -r "$(basename "$ZIP_FILE")" "$(basename "$PKG_DIR")/"
rm -rf "$PKG_DIR"

echo ""
echo "==> Package: $ZIP_FILE"
echo "    Install:  unzip $PKG_STEM.zip"
echo "              Add the extracted directory to PATH,"
echo "              or copy timelapse-render.exe and the .dll files to a directory already in PATH."
