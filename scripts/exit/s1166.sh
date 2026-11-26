#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1166: the install scripts pass their stub tests, the icon files are present,
# and on macOS the installer self-check passes. The self-check needs lipo, codesign and pkgbuild,
# so it runs only on Darwin.
set -euo pipefail
cd "$(dirname "$0")/../.."

bash v3/trd/packaging/macos/test-scripts.sh

for icon in trd-1024.png trd.icns monitor-1024.png monitor-512.png monitor.icns monitor.ico; do
  test -s "v3/assets/icon/$icon"
done
echo "icon files present"

if [ "$(uname -s)" = Darwin ]; then
  bash v3/trd/packaging/macos/build-pkg.sh --self-check
fi
