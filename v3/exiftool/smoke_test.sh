#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Usage: smoke_test.sh <bundle dir>
#
# Runs the cases below against the ExifTool bundle in <bundle dir>: exiftool (a Perl script, run
# with /usr/bin/perl when it exists, else perl) or exiftool.exe on Windows. Prints `PASS <case>` or
# `FAIL <case>` for each case and exits 1 when any case fails. Every case works in a mktemp
# directory under TMPDIR and never writes into the bundle or the worktree. Fixtures: fixtures/.
#   version            `exiftool -ver` prints EXIFTOOL_VERSION from versions.env
#   exif_round_trip    Make, Model and DateTimeOriginal written to a copy of tiny.tif, read back
#                      with -csv
#   xmp_round_trip     the TRD-10 XMP assignments written to a copy of tiny.mov, read back with
#                      -XMP:all
#   checksum_mismatch  fetch.sh refuses a tampered archive (non-zero, "checksum mismatch", nothing
#                      written under the destination); the untampered control gets past the
#                      checksum step and extracts the bundle
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fixtures="$here/fixtures"
versions="$here/versions.env"

if [ "$#" -ne 1 ]; then
  echo "usage: smoke_test.sh <bundle dir>" >&2
  exit 2
fi
if [ ! -d "$1" ]; then
  echo "FAIL bundle: no directory $1"
  exit 1
fi
bundle="$(cd "$1" && pwd)"

[ -f "$versions" ] || { echo "FAIL bundle: no versions file at $versions"; exit 1; }
# shellcheck source=/dev/null
. "$versions"

if [ -f "$bundle/exiftool.exe" ]; then
  run_tool() { "$bundle/exiftool.exe" "$@"; }
elif [ -f "$bundle/exiftool" ]; then
  perl_bin=perl
  if [ -x /usr/bin/perl ]; then perl_bin=/usr/bin/perl; fi
  run_tool() { "$perl_bin" "$bundle/exiftool" "$@"; }
else
  echo "FAIL bundle: no exiftool or exiftool.exe in $bundle"
  exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/exiftool-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT

failures=0
pass() { echo "PASS $1"; }
fail() {
  echo "FAIL $1"
  failures=$((failures + 1))
  if [ -n "${2:-}" ]; then printf '%s\n' "$2" | sed 's/^/     /'; fi
}

# contains <text> <needle>: true when the text holds the needle.
contains() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

# value_of <output> <tag>: the value of the `-s` line for <tag>, or nothing.
value_of() { printf '%s\n' "$1" | tr -d '\r' | sed -n "s/^$2[[:space:]]*: //p" | sed -n 1p; }

# sha256_of <file>
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    /usr/bin/shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

case_version() {
  local out rc
  out="$(run_tool -ver 2>&1)"; rc=$?
  out="$(printf '%s' "$out" | tr -d '\r' | sed 's/[[:space:]]*$//')"
  if [ "$rc" -eq 0 ] && [ -n "${EXIFTOOL_VERSION:-}" ] && [ "$out" = "$EXIFTOOL_VERSION" ]; then
    pass version
  else
    fail version "rc=$rc got '$out', want '${EXIFTOOL_VERSION:-}'"
  fi
}

case_exif_round_trip() {
  local t="$work/exif" out
  mkdir -p "$t"
  cp "$fixtures/tiny.tif" "$t/tiny.tif"
  if ! out="$(run_tool -q -overwrite_original -Make=TRDSmoke -Model=TinyCam \
      '-DateTimeOriginal=2026:01:02 03:04:05' "$t/tiny.tif" 2>&1)"; then
    fail exif_round_trip "write failed: $out"
    return
  fi
  out="$(run_tool -csv -Make -Model -DateTimeOriginal "$t/tiny.tif" 2>&1)"
  if contains "$out" "SourceFile,Make,Model,DateTimeOriginal" \
     && contains "$out" ",TRDSmoke,TinyCam,2026:01:02 03:04:05"; then
    pass exif_round_trip
  else
    fail exif_round_trip "$out"
  fi
}

case_xmp_round_trip() {
  local t="$work/xmp" out v
  mkdir -p "$t"
  cp "$fixtures/tiny.mov" "$t/tiny.mov"
  if ! out="$(run_tool -q -overwrite_original \
      -XMP-tiff:Make=TRDSmoke -XMP-tiff:Model=TinyCam -XMP-aux:Lens=TinyLens \
      -XMP-exif:ISO=400 -XMP-exif:FNumber=2.8 -XMP-exif:FocalLength=35 \
      '-XMP-exif:DateTimeOriginal=2026:01:02 03:04:05' "$t/tiny.mov" 2>&1)"; then
    fail xmp_round_trip "write failed: $out"
    return
  fi
  out="$(run_tool -n -s -XMP:all "$t/tiny.mov" 2>&1)"
  local bad=""
  v="$(value_of "$out" Make)";     [ "$v" = TRDSmoke ] || bad="$bad Make='$v'"
  v="$(value_of "$out" Model)";    [ "$v" = TinyCam ] || bad="$bad Model='$v'"
  v="$(value_of "$out" Lens)";     [ "$v" = TinyLens ] || bad="$bad Lens='$v'"
  v="$(value_of "$out" ISO)";      [ "$v" = 400 ] || bad="$bad ISO='$v'"
  v="$(value_of "$out" FNumber)";  [ "$v" = 2.8 ] || bad="$bad FNumber='$v'"
  v="$(value_of "$out" FocalLength)"; [ "$v" = 35 ] || bad="$bad FocalLength='$v'"
  # ExifTool may store the XMP date as 2026-01-02T03:04:05 or keep the EXIF form; accept either.
  v="$(value_of "$out" DateTimeOriginal)"
  case "$v" in *2026?01?02?03:04:05*) ;; *) bad="$bad DateTimeOriginal='$v'" ;; esac
  if [ -z "$bad" ]; then
    pass xmp_round_trip
  else
    fail xmp_round_trip "not read back:$bad"$'\n'"$out"
  fi
}

case_checksum_mismatch() {
  local t="$work/checksum" sha size off orig byte
  local rc_bad rc_ok bad_log ok_log
  mkdir -p "$t/payload/exiftool-smoke/lib"
  printf '#!/bin/sh\necho smoke\n' > "$t/payload/exiftool-smoke/exiftool"
  printf '1;\n' > "$t/payload/exiftool-smoke/lib/Smoke.pm"
  printf 'smoke licence\n' > "$t/payload/exiftool-smoke/LICENSE"
  tar -czf "$t/bundle.tar.gz" -C "$t/payload" exiftool-smoke
  sha="$(sha256_of "$t/bundle.tar.gz")"
  cat > "$t/versions.env" <<EOF
EXIFTOOL_VERSION=smoke
EXIFTOOL_UNIX_URL=file:///not-used
EXIFTOOL_UNIX_SHA256=$sha
EOF

  # A tampered copy: one byte in the middle of the archive changes.
  cp "$t/bundle.tar.gz" "$t/tampered.tar.gz"
  size="$(wc -c < "$t/tampered.tar.gz" | tr -d ' ')"
  off=$((size / 2))
  orig="$(dd if="$t/tampered.tar.gz" bs=1 skip="$off" count=1 2>/dev/null | od -An -tu1 | tr -d ' ')"
  if [ "$orig" = 65 ]; then byte=B; else byte=A; fi
  printf '%s' "$byte" | dd of="$t/tampered.tar.gz" bs=1 seek="$off" count=1 conv=notrunc 2>/dev/null

  EXIFTOOL_VERSIONS_FILE="$t/versions.env" EXIFTOOL_LOCAL_ARCHIVE="$t/tampered.tar.gz" \
    EXIFTOOL_WORK_DIR="$t/work-bad" bash "$here/fetch.sh" unix "$t/dest-bad" > "$t/bad.log" 2>&1
  rc_bad=$?
  bad_log="$(cat "$t/bad.log")"

  # The untampered control: the same archive must get past the checksum step and extract.
  EXIFTOOL_VERSIONS_FILE="$t/versions.env" EXIFTOOL_LOCAL_ARCHIVE="$t/bundle.tar.gz" \
    EXIFTOOL_WORK_DIR="$t/work-ok" bash "$here/fetch.sh" unix "$t/dest-ok" > "$t/ok.log" 2>&1
  rc_ok=$?
  ok_log="$(cat "$t/ok.log")"

  local bad=""
  [ "$rc_bad" -ne 0 ] || bad="$bad tampered-exit-was-zero;"
  contains "$bad_log" "checksum mismatch" || bad="$bad no-checksum-mismatch-message;"
  [ ! -e "$t/dest-bad" ] || bad="$bad tampered-wrote-destination;"
  if contains "$ok_log" "checksum mismatch"; then bad="$bad control-failed-checksum;"; fi
  [ "$rc_ok" -eq 0 ] || bad="$bad control-exit=$rc_ok;"
  [ -f "$t/dest-ok/exiftool" ] && [ -d "$t/dest-ok/lib" ] && [ -f "$t/dest-ok/LICENSE-exiftool.txt" ] \
    || bad="$bad control-bundle-incomplete;"

  if [ -z "$bad" ]; then
    pass checksum_mismatch
  else
    fail checksum_mismatch "$bad"$'\n'"tampered: rc=$rc_bad $bad_log"$'\n'"control: rc=$rc_ok $ok_log"
  fi
}

case_version
case_exif_round_trip
case_xmp_round_trip
case_checksum_mismatch

if [ "$failures" -ne 0 ]; then
  exit 1
fi
exit 0
