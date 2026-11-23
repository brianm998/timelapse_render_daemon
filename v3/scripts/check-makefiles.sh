#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# For every component of overview §9 whose directory exists, `make -C <dir> -n <target>` must
# succeed for run, develop, release, clean, test, check and ci (a dry run: nothing is built or
# started). v3/Makefile is checked the same way. Prints each missing target and exits 1.
# A component directory whose only entries are generated build/ or .gradle/ output is not a
# component yet, so it is skipped rather than reported. suite.sh's kotlin-test and android-test
# write their junit placeholders into clients/kotlin/build (mason.toml reads them from there),
# so a tree holding only that output must not fail the Makefile check.
# V3_ROOT is the directory that plays the role of v3/ (default: the real one).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"

targets="run develop release clean test check ci"
bad=0
checked=0

# generated_only <dir>: true when dir has at least one entry (dotfiles included), every entry is a
# directory named build or .gradle, and there is no Makefile. An empty directory is not skipped.
generated_only() {
  local dir="$1" entry name generated=0 others=0
  for entry in "$dir"/* "$dir"/.[!.]* "$dir"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    name="${entry##*/}"
    if { [ "$name" = build ] || [ "$name" = .gradle ]; } && [ -d "$entry" ]; then
      generated=1
    else
      others=1
    fi
  done
  [ "$generated" -eq 1 ] && [ "$others" -eq 0 ]
}

check_dir() {
  local dir="$1" label="$2" t
  if [ ! -f "$dir/Makefile" ]; then
    echo "$label: no Makefile"
    bad=1
    return
  fi
  for t in $targets; do
    if ! make -C "$dir" -n "$t" >/dev/null 2>&1; then
      echo "$label: missing or failing target '$t'"
      bad=1
    fi
  done
  checked=$((checked + 1))
}

check_dir "$V3_ROOT" "v3"
for comp in ffmpeg exiftool trd clients/kotlin clients/kotlin/android ios; do
  if [ -d "$V3_ROOT/$comp" ] && ! generated_only "$V3_ROOT/$comp"; then
    check_dir "$V3_ROOT/$comp" "v3/$comp"
  fi
done

if [ "$bad" -ne 0 ]; then
  exit 1
fi
echo "Makefiles OK ($checked checked)"
