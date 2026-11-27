#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The real-tool tests (spec 1 §9.6, §16; TRD-10 and TRD-GATE): the end-to-end daemon test in
# crates/trd/tests/e2e.rs and the real exiftool test in crates/trd-core/tests/xmp.rs. Both run only
# with TRD_E2E=1, and they fail rather than skip when ffmpeg, ffprobe or exiftool do not resolve
# (§6.15: TRD_FFMPEG_DIR and TRD_EXIFTOOL_DIR, then the development tree, then PATH). The gate's
# trd-media job sets those variables from the ffmpeg and exiftool bundles; the owner's Mac runs
# this through `bash v3/scripts/suite.sh trd-e2e`. It runs from the trd workspace, whatever the
# current directory is, and is bash 3.2 compatible.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

export TRD_E2E=1
cargo test -p trd --test e2e --locked
cargo test -p trd-core --test xmp --locked
