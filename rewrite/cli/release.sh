#!/bin/bash

# Builds the timelapse-render CLI for macOS using SwiftPM and packages it as a
# signed/notarized .pkg installer.
#
# Signing/notarization can be skipped by setting SKIP_SIGN=1 — useful for local
# testing or for builds on machines without the developer certificate.
#
# Configurable via environment variables:
#   SIGN_IDENTITY      — Developer ID Installer / Application identity to use
#                        (default: "Developer ID Installer: Brian Martin (G3L75S65V9)")
#   APP_SIGN_IDENTITY  — Developer ID Application identity for codesign
#                        (default: "Developer ID Application: Brian Martin (G3L75S65V9)")
#   NOTARY_PROFILE     — `notarytool` keychain profile (default: "timelapse-render")
#   SKIP_SIGN=1        — skip signing and notarization (produces an unsigned .pkg)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

VERSION="$(../TimelapseRenderCore/version.sh)"
BUNDLE_ID="com.timelapse-render-daemon.cli"
PKG_NAME=".build/timelapse-render-cli_${VERSION}.pkg"

SIGN_IDENTITY="${SIGN_IDENTITY:-Developer ID Installer: Brian Martin (G3L75S65V9)}"
APP_SIGN_IDENTITY="${APP_SIGN_IDENTITY:-Developer ID Application: Brian Martin (G3L75S65V9)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-timelapse-render}"

echo "==> timelapse-render CLI v${VERSION} (macOS)"

rm -rf .build

# ── Build ────────────────────────────────────────────────────────────────────
echo "==> Building release binary (universal arm64+x86_64)..."
swift build -c release \
    --arch arm64 --arch x86_64

BIN=".build/apple/Products/Release/timelapse-render"
if [ ! -f "$BIN" ]; then
    # Fallback for older SwiftPM that doesn't lay down the apple/Products dir.
    BIN=".build/release/timelapse-render"
fi
[ -f "$BIN" ] || { echo "ERROR: built binary not found"; exit 1; }

# ── Stage ────────────────────────────────────────────────────────────────────
STAGE=".build/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE/usr/local/bin"
cp "$BIN" "$STAGE/usr/local/bin/timelapse-render"
chmod 755 "$STAGE/usr/local/bin/timelapse-render"

# ── Sign the binary ──────────────────────────────────────────────────────────
if [ "${SKIP_SIGN:-0}" = "1" ]; then
    echo "==> SKIP_SIGN=1, producing UNSIGNED package"
    pkgbuild --root "$STAGE/usr/local/bin" \
             --identifier "$BUNDLE_ID" \
             --version "$VERSION" \
             --install-location /usr/local/bin \
             "$PKG_NAME"
    echo ""
    echo "==> UNSIGNED package: $PKG_NAME"
    exit 0
fi

echo "==> Codesigning binary..."
codesign --force --options=runtime --timestamp \
         --sign "$APP_SIGN_IDENTITY" \
         "$STAGE/usr/local/bin/timelapse-render"

echo "==> Building signed .pkg..."
pkgbuild --root "$STAGE/usr/local/bin" \
         --identifier "$BUNDLE_ID" \
         --version "$VERSION" \
         --install-location /usr/local/bin \
         --sign "$SIGN_IDENTITY" \
         "$PKG_NAME"

echo "==> Submitting to notarytool (profile: ${NOTARY_PROFILE})..."
xcrun notarytool submit "$PKG_NAME" \
      --keychain-profile "$NOTARY_PROFILE" \
      --wait

echo "==> Stapling..."
xcrun stapler staple "$PKG_NAME"

echo ""
echo "==> Signed, notarized, stapled package: $PKG_NAME"
