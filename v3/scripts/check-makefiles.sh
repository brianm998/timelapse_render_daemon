#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# For every component of overview §9 whose directory exists, `make -C <dir> -n <target>` must
# succeed for run, develop, release, clean, test, check and ci (a dry run: nothing is built or
# started). v3/Makefile is checked the same way. Prints each missing target and exits 1.
# V3_ROOT is the directory that plays the role of v3/ (default: the real one).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"

targets="run develop release clean test check ci"
bad=0
checked=0

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

# generated_only <dir>: true when <dir> has no Makefile and its only entries are generated
# build/ or .gradle/ output. suite.sh's kotlin-test and android-test write their placeholder junit
# files into clients/kotlin/build (mason.toml reads them from there), so a tree holding only that
# output is an absent component, not a component missing its Makefile. An empty directory, or one
# with any other entry, is not generated-only.
generated_only() {
  local dir="$1"
  [ ! -f "$dir/Makefile" ] || return 1
  [ -e "$dir/build" ] || [ -e "$dir/.gradle" ] || return 1
  [ -z "$(find "$dir" -mindepth 1 -maxdepth 1 ! -name build ! -name .gradle)" ]
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
