#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Usage: bump-version.sh X.Y.Z[-pre]
# Writes v3/VERSION, [workspace.package] version in v3/trd/Cargo.toml and MARKETING_VERSION in
# v3/ios/project.yml (without the pre-release suffix: App Store Connect does not accept one), when
# those files exist (the Kotlin build reads v3/VERSION). Prints the commit and tag commands.
# V3_ROOT is the directory that plays the role of v3/ (default: the real one).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"

if [ "$#" -ne 1 ]; then
  echo "usage: bump-version.sh X.Y.Z[-pre]" >&2
  exit 2
fi
version="$1"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "bump-version.sh: '$version' is not X.Y.Z[-pre]" >&2; exit 2 ;;
esac
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
  echo "bump-version.sh: '$version' is not X.Y.Z[-pre]" >&2
  exit 2
fi
marketing="${version%%-*}"

printf '%s\n' "$version" > "$V3_ROOT/VERSION"
echo "wrote $V3_ROOT/VERSION: $version"

cargo="$V3_ROOT/trd/Cargo.toml"
if [ -f "$cargo" ]; then
  tmp="$(mktemp "${TMPDIR:-/tmp}/bump.XXXXXX")"
  awk -v v="$version" '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") }
    in_pkg && /^version[ \t]*=/ { print "version = \"" v "\""; next }
    { print }
  ' "$cargo" > "$tmp"
  cat "$tmp" > "$cargo"
  rm -f "$tmp"
  echo "wrote $cargo: $version"
fi

project="$V3_ROOT/ios/project.yml"
if [ -f "$project" ]; then
  tmp="$(mktemp "${TMPDIR:-/tmp}/bump.XXXXXX")"
  awk -v v="$marketing" '
    /^[ \t]*MARKETING_VERSION:/ {
      match($0, /^[ \t]*/)
      print substr($0, 1, RLENGTH) "MARKETING_VERSION: \"" v "\""
      next
    }
    { print }
  ' "$project" > "$tmp"
  cat "$tmp" > "$project"
  rm -f "$tmp"
  echo "wrote $project: MARKETING_VERSION \"$marketing\""
fi

echo
echo "Next:"
echo "  git commit -am \"v3: version $version\""
echo "  git tag v$version"
