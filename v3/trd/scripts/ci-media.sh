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

# The repository root (v3/) is one level above the trd workspace.
V3_ROOT="$(cd .. && pwd)"

# provision_ffmpeg: a machine with no ffmpeg on PATH and no bundle in TRD_FFMPEG_DIR gets the one
# the ffmpeg component builds for its native slice, as the ffmpeg-smoke suite does. The daemon then
# reads it through TRD_FFMPEG_DIR (§6.15), so the test never falls back to a different binary.
provision_ffmpeg() {
  [ -z "${TRD_FFMPEG_DIR:-}" ] || return 0
  command -v ffmpeg >/dev/null 2>&1 && return 0
  local target
  target="$(bash "$V3_ROOT/ffmpeg/build.sh" --print-native)"
  if [ ! -x "$V3_ROOT/ffmpeg/dist/$target/ffmpeg" ] || [ ! -x "$V3_ROOT/ffmpeg/dist/$target/ffprobe" ]; then
    echo "ci-media.sh: no ffmpeg on PATH; building the $target slice in v3/ffmpeg" >&2
    bash "$V3_ROOT/ffmpeg/build.sh" "$target"
  fi
  export TRD_FFMPEG_DIR="$V3_ROOT/ffmpeg/dist/$target"
}

# provision_exiftool: the same for exiftool, fetched into v3/exiftool/dist/unix.
provision_exiftool() {
  [ -z "${TRD_EXIFTOOL_DIR:-}" ] || return 0
  command -v exiftool >/dev/null 2>&1 && return 0
  if [ ! -f "$V3_ROOT/exiftool/dist/unix/exiftool" ]; then
    echo "ci-media.sh: no exiftool on PATH; fetching the unix bundle into v3/exiftool/dist/unix" >&2
    bash "$V3_ROOT/exiftool/fetch.sh" unix "$V3_ROOT/exiftool/dist/unix"
  fi
  export TRD_EXIFTOOL_DIR="$V3_ROOT/exiftool/dist/unix"
}

provision_ffmpeg
provision_exiftool

export TRD_E2E=1
cargo test -p trd --test e2e --locked
cargo test -p trd-core --test xmp --locked
