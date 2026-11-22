#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Runs actionlint on .github/workflows/v3-*.yml. actionlint is downloaded into v3/.tools/
# (gitignored), pinned by version and SHA-256 per platform, unless it is already there.
# V3_ROOT is the directory that plays the role of v3/; V3_TOOLS overrides the tool directory.
set -euo pipefail

VERSION="1.7.12"

sha_for() {
  case "$1" in
    darwin_amd64) echo 5b44c3bc2255115c9b69e30efc0fecdf498fdb63c5d58e17084fd5f16324c644 ;;
    darwin_arm64) echo aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f ;;
    linux_amd64)  echo 8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
    linux_arm64)  echo 325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6 ;;
    *) echo "" ;;
  esac
}

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"
V3_TOOLS="${V3_TOOLS:-$V3_ROOT/.tools}"
repo="$(cd "$V3_ROOT/.." && pwd)"

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) echo "check-workflows.sh: unsupported OS $(uname -s)" >&2; exit 2 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=amd64 ;;
  arm64|aarch64) arch=arm64 ;;
  *) echo "check-workflows.sh: unsupported CPU $(uname -m)" >&2; exit 2 ;;
esac
platform="${os}_${arch}"
bin="$V3_TOOLS/actionlint-$VERSION/actionlint"

if [ ! -x "$bin" ]; then
  want="$(sha_for "$platform")"
  [ -n "$want" ] || { echo "check-workflows.sh: no pinned checksum for $platform" >&2; exit 2; }
  curl_bin=curl
  [ -x /usr/bin/curl ] && curl_bin=/usr/bin/curl   # MacPorts curl may lack current CA roots
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/actionlint.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  archive="actionlint_${VERSION}_${platform}.tar.gz"
  "$curl_bin" -fsSL -o "$tmp/$archive" \
    "https://github.com/rhysd/actionlint/releases/download/v$VERSION/$archive"
  if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$tmp/$archive" | cut -d' ' -f1)"
  else
    got="$(/usr/bin/shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)"
  fi
  if [ "$got" != "$want" ]; then
    echo "check-workflows.sh: checksum mismatch for $archive (got $got, want $want)" >&2
    exit 2
  fi
  mkdir -p "$V3_TOOLS/actionlint-$VERSION"
  tar -xzf "$tmp/$archive" -C "$V3_TOOLS/actionlint-$VERSION" actionlint
fi

cd "$repo"
set -- .github/workflows/v3-*.yml
if [ ! -e "$1" ]; then
  echo "no v3 workflows to check"
  exit 0
fi
"$bin" -color=false "$@"
echo "actionlint OK ($# workflows)"
