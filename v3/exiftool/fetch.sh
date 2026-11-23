#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Usage: fetch.sh unix|windows <dest>
#
# Fetches the ExifTool pinned in versions.env, checks its SHA-256 and extracts the bundle into
# <dest>, in the layout spec 1 §6.15 expects:
#   unix:    <dest>/exiftool (the Perl script), <dest>/lib/, <dest>/LICENSE-exiftool.txt
#   windows: <dest>/exiftool.exe (renamed from exiftool(-k).exe), <dest>/exiftool_files/,
#            <dest>/LICENSE-exiftool.txt
# A checksum mismatch exits 2 with "checksum mismatch" and writes nothing under <dest>. Old outputs
# in <dest> are replaced only after the archive has been verified and extracted.
#
# Environment hooks (tests and offline use):
#   EXIFTOOL_VERSIONS_FILE  versions file to read instead of versions.env
#   EXIFTOOL_LOCAL_ARCHIVE  archive to use instead of downloading it (its checksum is still checked)
#   EXIFTOOL_WORK_DIR       work directory instead of v3/exiftool/.work
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
versions="${EXIFTOOL_VERSIONS_FILE:-$here/versions.env}"
work="${EXIFTOOL_WORK_DIR:-$here/.work}"

die() { echo "fetch.sh: $1" >&2; exit "${2:-1}"; }

if [ "$#" -ne 2 ]; then
  die "usage: fetch.sh unix|windows <dest>" 2
fi
kind="$1"
dest="$2"
case "$kind" in
  unix|windows) ;;
  *) die "unknown bundle '$kind' (want unix or windows)" 2 ;;
esac

[ -f "$versions" ] || die "no versions file at $versions" 2
# shellcheck source=/dev/null
. "$versions"
[ -n "${EXIFTOOL_VERSION:-}" ] || die "versions.env has no EXIFTOOL_VERSION (not pinned yet)" 2

if [ "$kind" = unix ]; then
  url="${EXIFTOOL_UNIX_URL:-}"
  want="${EXIFTOOL_UNIX_SHA256:-}"
  mirror=""
  archive_name="exiftool-${EXIFTOOL_VERSION}-unix.tar.gz"
else
  url="${EXIFTOOL_WINDOWS_URL:-}"
  want="${EXIFTOOL_WINDOWS_SHA256:-}"
  mirror="${EXIFTOOL_WINDOWS_MIRROR:-}"
  archive_name="exiftool-${EXIFTOOL_VERSION}-windows.zip"
fi
[ -n "$url" ] || die "versions.env has no download URL for $kind" 2
[ -n "$want" ] || die "versions.env has no SHA-256 for $kind" 2

# sha256_of <file>: prints the lower-case SHA-256 of the file.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1 | tr 'A-F' 'a-f'
  else
    /usr/bin/shasum -a 256 "$1" | cut -d' ' -f1 | tr 'A-F' 'a-f'
  fi
}

# download <url> <file>: /usr/bin/curl when present (MacPorts curl may lack current CA roots).
download() {
  local curl_bin=curl
  if [ -x /usr/bin/curl ]; then curl_bin=/usr/bin/curl; fi
  "$curl_bin" -fsSL --retry 3 -o "$2" "$1"
}

# find_one <dir> <name pattern>: the first file under <dir> whose name matches, or nothing.
find_one() {
  find "$1" -type f -iname "$2" 2>/dev/null | sort | sed -n 1p
}

mkdir -p "$work"
downloaded=0
if [ -n "${EXIFTOOL_LOCAL_ARCHIVE:-}" ]; then
  archive="$EXIFTOOL_LOCAL_ARCHIVE"
  [ -f "$archive" ] || die "EXIFTOOL_LOCAL_ARCHIVE is not a file: $archive" 2
else
  archive="$work/$archive_name"
  downloaded=1
  rm -f "$archive"
  if ! download "$url" "$archive"; then
    rm -f "$archive"
    [ -n "$mirror" ] || die "download failed: $url"
    echo "fetch.sh: download from $url failed; trying the mirror" >&2
    download "$mirror" "$archive" || { rm -f "$archive"; die "download failed from $url and $mirror"; }
  fi
fi

got="$(sha256_of "$archive")"
if [ "$got" != "$(printf '%s' "$want" | tr 'A-F' 'a-f')" ]; then
  if [ "$downloaded" -eq 1 ]; then rm -f "$archive"; fi
  echo "fetch.sh: checksum mismatch for $archive_name (got $got, want $want)" >&2
  exit 2
fi

stage="$(mktemp -d "$work/extract.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
out="$stage/out"
mkdir -p "$out"

if [ "$kind" = unix ]; then
  tar -xzf "$archive" -C "$stage" --strip-components=1
  [ -f "$stage/exiftool" ] || die "the archive has no exiftool script at its top level"
  [ -d "$stage/lib" ] || die "the archive has no lib/ directory"
  cp -p "$stage/exiftool" "$out/exiftool"
  cp -Rp "$stage/lib" "$out/lib"
  names="exiftool lib LICENSE-exiftool.txt"
  exe_dir="$stage"
else
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$archive" -d "$stage"
  elif command -v 7z >/dev/null 2>&1; then
    7z x -y -o"$stage" "$archive" >/dev/null
  else
    die "need unzip or 7z to extract $archive_name"
  fi
  # The official zip stores exiftool_files/ read-only (dr-xr-xr-x). cp -p keeps that mode, and a
  # read-only directory can neither be moved to another parent nor cleaned up by the EXIT trap.
  chmod -R u+w "$stage"
  exe_src="$(find_one "$stage" 'exiftool(-k).exe')"
  if [ -z "$exe_src" ]; then exe_src="$(find_one "$stage" 'exiftool.exe')"; fi
  [ -n "$exe_src" ] || die "the archive has no exiftool(-k).exe"
  exe_dir="$(dirname "$exe_src")"
  [ -d "$exe_dir/exiftool_files" ] || die "the archive has no exiftool_files/ next to the exe"
  cp -p "$exe_src" "$out/exiftool.exe"
  cp -Rp "$exe_dir/exiftool_files" "$out/exiftool_files"
  names="exiftool.exe exiftool_files LICENSE-exiftool.txt"
fi

licence=""
for pattern in 'LICEN[CS]E*' 'COPYING*' 'Copying*' 'Copyright*'; do
  licence="$(find_one "$stage" "$pattern")"
  if [ -n "$licence" ]; then break; fi
done
[ -n "$licence" ] || die "no licence file found in $archive_name"
cp -p "$licence" "$out/LICENSE-exiftool.txt"

mkdir -p "$dest"
for name in $names; do rm -rf "$dest/$name"; done
for name in $names; do mv "$out/$name" "$dest/$name"; done
echo "fetch.sh: wrote the $kind bundle (ExifTool $EXIFTOOL_VERSION) to $dest"
