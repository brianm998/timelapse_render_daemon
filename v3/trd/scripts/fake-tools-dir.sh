#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Makes <dest> a directory holding the fake tools as ffmpeg, ffprobe and exiftool (with .exe names
# on Windows), so TRD_FFMPEG_DIR and TRD_EXIFTOOL_DIR can name it (spec 1 TRD-05). For scripts
# outside cargo tests: the client contract tests and the sandbox tests. It builds trd-testtools
# unless TRD_FAKE_BIN_DIR names a directory that already holds the fake binaries. Bash 3.2.
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: fake-tools-dir.sh <dest>" >&2
  exit 2
fi
dest="$1"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ws="$(cd "$here/.." && pwd)"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) suffix=".exe" ;;
  *) suffix="" ;;
esac

if [ -n "${TRD_FAKE_BIN_DIR:-}" ]; then
  bin_dir="$TRD_FAKE_BIN_DIR"
else
  (cd "$ws" && cargo build --quiet --locked -p trd-testtools --bins)
  bin_dir="${CARGO_TARGET_DIR:-$ws/target}/debug"
fi

mkdir -p "$dest"
for pair in fake-ffmpeg:ffmpeg fake-ffprobe:ffprobe fake-exiftool:exiftool; do
  from="${pair%%:*}"
  to="${pair##*:}"
  cp "$bin_dir/$from$suffix" "$dest/$to$suffix"
  chmod 755 "$dest/$to$suffix"
  # A copied binary's ad-hoc signature names the old file, so sign each copy again (rule 8).
  if [ "$(uname -s)" = "Darwin" ]; then
    /usr/bin/codesign -s - -f "$dest/$to$suffix"
  fi
done
