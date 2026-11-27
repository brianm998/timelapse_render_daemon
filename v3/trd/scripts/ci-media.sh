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

# Outside CI (GITHUB_ACTIONS unset) a machine may have no ffmpeg or exiftool and no way to build or
# fetch them (no meson, no network). Then the real-tool path cannot run there: skip() prints a
# SKIPPED notice and runs the same tests without TRD_E2E, so they return early and the suite still
# reports them. In CI a missing tool is an error: the trd-media job always provides both bundles.
SKIP_REASON=""
skip() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "ci-media.sh: $1" >&2
    exit 1
  fi
  [ -n "$SKIP_REASON" ] || SKIP_REASON="$1"
}

# have_tool NAME: NAME resolves on PATH or in MacPorts' /opt/local/bin (TRD_MACPORTS_BIN overrides
# that directory; the script tests use it to hide a real MacPorts install). Prints the directory.
have_tool() {
  local p macports
  macports="${TRD_MACPORTS_BIN:-/opt/local/bin}"
  p="$(command -v "$1" 2>/dev/null || true)"
  if [ -n "$p" ] && [ -x "$p" ]; then dirname "$p"; return 0; fi
  if [ -x "$macports/$1" ]; then echo "$macports"; return 0; fi
  return 1
}

# provision_ffmpeg: a machine with no ffmpeg on PATH and no bundle in TRD_FFMPEG_DIR gets the one
# the ffmpeg component builds for its native slice, as the ffmpeg-smoke suite does. The daemon then
# reads it through TRD_FFMPEG_DIR (§6.15), so the test never falls back to a different binary.
provision_ffmpeg() {
  [ -z "${TRD_FFMPEG_DIR:-}" ] || return 0
  local dir target
  if dir="$(have_tool ffmpeg)" && [ -x "$dir/ffprobe" ]; then
    export TRD_FFMPEG_DIR="$dir"
    return 0
  fi
  target="$(bash "$V3_ROOT/ffmpeg/build.sh" --print-native 2>/dev/null || true)"
  if [ -z "$target" ]; then
    skip "no ffmpeg on PATH or in /opt/local/bin, and no native ffmpeg target to build"
    return 0
  fi
  if [ ! -x "$V3_ROOT/ffmpeg/dist/$target/ffmpeg" ] || [ ! -x "$V3_ROOT/ffmpeg/dist/$target/ffprobe" ]; then
    echo "ci-media.sh: no ffmpeg on PATH; building the $target slice in v3/ffmpeg" >&2
    if ! bash "$V3_ROOT/ffmpeg/build.sh" "$target"; then
      skip "no ffmpeg on PATH or in /opt/local/bin, and the $target slice could not be built"
      return 0
    fi
  fi
  export TRD_FFMPEG_DIR="$V3_ROOT/ffmpeg/dist/$target"
}

# provision_exiftool: the same for exiftool, fetched into v3/exiftool/dist/unix.
provision_exiftool() {
  [ -z "${TRD_EXIFTOOL_DIR:-}" ] || return 0
  local dir
  if dir="$(have_tool exiftool)"; then
    export TRD_EXIFTOOL_DIR="$dir"
    return 0
  fi
  if [ ! -f "$V3_ROOT/exiftool/dist/unix/exiftool" ]; then
    echo "ci-media.sh: no exiftool on PATH; fetching the unix bundle into v3/exiftool/dist/unix" >&2
    if ! bash "$V3_ROOT/exiftool/fetch.sh" unix "$V3_ROOT/exiftool/dist/unix"; then
      skip "no exiftool on PATH or in /opt/local/bin, and the unix bundle could not be fetched"
      return 0
    fi
  fi
  export TRD_EXIFTOOL_DIR="$V3_ROOT/exiftool/dist/unix"
}

provision_ffmpeg
provision_exiftool

if [ -n "$SKIP_REASON" ]; then
  echo "ci-media.sh: SKIPPED the real-tool path: $SKIP_REASON." >&2
  echo "ci-media.sh: SKIPPED: running the e2e and XMP tests without TRD_E2E; they return early." >&2
  unset TRD_E2E
else
  export TRD_E2E=1
fi
# The tests drive fake-ffmpeg, fake-ffprobe and fake-exiftool, which are bin targets of
# trd-testtools; cargo test does not build them for a dev-dependency, so build them here.
cargo build --locked -p trd-testtools --bins
# Each run is captured; on failure its panic lines are printed last, so a truncated log tail shows
# why it failed.
LOG="$(mktemp "${TMPDIR:-/tmp}/ci-media.XXXXXX")"
trap 'rm -f "$LOG"' EXIT
status=0
run_tests() {
  if ! "$@" 2>&1 | tee "$LOG"; then
    status=1
    echo "ci-media.sh: FAILED: $*" >&2
    grep -A3 "panicked at" "$LOG" >&2 || true
  fi
}
run_tests cargo test -p trd --test e2e --locked
run_tests cargo test -p trd-core --test xmp --locked
exit "$status"
