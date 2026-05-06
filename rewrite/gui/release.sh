#!/bin/bash

# Builds the SwiftUI GUI for macOS, wraps the SwiftPM executable in a .app
# bundle, and packages it as a signed/notarized .pkg installer.
#
# Configurable via environment variables:
#   SIGN_IDENTITY      — Developer ID Installer identity for the .pkg
#   APP_SIGN_IDENTITY  — Developer ID Application identity for the .app
#   NOTARY_PROFILE     — `notarytool` keychain profile (default: "timelapse-render")
#   SKIP_SIGN=1        — skip signing and notarization

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

VERSION="$(../TimelapseRenderCore/version.sh)"
APP_NAME="TimelapseRenderGUI"
BUNDLE_ID="com.timelapse-render-daemon.gui"
APP_DIR=".build/${APP_NAME}.app"
PKG_NAME=".build/timelapse-render-gui_${VERSION}.pkg"

SIGN_IDENTITY="${SIGN_IDENTITY:-Developer ID Installer: Brian Martin (G3L75S65V9)}"
APP_SIGN_IDENTITY="${APP_SIGN_IDENTITY:-Developer ID Application: Brian Martin (G3L75S65V9)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-timelapse-render}"

echo "==> timelapse-render GUI v${VERSION} (macOS)"

rm -rf .build

# ── Build ────────────────────────────────────────────────────────────────────
echo "==> Building release binary (universal arm64+x86_64)..."
swift build -c release \
    --arch arm64 --arch x86_64

BIN=".build/apple/Products/Release/${APP_NAME}"
if [ ! -f "$BIN" ]; then
    BIN=".build/release/${APP_NAME}"
fi
[ -f "$BIN" ] || { echo "ERROR: built binary not found"; exit 1; }

# ── Build .app bundle ────────────────────────────────────────────────────────
echo "==> Assembling ${APP_NAME}.app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$BIN" "$APP_DIR/Contents/MacOS/${APP_NAME}"

cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleName</key><string>Timelapse Render Daemon</string>
    <key>CFBundleDisplayName</key><string>Timelapse Render Daemon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# ── Sign / notarize ──────────────────────────────────────────────────────────
if [ "${SKIP_SIGN:-0}" = "1" ]; then
    echo "==> SKIP_SIGN=1, producing UNSIGNED .app and .pkg"
    pkgbuild --root "$APP_DIR" \
             --identifier "$BUNDLE_ID" \
             --version "$VERSION" \
             --install-location "/Applications/${APP_NAME}.app" \
             "$PKG_NAME"
    echo ""
    echo "==> UNSIGNED package: $PKG_NAME"
    exit 0
fi

echo "==> Codesigning .app..."
codesign --force --deep --options=runtime --timestamp \
         --sign "$APP_SIGN_IDENTITY" \
         "$APP_DIR"

echo "==> Notarizing .app..."
NOTARIZE_ZIP=".build/${APP_NAME}-for-notarization.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$NOTARIZE_ZIP"
xcrun notarytool submit "$NOTARIZE_ZIP" \
      --keychain-profile "$NOTARY_PROFILE" \
      --wait
rm -f "$NOTARIZE_ZIP"

WAIT_TIME=20
until xcrun stapler staple "$APP_DIR"; do
    echo "wait ${WAIT_TIME} seconds for staple to settle..."
    sleep ${WAIT_TIME}
done

echo "==> Building signed .pkg..."
pkgbuild --root "$APP_DIR" \
         --identifier "$BUNDLE_ID" \
         --version "$VERSION" \
         --install-location "/Applications/${APP_NAME}.app" \
         --sign "$SIGN_IDENTITY" \
         "$PKG_NAME"

echo "==> Notarizing .pkg..."
xcrun notarytool submit "$PKG_NAME" \
      --keychain-profile "$NOTARY_PROFILE" \
      --wait
xcrun stapler staple "$PKG_NAME"

echo ""
echo "==> Signed, notarized, stapled package: $PKG_NAME"
