#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# What v3-gate.yml's `kotlin` job runs: build and test the desktop client and verify coverage.
set -euo pipefail
cd "$(dirname "$0")/.."
exec ./gradlew --no-daemon --console=plain -Djava.awt.headless=true \
  :proto:build :core:test :desktop:test :core:koverVerify :desktop:koverVerify
