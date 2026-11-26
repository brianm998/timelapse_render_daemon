#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Builds the Linux packages of overview §10.2 for one architecture:
#   trd_<v>_<amd64|arm64>.deb           dpkg-deb package, Depends: libc6 (>= 2.39), perl
#   trd-<v>-linux-<x64|arm64>.tar.gz    the same usr/lib/trd tree, the unit, the licences, INSTALL.txt
#
#   build-deb.sh [--version <v>] --arch amd64|arm64 --trd-bin <file> --ffmpeg-dir <dir>
#                --exiftool-dir <dir> --out <dir>
#   build-deb.sh --self-check <deb>
#
# --version defaults to v3/VERSION. --ffmpeg-dir is a v3/ffmpeg/dist/<target> directory (ffmpeg,
# ffprobe, LICENSE-ffmpeg.txt, BUILDINFO.txt). --exiftool-dir is v3/exiftool/dist/unix (exiftool,
# lib/, LICENSE-exiftool.txt). Building the .deb and --self-check need dpkg-deb, so run them on
# Linux; the tarball needs GNU tar. The package's Version field drops a leading "v".
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/../../.." && pwd)}"

# The §10.2 path list that dpkg-deb --contents must show (directories are not listed).
REQUIRED_PATHS="./usr/lib/trd/trd ./usr/lib/trd/ffmpeg ./usr/lib/trd/ffprobe ./usr/lib/trd/exiftool/exiftool ./usr/bin/trd ./usr/share/doc/trd/LICENSE ./usr/share/doc/trd/LICENSE-ffmpeg.txt ./usr/share/doc/trd/BUILDINFO.txt ./usr/share/doc/trd/LICENSE-exiftool.txt ./usr/lib/systemd/user/trd.service"
# Any file under this directory may appear: exiftool's Perl modules.
EXIFTOOL_LIB_PREFIX="./usr/lib/trd/exiftool/lib/"
SYMLINK_TARGET="/usr/lib/trd/trd"
DEPENDS_EXPECTED="libc6 (>= 2.39), perl"
PACKAGE_EXPECTED="trd"

problems=0

die() { echo "build-deb.sh: $*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
usage: build-deb.sh [--version <v>] --arch amd64|arm64 --trd-bin <file> --ffmpeg-dir <dir>
                    --exiftool-dir <dir> --out <dir>
       build-deb.sh --self-check <deb>
USAGE
  exit "$1"
}

# need_value <argc> <flag>: the flag must be followed by a value.
need_value() { [ "$1" -ge 2 ] || die "$2 needs a value"; }

# report <text>: one difference found by --self-check.
report() { echo "DIFF: $*"; problems=$((problems + 1)); }

# self_check <deb>: compares the deb with §10.2. Prints each difference; exits 1 on any.
self_check() {
  local deb="$1" entries paths depends package req have_lib=0 kind path target

  [ -f "$deb" ] || die "no such file: $deb"
  command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb is needed for --self-check"

  # One line per non-directory entry: kind|path|symlink target (empty unless a symlink).
  entries="$(dpkg-deb --contents "$deb" | awk '
    {
      kind = substr($1, 1, 1)
      if (kind == "d") next
      p = $6
      for (i = 7; i <= NF; i++) p = p " " $i
      target = ""
      if (kind == "l") {
        k = index(p, " -> ")
        if (k > 0) { target = substr(p, k + 4); p = substr(p, 1, k - 1) }
      }
      print kind "|" p "|" target
    }')"
  paths="$(cut -d'|' -f2 <<<"$entries")"

  for req in $REQUIRED_PATHS; do
    grep -Fxq -- "$req" <<<"$paths" || report "missing: $req"
  done

  while IFS='|' read -r kind path target; do
    [ -n "$path" ] || continue
    case "$path" in
      "$EXIFTOOL_LIB_PREFIX"*) have_lib=1; continue ;;
    esac
    if [ "$path" = ./usr/bin/trd ]; then
      if [ "$kind" != l ] || [ "$target" != "$SYMLINK_TARGET" ]; then
        report "./usr/bin/trd is not a symlink to $SYMLINK_TARGET (type '$kind', target '$target')"
      fi
      continue
    fi
    case " $REQUIRED_PATHS " in
      *" $path "*) ;;
      *) report "unexpected: $path" ;;
    esac
  done <<<"$entries"
  [ "$have_lib" -eq 1 ] || report "missing: ${EXIFTOOL_LIB_PREFIX}* (no file under exiftool/lib/)"

  depends="$(dpkg-deb --field "$deb" Depends)"
  [ "$depends" = "$DEPENDS_EXPECTED" ] || report "Depends: expected '$DEPENDS_EXPECTED', got '$depends'"
  package="$(dpkg-deb --field "$deb" Package)"
  [ "$package" = "$PACKAGE_EXPECTED" ] || report "Package: expected '$PACKAGE_EXPECTED', got '$package'"

  if [ "$problems" -ne 0 ]; then
    echo "self-check failed: $problems difference(s) in $deb" >&2
    exit 1
  fi
  echo "self-check OK: $deb"
}

# copy_file <mode> <src> <dst>
copy_file() { cp "$2" "$3" && chmod "$1" "$3"; }

need_file() {
  [ -f "$1" ] || die "missing ${2:-file}: $1"
}

# stage_payload <root>: the usr/ tree shared by the .deb and the tarball, without the
# /usr/bin/trd link (the .deb makes it; INSTALL.txt tells the tarball user to).
stage_payload() {
  local root="$1"
  mkdir -p "$root/usr/lib/trd/exiftool" "$root/usr/share/doc/trd" "$root/usr/lib/systemd/user"
  copy_file 0755 "$TRD_BIN" "$root/usr/lib/trd/trd"
  copy_file 0755 "$FFMPEG_DIR/ffmpeg" "$root/usr/lib/trd/ffmpeg"
  copy_file 0755 "$FFMPEG_DIR/ffprobe" "$root/usr/lib/trd/ffprobe"
  copy_file 0755 "$EXIFTOOL_DIR/exiftool" "$root/usr/lib/trd/exiftool/exiftool"
  cp -R "$EXIFTOOL_DIR/lib" "$root/usr/lib/trd/exiftool/lib"
  find "$root/usr/lib/trd/exiftool/lib" -type d -exec chmod 0755 {} +
  find "$root/usr/lib/trd/exiftool/lib" -type f -exec chmod 0644 {} +
  copy_file 0644 "$V3_ROOT/LICENSE" "$root/usr/share/doc/trd/LICENSE"
  copy_file 0644 "$FFMPEG_DIR/LICENSE-ffmpeg.txt" "$root/usr/share/doc/trd/LICENSE-ffmpeg.txt"
  copy_file 0644 "$FFMPEG_DIR/BUILDINFO.txt" "$root/usr/share/doc/trd/BUILDINFO.txt"
  copy_file 0644 "$EXIFTOOL_DIR/LICENSE-exiftool.txt" "$root/usr/share/doc/trd/LICENSE-exiftool.txt"
  # The template's @EXEC_START@ is the only placeholder; the rendered unit must not keep it.
  sed 's|@EXEC_START@|/usr/lib/trd/trd daemon|' "$here/trd.service" \
    > "$root/usr/lib/systemd/user/trd.service"
  chmod 0644 "$root/usr/lib/systemd/user/trd.service"
  grep -q '@EXEC_START@' "$root/usr/lib/systemd/user/trd.service" \
    && die "trd.service still has @EXEC_START@"
  grep -qx 'ExecStart=/usr/lib/trd/trd daemon' "$root/usr/lib/systemd/user/trd.service" \
    || die "trd.service has no ExecStart=/usr/lib/trd/trd daemon"
  return 0
}

# build_deb: writes $OUT/trd_<v>_<arch>.deb.
build_deb() {
  local root="$STAGE/deb" size deb_version="${VERSION#v}"
  command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb is needed to build the .deb (run on Linux)"
  mkdir -p "$root/DEBIAN" "$root/usr/bin"
  stage_payload "$root"
  ln -s "$SYMLINK_TARGET" "$root/usr/bin/trd"
  size="$(du -sk "$root/usr" | awk '{print $1}')"
  cat > "$root/DEBIAN/control" <<CONTROL
Package: trd
Version: $deb_version
Architecture: $ARCH
Maintainer: Timelapse Render Daemon contributors
Installed-Size: $size
Depends: libc6 (>= 2.39), perl
Section: video
Priority: optional
Description: Timelapse render daemon
 Renders timelapse videos from image sequences in the background, with the ffmpeg,
 ffprobe and exiftool it needs bundled. Runs as a per-user systemd service.
CONTROL
  copy_file 0755 "$here/postinst" "$root/DEBIAN/postinst"
  copy_file 0755 "$here/prerm" "$root/DEBIAN/prerm"
  chmod 0644 "$root/DEBIAN/control"
  chmod 0755 "$root/DEBIAN"
  dpkg-deb --build --root-owner-group "$root" "$OUT/trd_${VERSION}_${ARCH}.deb"
  echo "wrote $OUT/trd_${VERSION}_${ARCH}.deb"
}

# write_install <file>: INSTALL.txt for the tarball.
write_install() {
  cat > "$1" <<INSTALL
Timelapse Render Daemon $VERSION for Linux ($ARCH)

This archive holds the files of the .deb package, for installing without the package system.
Run the steps from the directory this archive unpacked into. Steps 1 to 5 need root; step 6
runs as the user who will use trd.

  1. Copy the program and its tools:
       sudo cp -R usr/lib/trd /usr/lib/
  2. Copy the licences:
       sudo mkdir -p /usr/share/doc
       sudo cp -R usr/share/doc/trd /usr/share/doc/
  3. Make the command:
       sudo ln -sfn /usr/lib/trd/trd /usr/bin/trd
  4. Install the user unit:
       sudo install -D -m 0644 usr/lib/systemd/user/trd.service /usr/lib/systemd/user/trd.service
  5. Start trd at every login, for all users:
       sudo systemctl --global enable trd.service
  6. Start it now:
       systemctl --user start trd

Check it with: trd status
To remove: sudo systemctl --global disable trd.service, then delete /usr/lib/trd,
/usr/share/doc/trd, /usr/bin/trd and /usr/lib/systemd/user/trd.service.
INSTALL
}

# build_tarball: writes $OUT/trd-<v>-linux-<x64|arm64>.tar.gz with root-owned entries.
build_tarball() {
  local top="trd-${VERSION}-linux-${TAR_ARCH}" dir="$STAGE/tar/trd-${VERSION}-linux-${TAR_ARCH}"
  mkdir -p "$dir"
  stage_payload "$dir"
  write_install "$dir/INSTALL.txt"
  (cd "$STAGE/tar" && tar --owner=0 --group=0 --numeric-owner -czf "$OUT/$top.tar.gz" "$top")
  echo "wrote $OUT/$top.tar.gz"
}

build() {
  case "$ARCH" in
    amd64) TAR_ARCH=x64 ;;
    arm64) TAR_ARCH=arm64 ;;
    *) die "--arch must be amd64 or arm64" ;;
  esac
  [ -n "$VERSION" ] || VERSION="$(tr -d '[:space:]' < "$V3_ROOT/VERSION")"
  [ -n "$VERSION" ] || die "no version: pass --version or fill in v3/VERSION"
  [ -n "$TRD_BIN" ] || die "--trd-bin is required"
  [ -n "$FFMPEG_DIR" ] || die "--ffmpeg-dir is required"
  [ -n "$EXIFTOOL_DIR" ] || die "--exiftool-dir is required"
  [ -n "$OUT" ] || die "--out is required"

  need_file "$TRD_BIN" "trd binary"
  need_file "$FFMPEG_DIR/ffmpeg" "ffmpeg"
  need_file "$FFMPEG_DIR/ffprobe" "ffprobe"
  need_file "$FFMPEG_DIR/LICENSE-ffmpeg.txt" "LICENSE-ffmpeg.txt"
  need_file "$FFMPEG_DIR/BUILDINFO.txt" "BUILDINFO.txt"
  need_file "$EXIFTOOL_DIR/exiftool" "exiftool"
  need_file "$EXIFTOOL_DIR/LICENSE-exiftool.txt" "LICENSE-exiftool.txt"
  [ -d "$EXIFTOOL_DIR/lib" ] || die "missing exiftool lib directory: $EXIFTOOL_DIR/lib"
  need_file "$V3_ROOT/LICENSE" "licence"
  need_file "$here/trd.service" "unit template"
  need_file "$here/postinst" "postinst"
  need_file "$here/prerm" "prerm"

  mkdir -p "$OUT"
  OUT="$(cd "$OUT" && pwd)"
  STAGE="$(mktemp -d "${TMPDIR:-/tmp}/trd-linux-pkg.XXXXXX")"
  trap 'rm -rf "$STAGE"' EXIT

  build_deb
  build_tarball
}

VERSION=""
ARCH=""
TRD_BIN=""
FFMPEG_DIR=""
EXIFTOOL_DIR=""
OUT=""
CHECK_DEB=""
MODE=build

[ $# -gt 0 ] || usage 2
while [ $# -gt 0 ]; do
  case "$1" in
    --self-check) need_value $# "$1"; MODE=check; CHECK_DEB="$2"; shift 2 ;;
    --version) need_value $# "$1"; VERSION="$2"; shift 2 ;;
    --arch) need_value $# "$1"; ARCH="$2"; shift 2 ;;
    --trd-bin) need_value $# "$1"; TRD_BIN="$2"; shift 2 ;;
    --ffmpeg-dir) need_value $# "$1"; FFMPEG_DIR="$2"; shift 2 ;;
    --exiftool-dir) need_value $# "$1"; EXIFTOOL_DIR="$2"; shift 2 ;;
    --out) need_value $# "$1"; OUT="$2"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [ "$MODE" = check ]; then
  self_check "$CHECK_DEB"
else
  build
fi
