#!/bin/bash

# Builds the timelapse-render CLI and GUI for macOS and packages them as
# .pkg installers.  Mirrors nighttime/release_darwin.sh.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

VERSION="$(./TimelapseRenderCore/version.sh)"
echo "==> timelapse-render v${VERSION} (macOS)"

# ── 1. CLI ────────────────────────────────────────────────────────────────────
echo "==> Building timelapse-render CLI..."
cd cli
./release.sh
cd "$SCRIPT_DIR"

# ── 2. GUI ────────────────────────────────────────────────────────────────────
echo "==> Building timelapse-render GUI..."
cd gui
./release.sh
cd "$SCRIPT_DIR"

if command -v say >/dev/null 2>&1; then
    say 'the release worked'
fi

echo ""
echo "==> Done. Artifacts:"
ls -1 cli/.build/*.pkg gui/.build/*.pkg 2>/dev/null | sed 's/^/    /' || true
