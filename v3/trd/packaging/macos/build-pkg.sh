#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Builds the macOS installer of the Timelapse Render Daemon (spec 1 §10.1, TRD-24): the app bundle
# "Timelapse Render Daemon.app" (trd, ffmpeg, ffprobe, exiftool, licences, icon), the LaunchAgent that
# starts trd, and the .pkg that installs both. Bash 3.2 compatible.
#
# Usage:
#   build-pkg.sh --version V --ffmpeg-dir DIR --exiftool-dir DIR [--trd-bin FILE] [--out DIR]
#                [--sign-app IDENTITY] [--sign-installer IDENTITY] [--notarize] [--require-universal]
#   build-pkg.sh --self-check
#
#   --version V                 MAJOR.MINOR.PATCH with an optional -PRERELEASE suffix
#   --trd-bin FILE              trd to package; without it trd is built for both Apple targets and joined
#   --ffmpeg-dir DIR            ffmpeg, ffprobe, LICENSE-ffmpeg.txt, BUILDINFO.txt (v3/ffmpeg/dist/<target>)
#   --exiftool-dir DIR          exiftool, lib/, LICENSE-exiftool.txt (v3/exiftool/dist/unix)
#   --out DIR                   where trd-<V>-macos-universal.pkg is written (default: dist/trd)
#   --sign-app IDENTITY         Developer ID Application identity; without it the app is ad hoc signed
#   --sign-installer IDENTITY   Developer ID Installer identity; without it the .pkg is unsigned
#   --notarize                  notarize and staple the .pkg. Needs --sign-app, --sign-installer and
#                               APPLE_API_KEY_PATH, APPLE_API_KEY_ID, APPLE_API_ISSUER_ID. Without the
#                               three variables it is skipped with a message, except under
#                               GITHUB_ACTIONS, where it fails.
#   --require-universal         every Mach-O in Contents/MacOS needs x86_64 and arm64 slices
#   --self-check                assemble a bundle and an unsigned .pkg from stubs and check them
#
# The bundle is signed inside out (trd, ffmpeg, ffprobe, then the bundle) with hardened runtime and a
# timestamp when an identity is given, and ad hoc (--sign -) when not.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
trd_dir="$(cd "$script_dir/../.." && pwd)"
v3_dir="$(cd "$trd_dir/.." && pwd)"
icon_dir="$v3_dir/assets/icon"

APP_NAME="Timelapse Render Daemon.app"
APP_EXEC="/Applications/$APP_NAME/Contents/MacOS/trd"
AGENT_FILE="com.timelapserender.trd.plist"
PKG_ID="com.timelapserender.trd.pkg"

die() {
  echo "build-pkg.sh: error: $*" >&2
  exit 1
}

say() {
  echo "build-pkg.sh: $*"
}

usage() {
  sed -n '3,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

VERSION=""
TRD_BIN=""
FFMPEG_DIR=""
EXIFTOOL_DIR=""
OUT_DIR=""
SIGN_APP_ID=""
SIGN_PKG_ID=""
NOTARIZE=0
REQUIRE_UNIVERSAL=0
SELF_CHECK=0

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "$1 needs a value"; VERSION="$2"; shift 2 ;;
    --trd-bin) [ $# -ge 2 ] || die "$1 needs a value"; TRD_BIN="$2"; shift 2 ;;
    --ffmpeg-dir) [ $# -ge 2 ] || die "$1 needs a value"; FFMPEG_DIR="$2"; shift 2 ;;
    --exiftool-dir) [ $# -ge 2 ] || die "$1 needs a value"; EXIFTOOL_DIR="$2"; shift 2 ;;
    --out) [ $# -ge 2 ] || die "$1 needs a value"; OUT_DIR="$2"; shift 2 ;;
    --sign-app) [ $# -ge 2 ] || die "$1 needs a value"; SIGN_APP_ID="$2"; shift 2 ;;
    --sign-installer) [ $# -ge 2 ] || die "$1 needs a value"; SIGN_PKG_ID="$2"; shift 2 ;;
    --notarize) NOTARIZE=1; shift ;;
    --require-universal) REQUIRE_UNIVERSAL=1; shift ;;
    --self-check) SELF_CHECK=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

work="$(mktemp -d "${TMPDIR:-/tmp}/trd-pkg.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# expected_payload: the files of the package payload, relative to its root (TRD-24 A1).
expected_payload() {
  local contents="Applications/$APP_NAME/Contents"
  cat <<EOF
$contents/Info.plist
$contents/MacOS/trd
$contents/MacOS/ffmpeg
$contents/MacOS/ffprobe
$contents/Resources/exiftool/exiftool
$contents/Resources/exiftool/lib
$contents/Resources/trd.icns
$contents/Resources/licenses/LICENSE
$contents/Resources/licenses/LICENSE-ffmpeg.txt
$contents/Resources/licenses/BUILDINFO.txt
$contents/Resources/licenses/LICENSE-exiftool.txt
Library/LaunchAgents/$AGENT_FILE
EOF
}

# check_archs <file> <require-universal 0|1>: the file is a Mach-O with at least one architecture;
# with require-universal it also needs x86_64 and arm64 (arm64e counts as arm64).
check_archs() {
  local file="$1" need="$2" archs
  if ! archs="$(/usr/bin/lipo -archs "$file" 2>/dev/null)"; then
    echo "not a Mach-O file: $file" >&2
    return 1
  fi
  if [ "$need" = 1 ]; then
    case " $archs " in
      *" x86_64 "*) ;;
      *) echo "$file has no x86_64 slice ($archs)" >&2; return 1 ;;
    esac
    case " $archs " in
      *" arm64 "*|*" arm64e "*) ;;
      *) echo "$file has no arm64 slice ($archs)" >&2; return 1 ;;
    esac
  fi
  echo "$(basename "$file"): $archs"
}

# check_plist_key <plist> <key path> <expected value>: prints the mismatch and returns 1 on failure.
check_plist_key() {
  local actual
  actual="$(/usr/bin/plutil -extract "$2" raw -o - "$1" 2>/dev/null)" || actual=""
  if [ "$actual" != "$3" ]; then
    echo "$1: $2 is '$actual', want '$3'" >&2
    return 1
  fi
}

# check_bundle <app> <version> <require-universal 0|1>: the bundle's files, the architectures of its
# Mach-O files, its Info.plist keys and its signature. Returns non-zero when any check fails.
check_bundle() {
  local app="$1" version="$2" need="$3" failed=0 rel exe contents="$1/Contents"
  for rel in Info.plist Resources/trd.icns Resources/exiftool/exiftool \
             Resources/licenses/LICENSE Resources/licenses/LICENSE-ffmpeg.txt \
             Resources/licenses/BUILDINFO.txt Resources/licenses/LICENSE-exiftool.txt; do
    if [ ! -f "$contents/$rel" ]; then
      echo "missing $contents/$rel" >&2
      failed=1
    fi
  done
  [ -x "$contents/Resources/exiftool/exiftool" ] || { echo "exiftool is not executable" >&2; failed=1; }
  [ -d "$contents/Resources/exiftool/lib" ] || { echo "missing $contents/Resources/exiftool/lib" >&2; failed=1; }
  for exe in trd ffmpeg ffprobe; do
    if [ ! -x "$contents/MacOS/$exe" ]; then
      echo "missing executable $contents/MacOS/$exe" >&2
      failed=1
      continue
    fi
    check_archs "$contents/MacOS/$exe" "$need" || failed=1
  done
  check_plist_key "$contents/Info.plist" CFBundleIdentifier com.timelapserender.trd || failed=1
  check_plist_key "$contents/Info.plist" CFBundleName "Timelapse Render Daemon" || failed=1
  check_plist_key "$contents/Info.plist" CFBundleExecutable trd || failed=1
  check_plist_key "$contents/Info.plist" CFBundleIconFile trd || failed=1
  check_plist_key "$contents/Info.plist" CFBundlePackageType APPL || failed=1
  check_plist_key "$contents/Info.plist" CFBundleShortVersionString "$version" || failed=1
  check_plist_key "$contents/Info.plist" CFBundleVersion "${version%%-*}" || failed=1
  check_plist_key "$contents/Info.plist" LSUIElement true || failed=1
  check_plist_key "$contents/Info.plist" LSMinimumSystemVersion 11.0 || failed=1
  /usr/bin/codesign --verify --strict --deep "$app" || failed=1
  [ "$failed" -eq 0 ]
}

# check_agent <plist>: the LaunchAgent is valid and starts the installed trd with "daemon".
check_agent() {
  local plist="$1" failed=0
  if ! /usr/bin/plutil -lint "$plist" >/dev/null; then
    echo "invalid plist: $plist" >&2
    return 1
  fi
  check_plist_key "$plist" Label com.timelapserender.trd || failed=1
  check_plist_key "$plist" ProgramArguments.0 "$APP_EXEC" || failed=1
  check_plist_key "$plist" ProgramArguments.1 daemon || failed=1
  check_plist_key "$plist" AssociatedBundleIdentifiers com.timelapserender.trd || failed=1
  check_plist_key "$plist" LimitLoadToSessionType Aqua || failed=1
  check_plist_key "$plist" RunAtLoad true || failed=1
  check_plist_key "$plist" AbandonProcessGroup true || failed=1
  [ "$failed" -eq 0 ]
}

# sign_item <identity or -> <path>: one executable or the bundle.
sign_item() {
  if [ "$1" = - ]; then
    /usr/bin/codesign --force --sign - "$2"
  else
    /usr/bin/codesign --force --options runtime --timestamp --sign "$1" "$2"
  fi
}

# sign_bundle <app> <identity or ->: spec 1 §10.1 order. ffmpeg and ffprobe are signed first, then the
# bundle. trd is not signed on its own: codesign signs the bundle's main executable (CFBundleExecutable
# = trd) as part of signing the bundle, with the same options and identity. --verify --strict --deep in
# check_bundle would catch an unsigned trd.
sign_bundle() {
  local app="$1" identity="$2" exe
  for exe in ffmpeg ffprobe; do
    sign_item "$identity" "$app/Contents/MacOS/$exe"
  done
  sign_item "$identity" "$app"
}

# assemble_bundle <app> <trd> <ffmpeg dir> <exiftool dir> <version>: copies the parts into the bundle
# and writes Info.plist from its template.
assemble_bundle() {
  local app="$1" trd="$2" ffmpeg="$3" exiftool="$4" version="$5" contents
  contents="$app/Contents"
  mkdir -p "$contents/MacOS" "$contents/Resources/exiftool" "$contents/Resources/licenses"
  cp -p "$trd" "$contents/MacOS/trd"
  cp -p "$ffmpeg/ffmpeg" "$ffmpeg/ffprobe" "$contents/MacOS/"
  cp -p "$exiftool/exiftool" "$contents/Resources/exiftool/exiftool"
  cp -Rp "$exiftool/lib" "$contents/Resources/exiftool/lib"
  cp -p "$v3_dir/LICENSE" "$contents/Resources/licenses/LICENSE"
  cp -p "$ffmpeg/LICENSE-ffmpeg.txt" "$ffmpeg/BUILDINFO.txt" "$exiftool/LICENSE-exiftool.txt" \
    "$contents/Resources/licenses/"
  cp -p "$icon_dir/trd.icns" "$contents/Resources/trd.icns"
  sed -e "s|@SHORT_VERSION@|$version|" -e "s|@BUNDLE_VERSION@|${version%%-*}|" \
    "$script_dir/Info.plist.in" > "$contents/Info.plist"
}

# render_agent <out plist>: the LaunchAgent of spec 1 §10.1, from the TRD-21 template.
render_agent() {
  sed "s|@PROGRAM_ARGUMENTS@|<string>$APP_EXEC</string><string>daemon</string>|" \
    "$script_dir/$AGENT_FILE" > "$1"
}

# stage_payload <payload> <version> <trd> <ffmpeg dir> <exiftool dir> <identity or ->: the package
# root, with the signed app and the LaunchAgent.
stage_payload() {
  local payload="$1" version="$2" trd="$3" ffmpeg="$4" exiftool="$5" identity="$6" app
  mkdir -p "$payload/Applications" "$payload/Library/LaunchAgents"
  app="$payload/Applications/$APP_NAME"
  assemble_bundle "$app" "$trd" "$ffmpeg" "$exiftool" "$version"
  sign_bundle "$app" "$identity"
  render_agent "$payload/Library/LaunchAgents/$AGENT_FILE"
}

# make_component_plist <payload> <out plist>: pkgbuild --analyze, then BundleIsRelocatable = false for
# every bundle it finds, so Installer always installs the app to /Applications (TRD-24 A2).
make_component_plist() {
  local payload="$1" out="$2" index=0 value
  /usr/bin/pkgbuild --analyze --root "$payload" "$out" >/dev/null || die "pkgbuild --analyze failed"
  while /usr/bin/plutil -extract "$index" raw -o /dev/null "$out" >/dev/null 2>&1; do
    /usr/bin/plutil -replace "$index.BundleIsRelocatable" -bool NO "$out"
    value="$(/usr/bin/plutil -extract "$index.BundleIsRelocatable" raw -o - "$out")"
    say "component plist entry $index: BundleIsRelocatable = $value"
    [ "$value" = false ] || die "BundleIsRelocatable is '$value' in entry $index, want false"
    index=$((index + 1))
  done
  [ "$index" -gt 0 ] || die "pkgbuild --analyze found no bundle in the payload"
}

# prepare_scripts <dir>: the installer scripts, named as pkgbuild expects, executable.
prepare_scripts() {
  mkdir -p "$1"
  cp "$script_dir/preinstall" "$script_dir/postinstall" "$1/"
  chmod 755 "$1/preinstall" "$1/postinstall"
}

# build_unsigned_pkg <payload> <component plist> <scripts> <version> <out pkg>
build_unsigned_pkg() {
  /usr/bin/pkgbuild --root "$1" --component-plist "$2" --scripts "$3" \
    --identifier "$PKG_ID" --version "$4" --install-location / "$5" >/dev/null \
    || die "pkgbuild failed"
}

# check_pkg_payload <pkg>: every expected payload file is in the package (pkgutil --payload-files).
check_pkg_payload() {
  local list rel failed=0
  list="$(/usr/sbin/pkgutil --payload-files "$1")" || die "pkgutil --payload-files failed on $1"
  while IFS= read -r rel; do
    if ! printf '%s\n' "$list" | grep -Fqx "./$rel"; then
      echo "payload of $1 lacks $rel" >&2
      failed=1
    fi
  done <<EOF
$(expected_payload)
EOF
  [ "$failed" -eq 0 ] || die "the package payload is incomplete"
  say "payload of $(basename "$1") has all $(expected_payload | wc -l | tr -d ' ') expected entries"
}

# build_universal_trd <out>: trd for both Apple targets, joined with lipo.
build_universal_trd() {
  local target_dir="${CARGO_TARGET_DIR:-$trd_dir/target}" arm x86
  command -v cargo >/dev/null 2>&1 || die "cargo is needed to build trd (or pass --trd-bin)"
  (cd "$trd_dir" && cargo build --release --locked --target aarch64-apple-darwin -p trd --bin trd) \
    || die "cargo build for aarch64-apple-darwin failed"
  (cd "$trd_dir" && cargo build --release --locked --target x86_64-apple-darwin -p trd --bin trd) \
    || die "cargo build for x86_64-apple-darwin failed"
  arm="$target_dir/aarch64-apple-darwin/release/trd"
  x86="$target_dir/x86_64-apple-darwin/release/trd"
  [ -f "$arm" ] && [ -f "$x86" ] || die "cargo did not produce trd for both targets"
  /usr/bin/lipo -create "$arm" "$x86" -output "$1"
}

# notarize_pkg <pkg>: submits, waits, requires Accepted, staples. Without the API variables it is
# skipped with a message (failing under GitHub Actions instead).
notarize_pkg() {
  local pkg="$1" output rc=0 id="" status=""
  if [ -z "${APPLE_API_KEY_PATH:-}" ] || [ -z "${APPLE_API_KEY_ID:-}" ] || [ -z "${APPLE_API_ISSUER_ID:-}" ]; then
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
      die "--notarize needs APPLE_API_KEY_PATH, APPLE_API_KEY_ID and APPLE_API_ISSUER_ID"
    fi
    say "notarization skipped: APPLE_API_KEY_PATH, APPLE_API_KEY_ID and APPLE_API_ISSUER_ID are not all set"
    return 0
  fi
  [ -f "$APPLE_API_KEY_PATH" ] || die "APPLE_API_KEY_PATH is not a file"
  output="$(/usr/bin/xcrun notarytool submit "$pkg" --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" --wait 2>&1)" || rc=$?
  printf '%s\n' "$output"
  id="$(printf '%s\n' "$output" | sed -n 's/^ *id: *//p' | head -n 1)"
  status="$(printf '%s\n' "$output" | sed -n 's/^ *status: *//p' | tail -n 1)"
  if [ "$status" != Accepted ]; then
    if [ -n "$id" ]; then
      /usr/bin/xcrun notarytool log "$id" --key "$APPLE_API_KEY_PATH" \
        --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" || true
    fi
    die "notarization result is '${status:-unknown}' (notarytool exit $rc), not Accepted"
  fi
  /usr/bin/xcrun stapler staple "$pkg" || die "stapling $pkg failed"
  say "notarized and stapled: $(basename "$pkg")"
}

self_check() {
  local root="$work/self" ffmpeg exiftool payload version="3.0.0-selfcheck"
  ffmpeg="$root/ffmpeg"
  exiftool="$root/exiftool"
  payload="$root/payload"
  mkdir -p "$ffmpeg" "$exiftool/lib/Image"
  # Stand-ins: copies of a system Mach-O (universal on Apple silicon) and small text files.
  cp /usr/bin/true "$root/trd"
  cp /usr/bin/true "$ffmpeg/ffmpeg"
  cp /usr/bin/true "$ffmpeg/ffprobe"
  echo "LICENSE-ffmpeg stand-in" > "$ffmpeg/LICENSE-ffmpeg.txt"
  echo "BUILDINFO stand-in" > "$ffmpeg/BUILDINFO.txt"
  printf '#!/bin/sh\necho "exiftool stand-in"\n' > "$exiftool/exiftool"
  chmod 755 "$exiftool/exiftool"
  echo "package stand-in" > "$exiftool/lib/Image/ExifTool.pm"
  echo "LICENSE-exiftool stand-in" > "$exiftool/LICENSE-exiftool.txt"

  stage_payload "$payload" "$version" "$root/trd" "$ffmpeg" "$exiftool" -
  check_bundle "$payload/Applications/$APP_NAME" "$version" 1 || die "self-check: the bundle failed"
  check_agent "$payload/Library/LaunchAgents/$AGENT_FILE" || die "self-check: the LaunchAgent failed"
  make_component_plist "$payload" "$root/component.plist"
  prepare_scripts "$root/scripts"
  build_unsigned_pkg "$payload" "$root/component.plist" "$root/scripts" "$version" "$root/selfcheck.pkg"
  check_pkg_payload "$root/selfcheck.pkg"

  # A single-architecture file must fail the universal check.
  /usr/bin/lipo -thin x86_64 /usr/bin/true -output "$root/thin"
  if check_archs "$root/thin" 1 >/dev/null 2>&1; then
    die "self-check: a single-architecture file passed --require-universal"
  fi
  say "self-check OK"
}

if [ "$SELF_CHECK" = 1 ]; then
  self_check
  exit 0
fi

[ -n "$VERSION" ] || die "--version is required (or use --self-check)"
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' \
  || die "--version '$VERSION' is not MAJOR.MINOR.PATCH with an optional -PRERELEASE suffix"
[ -n "$FFMPEG_DIR" ] || die "--ffmpeg-dir is required (build it with make release in v3/ffmpeg)"
[ -n "$EXIFTOOL_DIR" ] || die "--exiftool-dir is required (build it with make release in v3/exiftool)"
[ -d "$FFMPEG_DIR" ] || die "--ffmpeg-dir $FFMPEG_DIR is not a directory"
[ -d "$EXIFTOOL_DIR" ] || die "--exiftool-dir $EXIFTOOL_DIR is not a directory"
for file in ffmpeg ffprobe LICENSE-ffmpeg.txt BUILDINFO.txt; do
  [ -f "$FFMPEG_DIR/$file" ] || die "$FFMPEG_DIR has no $file"
done
for file in exiftool LICENSE-exiftool.txt; do
  [ -f "$EXIFTOOL_DIR/$file" ] || die "$EXIFTOOL_DIR has no $file"
done
[ -d "$EXIFTOOL_DIR/lib" ] || die "$EXIFTOOL_DIR has no lib/ directory"
if [ "$NOTARIZE" = 1 ] && { [ -z "$SIGN_APP_ID" ] || [ -z "$SIGN_PKG_ID" ]; }; then
  die "--notarize needs both --sign-app and --sign-installer"
fi
if [ -n "$TRD_BIN" ] && [ ! -f "$TRD_BIN" ]; then
  die "--trd-bin $TRD_BIN is not a file"
fi
OUT_DIR="${OUT_DIR:-$trd_dir/dist/trd}"
pkg="$work/trd-$VERSION-macos-universal.pkg"

if [ -n "$TRD_BIN" ]; then
  trd="$TRD_BIN"
else
  say "building trd for aarch64 and x86_64"
  build_universal_trd "$work/trd"
  trd="$work/trd"
fi

stage_payload "$work/payload" "$VERSION" "$trd" "$FFMPEG_DIR" "$EXIFTOOL_DIR" "${SIGN_APP_ID:--}"
say "bundle signed with ${SIGN_APP_ID:-an ad hoc signature}"
check_bundle "$work/payload/Applications/$APP_NAME" "$VERSION" "$REQUIRE_UNIVERSAL" \
  || die "the bundle failed its checks"
check_agent "$work/payload/Library/LaunchAgents/$AGENT_FILE" || die "the LaunchAgent failed its checks"
make_component_plist "$work/payload" "$work/component.plist"
prepare_scripts "$work/scripts"
build_unsigned_pkg "$work/payload" "$work/component.plist" "$work/scripts" "$VERSION" "$work/unsigned.pkg"

if [ -n "$SIGN_PKG_ID" ]; then
  /usr/bin/productsign --sign "$SIGN_PKG_ID" "$work/unsigned.pkg" "$pkg" || die "productsign failed"
else
  mv "$work/unsigned.pkg" "$pkg"
  say "the .pkg is unsigned (no --sign-installer)"
fi

if [ "$NOTARIZE" = 1 ]; then
  notarize_pkg "$pkg"
else
  say "the .pkg is not notarized (no --notarize)"
fi

if [ -n "$SIGN_PKG_ID" ]; then
  /usr/sbin/pkgutil --check-signature "$pkg" || die "the .pkg signature does not check"
else
  /usr/sbin/pkgutil --check-signature "$pkg" || true
fi
check_pkg_payload "$pkg"

mkdir -p "$OUT_DIR"
mv "$pkg" "$OUT_DIR/"
say "wrote $OUT_DIR/$(basename "$pkg")"
