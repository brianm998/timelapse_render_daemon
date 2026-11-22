#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Checks that every tracked source file under v3/, .mason/ and .github/workflows/v3-*.yml carries
# both SPDX lines (overview §10 rule 6). Skips generated files, JSON and anything under a
# `fixtures/` directory. Prints the offending files and exits 1.
# V3_ROOT is the directory that plays the role of v3/ (default: the real one).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"
repo="$(cd "$V3_ROOT/.." && pwd)"
v3name="$(basename "$V3_ROOT")"

list_files() {
  if git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
     && [ "$(git -C "$repo" rev-parse --show-toplevel)" = "$(cd "$repo" && pwd -P)" ]; then
    git -C "$repo" ls-files -- "$v3name" .mason ".github/workflows/v3-*.yml"
  else
    (cd "$repo" && find "$v3name" .mason -type f 2>/dev/null; \
     ls .github/workflows/v3-*.yml 2>/dev/null || true)
  fi
}

wants() {
  case "$1" in
    */fixtures/*|fixtures/*) return 1 ;;
    */generated/*|*/Generated/*) return 1 ;;
    *.pb.swift|*.grpc.swift|*.json) return 1 ;;
  esac
  case "$(basename "$1")" in
    Makefile|*.mk|*.rs|*.kt|*.kts|*.swift|*.sh|*.py|*.proto|*.toml|*.yml|*.yaml|*.plist|*.xml) return 0 ;;
  esac
  return 1
}

bad=0
checked=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  wants "$f" || continue
  [ -f "$repo/$f" ] || continue
  checked=$((checked + 1))
  head20="$(head -n 20 "$repo/$f")"
  if ! printf '%s\n' "$head20" | grep -q 'SPDX-License-Identifier: GPL-3.0-or-later' \
     || ! printf '%s\n' "$head20" | grep -q 'Copyright (C) 2026 Timelapse Render Daemon contributors'; then
    echo "missing SPDX header: $f"
    bad=1
  fi
done <<EOT
$(list_files)
EOT

if [ "$bad" -ne 0 ]; then
  exit 1
fi
echo "SPDX OK ($checked files)"
