#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Builds the Windows per-user installer and .zip of spec 1 §10.3 for trd:
#   trd-<version>-windows-x64-setup.exe  the NSIS installer (trd.nsi)
#   trd-<version>-windows-x64.zip        the same files, plus INSTALL.txt at the root
#
# Usage:
#   build-installer.sh --version X.Y.Z[-pre] --trd-dir DIR --ffmpeg-dir DIR --exiftool-dir DIR
#                      [--license-file FILE] [--out DIR]
#   build-installer.sh --self-check ZIP
#
# --trd-dir holds trd.exe and trdw.exe. --ffmpeg-dir holds ffmpeg.exe, ffprobe.exe,
# LICENSE-ffmpeg.txt and BUILDINFO.txt. --exiftool-dir holds exiftool.exe, exiftool_files/ and
# LICENSE-exiftool.txt. --license-file defaults to v3/LICENSE; --out defaults to the current
# directory. Building needs makensis on PATH (NSIS; under Git Bash on Windows) and Python 3
# (python3, or python).
#
# --self-check ZIP lists the zip's members and checks them against the members §10.3 requires.
# It needs no makensis.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_license="$here/../../../LICENSE"

usage() {
  cat >&2 <<'USAGE'
usage: build-installer.sh --version X.Y.Z[-pre] --trd-dir DIR --ffmpeg-dir DIR --exiftool-dir DIR
                          [--license-file FILE] [--out DIR]
       build-installer.sh --self-check ZIP
USAGE
}

die() {
  echo "build-installer.sh: $*" >&2
  exit 1
}

usage_error() {
  echo "build-installer.sh: $*" >&2
  usage
  exit 2
}

# Python 3 from the standard library only. Git Bash may have python3 as a Store stub that
# does not run, so each candidate is tried.
find_python() {
  local candidate
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1 \
       && "$candidate" -I -c 'import sys; sys.exit(0 if sys.version_info[0] == 3 else 1)' >/dev/null 2>&1; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

# Checks the members of a zip against the §10.3 list. Prints each missing member; exits 1 when
# any is missing.
check_code='import sys, zipfile

REQUIRED = ["trd.exe", "trdw.exe", "ffmpeg.exe", "ffprobe.exe", "exiftool/exiftool.exe",
            "licenses/LICENSE", "licenses/LICENSE-ffmpeg.txt", "licenses/BUILDINFO.txt",
            "licenses/LICENSE-exiftool.txt", "INSTALL.txt"]
PREFIX = "exiftool/exiftool_files/"

path = sys.argv[1]
try:
    with zipfile.ZipFile(path) as archive:
        files = [info.filename for info in archive.infolist() if not info.is_dir()]
except (OSError, zipfile.BadZipFile) as error:
    print("build-installer.sh: cannot read " + path + ": " + str(error), file=sys.stderr)
    sys.exit(1)

missing = [name for name in REQUIRED if name not in files]
if not any(name.startswith(PREFIX) for name in files):
    missing.append(PREFIX + " (with at least one file under it)")
for name in missing:
    print("build-installer.sh: " + path + " has no member " + name, file=sys.stderr)
if missing:
    sys.exit(1)
print("build-installer.sh: " + path + " has every required member")
'

# Zips every file under a directory, with forward-slash member names relative to it.
zip_code='import os, sys, zipfile

stage, target = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
    for root, dirs, files in os.walk(stage):
        dirs.sort()
        for name in sorted(files):
            full = os.path.join(root, name)
            archive.write(full, os.path.relpath(full, stage).replace(os.sep, "/"))
'

# Converts a POSIX path to a Windows one for makensis; plain paths outside Git Bash.
win_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s' "$1"
  fi
}

require_file() {
  [ -f "$1" ] || die "$2 not found: $1"
}

version=""
trd_dir=""
ffmpeg_dir=""
exiftool_dir=""
license_file="$default_license"
out_dir="."
self_check=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || usage_error "--version needs a value"
      version="$2"; shift 2 ;;
    --trd-dir)
      [ $# -ge 2 ] || usage_error "--trd-dir needs a value"
      trd_dir="$2"; shift 2 ;;
    --ffmpeg-dir)
      [ $# -ge 2 ] || usage_error "--ffmpeg-dir needs a value"
      ffmpeg_dir="$2"; shift 2 ;;
    --exiftool-dir)
      [ $# -ge 2 ] || usage_error "--exiftool-dir needs a value"
      exiftool_dir="$2"; shift 2 ;;
    --license-file)
      [ $# -ge 2 ] || usage_error "--license-file needs a value"
      license_file="$2"; shift 2 ;;
    --out)
      [ $# -ge 2 ] || usage_error "--out needs a value"
      out_dir="$2"; shift 2 ;;
    --self-check)
      [ $# -ge 2 ] || usage_error "--self-check needs a zip"
      self_check="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      usage_error "unknown argument: $1" ;;
  esac
done

PYTHON="$(find_python)" || die "needs Python 3 (python3 or python) on PATH"

if [ -n "$self_check" ]; then
  "$PYTHON" -I -c "$check_code" "$self_check"
  exit 0
fi

# Every argument is checked before anything is staged.
version_re='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
[[ "$version" =~ $version_re ]] \
  || die "--version must look like X.Y.Z or X.Y.Z-pre, got '$version'"

[ -n "$trd_dir" ] || usage_error "--trd-dir is required"
[ -n "$ffmpeg_dir" ] || usage_error "--ffmpeg-dir is required"
[ -n "$exiftool_dir" ] || usage_error "--exiftool-dir is required"
[ -d "$trd_dir" ] || die "--trd-dir is not a directory: $trd_dir"
[ -d "$ffmpeg_dir" ] || die "--ffmpeg-dir is not a directory: $ffmpeg_dir"
[ -d "$exiftool_dir" ] || die "--exiftool-dir is not a directory: $exiftool_dir"
require_file "$trd_dir/trd.exe" "trd.exe"
require_file "$trd_dir/trdw.exe" "trdw.exe"
require_file "$ffmpeg_dir/ffmpeg.exe" "ffmpeg.exe"
require_file "$ffmpeg_dir/ffprobe.exe" "ffprobe.exe"
require_file "$ffmpeg_dir/LICENSE-ffmpeg.txt" "LICENSE-ffmpeg.txt"
require_file "$ffmpeg_dir/BUILDINFO.txt" "BUILDINFO.txt"
require_file "$exiftool_dir/exiftool.exe" "exiftool.exe"
require_file "$exiftool_dir/LICENSE-exiftool.txt" "LICENSE-exiftool.txt"
[ -d "$exiftool_dir/exiftool_files" ] || die "exiftool_files/ not found in $exiftool_dir"
require_file "$license_file" "license file"

makensis_path="$(command -v makensis 2>/dev/null)" \
  || die "makensis is not on PATH; install NSIS and put makensis on PATH to build the installer"

mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"

setup_name="trd-$version-windows-x64-setup.exe"
zip_name="trd-$version-windows-x64.zip"
numeric_version="${version%%-*}.0"

work="$(mktemp -d "${TMPDIR:-/tmp}/trd-installer.XXXXXX")"
trap 'chmod -R u+w "$work" 2>/dev/null || true; rm -rf "$work"' EXIT
stage="$work/stage"
mkdir -p "$stage/exiftool" "$stage/licenses"

# The installed tree (§10.3) plus the zip's INSTALL.txt. The installer only copies the files
# its script names, so INSTALL.txt stays out of the setup.
cp "$trd_dir/trd.exe" "$trd_dir/trdw.exe" "$stage/"
cp "$ffmpeg_dir/ffmpeg.exe" "$ffmpeg_dir/ffprobe.exe" "$stage/"
cp "$exiftool_dir/exiftool.exe" "$stage/exiftool/"
cp -R "$exiftool_dir/exiftool_files" "$stage/exiftool/"
cp "$license_file" "$stage/licenses/LICENSE"
cp "$ffmpeg_dir/LICENSE-ffmpeg.txt" "$ffmpeg_dir/BUILDINFO.txt" "$stage/licenses/"
cp "$exiftool_dir/LICENSE-exiftool.txt" "$stage/licenses/"

cat > "$stage/INSTALL.txt" <<EOF
Timelapse Render Daemon $version for Windows (x64)

Install
  Run trd-$version-windows-x64-setup.exe. It installs for the current user only and
  needs no administrator rights, into %LOCALAPPDATA%\Programs\TimelapseRenderDaemon.
  The installer registers a scheduled task named TimelapseRenderDaemon that starts
  trdw.exe when you log on, and starts it now.

  To install from this zip instead, unzip it into that directory and run:
    trd.exe service install

Uninstall
  Use Apps in Windows Settings, or run uninstall.exe in the install directory. It stops
  and removes the scheduled task, then deletes the install directory.

Files
  trd.exe, trdw.exe      the daemon (trdw.exe is the windowless build the task starts)
  ffmpeg.exe, ffprobe.exe  the encoder and probe used for renders
  exiftool\exiftool.exe  metadata writer, with exiftool_files\ beside it
  licenses\              the licences of trd, ffmpeg (with its build information) and exiftool

The binaries are not code-signed. Windows SmartScreen may warn when you first run the
installer; choose "More info" and then "Run anyway" if you downloaded it from the release page.
EOF

makensis_args=(
  "-V2"
  "/DVERSION=$version"
  "/DNUMERIC_VERSION=$numeric_version"
  "/DSTAGE_DIR=$(win_path "$stage")"
  "/DOUT_FILE=$(win_path "$out_dir/$setup_name")"
  "$(win_path "$here/trd.nsi")"
)
# Git Bash would rewrite the /D arguments as paths unless path conversion is off.
MSYS_NO_PATHCONV=1 "$makensis_path" "${makensis_args[@]}" \
  || die "makensis failed"

[ -f "$out_dir/$setup_name" ] || die "makensis did not write $out_dir/$setup_name"

"$PYTHON" -I -c "$zip_code" "$stage" "$out_dir/$zip_name"
check_zip_status=0
"$PYTHON" -I -c "$check_code" "$out_dir/$zip_name" || check_zip_status=$?
[ "$check_zip_status" -eq 0 ] || die "the built zip is missing required members"

echo "wrote $out_dir/$setup_name"
echo "wrote $out_dir/$zip_name"
