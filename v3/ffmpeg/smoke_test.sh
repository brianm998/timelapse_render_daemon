#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Smoke test for a built ffmpeg/ffprobe pair, run by v3/ffmpeg/build.sh's callers (CI, the
# Makefile and the mason suite): does it run, was it built from the recipe it names, does it have
# what trd needs, does it link only against what every machine of that OS has, does it render the
# way trd drives it (1-trd-core-daemon.md §6.5 and §6.16), and do the hardware paths work where
# there is hardware.
#
#   smoke_test.sh [--target <target>] [--hw] <dir containing ffmpeg, ffprobe and BUILDINFO.txt>
#
#   --target T   also check the binaries really are built for T (architecture, linkage rules)
#   --hw         a hardware encoder that is built in but fails to start is a failure rather than
#                a skip.  Use it on a machine known to have the hardware.
#
# Exits non-zero if anything failed.  Bash 3.2 compatible (macOS /bin/bash) and runs under Git
# Bash on Windows.
#

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=versions.env
. "$HERE/versions.env"

TARGET=""
REQUIRE_HW=0
DIR=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --target) TARGET="$2"; shift 2 ;;
        --hw)     REQUIRE_HW=1; shift ;;
        -h|--help) sed -n '5,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*)       echo "unknown option: $1" >&2; exit 2 ;;
        *)        DIR="$1"; shift ;;
    esac
done
[ -n "$DIR" ] || { echo "usage: $0 [--target T] [--hw] DIR" >&2; exit 2; }
# the tests below run from a scratch directory, so the binaries need an absolute path
DIR="$(cd "$DIR" && pwd)" || { echo "no such directory: $DIR" >&2; exit 2; }

case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) EXE=.exe; HOST_OS=windows ;; Darwin) EXE=""; HOST_OS=macos ;; *) EXE=""; HOST_OS=linux ;; esac
[ -n "$TARGET" ] && TARGET_OS="${TARGET%%-*}" || TARGET_OS="$HOST_OS"

FFMPEG="$DIR/ffmpeg$EXE"
FFPROBE="$DIR/ffprobe$EXE"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/trd-ffmpeg-smoke.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASSED=0; FAILED=0; SKIPPED=0
pass() { PASSED=$((PASSED + 1));   printf '  PASS  %s\n' "$1"; }
fail() { FAILED=$((FAILED + 1));   printf '  FAIL  %s -- %s\n' "$1" "$2"; }
skip() { SKIPPED=$((SKIPPED + 1)); printf '  skip  %s -- %s\n' "$1" "$2"; }

# check <name> <command...>: pass if it exits 0, otherwise fail with its last output line.
check() {
    local name=$1 out
    shift
    if out=$("$@" 2>&1); then pass "$name"; else fail "$name" "$(printf '%s' "$out" | tail -n 1)"; fi
}

# expect <name> <want> <have>: pass if the two are equal.
expect() {
    if [ "$3" = "$2" ]; then pass "$1"; else fail "$1" "want '$2', have '${3:-nothing}'"; fi
}

# probe <file> <stream selector> <entry>: one value from ffprobe (the first line of it).
probe() {
    "$FFPROBE" -v error -count_frames -select_streams "$2" -show_entries "stream=$3" \
        -of default=noprint_wrappers=1:nokey=1 "$1" 2>/dev/null | head -n 1
}

ff() { "$FFMPEG" -hide_banner -loglevel error -y "$@"; }

echo "== $FFMPEG"
[ -x "$FFMPEG" ]  || { echo "  FAIL  $FFMPEG is missing or not executable"; exit 1; }
[ -x "$FFPROBE" ] || { echo "  FAIL  $FFPROBE is missing or not executable"; exit 1; }

# ── it runs ──────────────────────────────────────────────────────────────────
echo "-- runs"
for tool in "$FFMPEG" "$FFPROBE"; do
    if banner=$("$tool" -hide_banner -version 2>&1 | head -n 1) && case "$banner" in *-trd*) true ;; *) false ;; esac; then
        pass "${tool##*/} starts: $banner"
    else
        fail "${tool##*/} starts" "${banner:-no output}"
    fi
done

# ── build info (BUILDINFO.txt beside the binaries) ───────────────────────────
echo "-- build info"
info="$DIR/BUILDINFO.txt"
check "BUILDINFO.txt names FFmpeg $FFMPEG_VERSION" grep -q -F "FFmpeg $FFMPEG_VERSION" "$info"
check "BUILDINFO.txt names the recipe repository $RECIPE_URL" grep -q -F "$RECIPE_URL" "$info"
banner=$("$FFMPEG" -hide_banner -version 2>&1 | head -n 1)
case "$banner" in
    "ffmpeg version $FFMPEG_VERSION-trd"*) pass "ffmpeg -version shows the trd suffix: $banner" ;;
    *) fail "ffmpeg -version shows the trd suffix" "want 'ffmpeg version $FFMPEG_VERSION-trd', have '${banner:-no output}'" ;;
esac

# ── built for the right machine ──────────────────────────────────────────────
if [ -n "$TARGET" ]; then
    echo "-- architecture ($TARGET)"
    case "$TARGET" in
        macos-arm64)     want="arm64" ;;
        macos-x64)       want="x86_64" ;;
        macos-universal) want="x86_64 arm64" ;;
        *)               want="" ;;
    esac
    for tool in "$FFMPEG" "$FFPROBE"; do
        if [ "$TARGET_OS" = macos ]; then
            have=$(/usr/bin/lipo -archs "$tool" 2>&1)
            [ "$have" = "$want" ] && pass "${tool##*/} is $have" || fail "${tool##*/} architectures" "want '$want', have '$have'"
        else
            desc=$(file -b "$tool")
            case "$TARGET" in
                *-x64)   pat='x86-64|x86_64|AMD64' ;;
                *-arm64) pat='aarch64|ARM64|arm64' ;;
            esac
            if printf '%s' "$desc" | grep -q -i -E "$pat"; then pass "${tool##*/} is $TARGET"; else fail "${tool##*/} architecture" "file says: $desc"; fi
        fi
    done
fi

# ── links only against what every machine has ────────────────────────────────
echo "-- linkage"
for tool in "$FFMPEG" "$FFPROBE"; do
    case "$TARGET_OS" in
        macos)
            # library lines start with a tab; a universal binary has a header line per slice too
            bad=$(otool -L "$tool" | awk '/^\t/ {print $1}' | grep -v -E '^(/usr/lib/|/System/Library/)' || true)
            ;;
        linux)
            if [ "$HOST_OS" = linux ]; then
                bad=$(ldd "$tool" 2>&1 | awk '{print $1}' | grep -v -E '^(linux-vdso|linux-gate|/lib.*/ld-linux|libc\.so|libm\.so|libmvec\.so|libdl\.so|libpthread\.so|librt\.so|libstdc\+\+\.so|libgcc_s\.so|statically)' || true)
            else
                bad=""
            fi
            ;;
        windows)
            bad=$(objdump -p "$tool" 2>/dev/null | sed -n 's/^.*DLL Name: *//p' | grep -i -E '^(lib|msys|cyg)' || true)
            ;;
    esac
    if [ -z "$bad" ]; then pass "${tool##*/} links only system libraries"; else fail "${tool##*/} linkage" "unexpected: $(printf '%s' "$bad" | tr '\n' ' ')"; fi
done

# ── it has what trd needs ────────────────────────────────────────────────────
echo "-- components"
ENCODERS=$("$FFMPEG" -hide_banner -encoders 2>/dev/null)
DECODERS=$("$FFMPEG" -hide_banner -decoders 2>/dev/null)
MUXERS=$("$FFMPEG" -hide_banner -muxers 2>/dev/null)
HWACCELS=$("$FFMPEG" -hide_banner -hwaccels 2>/dev/null)
has() { printf '%s\n' "$1" | grep -q -E "^ [A-Z.]{6} $2( |$)"; }
hasmux() { printf '%s\n' "$1" | grep -q -E "^ +E +$2( |$)"; }
for enc in libx264 libx265 libsvtav1 libvpx-vp9 prores_ks dnxhd ffv1; do
    has "$ENCODERS" "$enc" && pass "encoder $enc" || fail "encoder $enc" "not built in"
done
for dec in h264 hevc prores dnxhd vp9 libdav1d; do
    has "$DECODERS" "$dec" && pass "decoder $dec" || fail "decoder $dec" "not built in"
done
for mux in mov mp4 matroska image2; do
    hasmux "$MUXERS" "$mux" && pass "muxer $mux" || fail "muxer $mux" "not built in"
done

# Hardware: what this OS's build is supposed to carry (see lib/targets.sh).  Quick Sync (libvpl)
# and VAAPI are left out on purpose, so they are not expected.
case "$TARGET_OS" in
    macos)   want_hwaccels="videotoolbox"
             want_hwenc="h264_videotoolbox hevc_videotoolbox prores_videotoolbox" ;;
    linux)   want_hwaccels="cuda vulkan"
             want_hwenc="h264_nvenc hevc_nvenc av1_nvenc" ;;
    windows) want_hwaccels="cuda d3d11va d3d12va dxva2 vulkan"
             want_hwenc="h264_nvenc hevc_nvenc av1_nvenc h264_amf hevc_amf av1_amf h264_mf hevc_mf" ;;
esac
for h in $want_hwaccels; do
    printf '%s\n' "$HWACCELS" | grep -q -x "$h" && pass "hwaccel $h" || fail "hwaccel $h" "not built in"
done
for e in $want_hwenc; do
    has "$ENCODERS" "$e" && pass "hardware encoder $e is built in" || fail "hardware encoder $e" "not built in"
done

# ── trd's renders (spec §6.5 argument vectors, §6.16 thumbnail) ──────────────
echo "-- renders the way trd drives them (48 frames, 600x400, 30 fps)"
cd "$WORK" || exit 1
mkdir frames

# the export step's input: 48 rgb48le TIFFs named LRT_%05d.tif, numbered from 5 as trd numbers
# a sequence whose first index is 5 or more (so the -start_number argument is exercised)
check "make 48 rgb48le TIFFs (LRT_%05d.tif from 5)" ff -f lavfi -i "testsrc2=size=600x400:rate=30" \
    -frames:v 48 -pix_fmt rgb48le -start_number 5 frames/LRT_%05d.tif
n=$(ls frames 2>/dev/null | wc -l | tr -d ' ')
[ "$n" = 48 ] && pass "48 TIFF frames written" || fail "TIFF frames" "got $n, want 48"

# trd_render <name> <size> <codec arguments...>: the §6.5 vector for one variant.  <size> is
# "original" (the source size, 600x400) or "WxH" (the scale filter trd adds for a resolution).
# The metadata and the .partial name are left out: neither changes what is checked.
trd_render() {
    local name=$1 size=$2
    shift 2
    local out="$WORK/out_$name.mov" progress="$WORK/progress_$name.txt"
    local filter=()
    if [ "$size" != original ]; then
        filter=(-filter_complex "crop=floor(iw/2)*2:floor(ih/2)*2,scale=${size%x*}:${size#*x}:in_range=full:out_range=full,setsar=1/1")
    fi
    check "render $name" "$FFMPEG" -nostdin -hide_banner -nostats -y \
        -progress "$progress" -stats_period 0.5 \
        -r 30 -start_number 5 -i frames/LRT_%05d.tif \
        -aspect 3:2 ${filter[@]+"${filter[@]}"} \
        "$@" -movflags +write_colr -an \
        -color_range pc -color_primaries bt709 -colorspace bt709 -color_trc bt709 \
        -timecode 00:00:00:00 "$out"
}

# check_render <name> <width> <height> <pix_fmt>: what every trd output must be
check_render() {
    local name=$1 w=$2 h=$3 pix=$4
    local f="$WORK/out_$name.mov" tags thumb half dur have_pix
    if [ ! -f "$f" ]; then
        fail "$name: output exists" "no file, so the other checks for it cannot run"
        return 0
    fi
    expect "$name: 48 frames" 48 "$(probe "$f" v:0 nb_read_frames)"
    expect "$name: ${w}x${h}" "${w}x${h}" "$(probe "$f" v:0 width)x$(probe "$f" v:0 height)"
    expect "$name: SAR 1:1" 1:1 "$(probe "$f" v:0 sample_aspect_ratio)"
    expect "$name: DAR 3:2" 3:2 "$(probe "$f" v:0 display_aspect_ratio)"
    # FFmpeg's H.264 and HEVC decoders report a full-range 4:2:0 stream as the legacy "yuvj420p";
    # the stream itself is yuv420p with color_range=pc, which the next check confirms.
    have_pix=$(probe "$f" v:0 pix_fmt)
    case "$have_pix" in yuvj*) have_pix="yuv${have_pix#yuvj}" ;; esac
    expect "$name: pix_fmt $pix" "$pix" "$have_pix"
    expect "$name: color_range pc" pc "$(probe "$f" v:0 color_range)"
    expect "$name: color_primaries bt709" bt709 "$(probe "$f" v:0 color_primaries)"

    # the timecode track has no decoder, so ffprobe names it only by its tag (codec_name=unknown)
    tags=$("$FFPROBE" -v error -show_entries stream=codec_tag_string -of default=noprint_wrappers=1:nokey=1 "$f")
    if printf '%s\n' "$tags" | grep -q -x tmcd; then pass "$name: tmcd stream"; else fail "$name: tmcd stream" "streams: $(printf '%s ' $tags)"; fi

    if grep -q -x 'progress=end' "$WORK/progress_$name.txt" 2>/dev/null; then
        pass "$name: -progress file ends with progress=end"
    else
        fail "$name: -progress file ends with progress=end" "last line: $(tail -n 1 "$WORK/progress_$name.txt" 2>/dev/null)"
    fi

    check "$name: decodes to the end (-f null)" ff -i "$f" -f null -

    # §6.16: a frame from the middle, scaled to 480 wide
    dur=$("$FFPROBE" -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 "$f" | head -n 1)
    half=$(awk -v d="$dur" 'BEGIN { printf "%.3f", d / 2 }')
    thumb="$WORK/thumb_$name.jpg"
    check "$name: JPEG thumbnail at duration/2" ff -ss "$half" -i "$f" -frames:v 1 -vf scale=480:-2 -q:v 3 "$thumb"
    expect "$name: thumbnail is mjpeg" mjpeg "$(probe "$thumb" v:0 codec_name)"
    expect "$name: thumbnail is 480 wide" 480 "$(probe "$thumb" v:0 width)"
}

# ProResUHQ at the original size; ProResMQ at 1080x720; h264 at width 720; libx265 at the original size
trd_render prores_uhq_original original -c:v prores_ks -pix_fmt yuv422p10le -threads 0 -profile:v 1
check_render prores_uhq_original 600 400 yuv422p10le
trd_render prores_mq_1080x720 1080x720 -c:v prores_ks -pix_fmt yuv444p10le -threads 0 -profile:v 1
check_render prores_mq_1080x720 1080 720 yuv444p10le
trd_render h264_w720 720x480 -c:v libx264 -pix_fmt yuv420p -threads 0 -profile:v high -crf 18
check_render h264_w720 720 480 yuv420p
trd_render hevc_original original -c:v libx265 -pix_fmt yuv420p -threads 0 -crf 18
check_render hevc_original 600 400 yuv420p

# ── hardware ─────────────────────────────────────────────────────────────────
echo "-- hardware (a skip means the build has the encoder but this machine has no device for it)"
hw_encode() { # <encoder>
    local e=$1 out
    has "$ENCODERS" "$e" || return 0
    if out=$(ff -f lavfi -i "testsrc2=size=1280x720:rate=24" -frames:v 24 -pix_fmt yuv420p -c:v "$e" -f null - 2>&1); then
        pass "hardware encode with $e"
    elif [ "$REQUIRE_HW" = 1 ]; then
        fail "hardware encode with $e" "$(printf '%s' "$out" | tail -n 1)"
    else
        skip "hardware encode with $e" "$(printf '%s' "$out" | tail -n 1)"
    fi
}
for e in $want_hwenc; do
    hw_encode "$e"
done

# hardware decode: only meaningful on the OS whose accelerator is always there
if [ "$TARGET_OS" = macos ] && [ "$HOST_OS" = macos ]; then
    if [ ! -f "$WORK/out_h264_w720.mov" ]; then
        skip "hardware decode with videotoolbox" "no H.264 file to decode"
    elif out=$(ff -hwaccel videotoolbox -i "$WORK/out_h264_w720.mov" -f null - 2>&1) \
       && ! printf '%s' "$out" | grep -q -i 'failed\|error'; then
        pass "hardware decode with videotoolbox"
    elif [ "$REQUIRE_HW" = 1 ]; then
        fail "hardware decode with videotoolbox" "$(printf '%s' "$out" | tail -n 1)"
    else
        skip "hardware decode with videotoolbox" "$(printf '%s' "$out" | tail -n 1)"
    fi
fi

echo
echo "$PASSED passed, $FAILED failed, $SKIPPED skipped"
[ "$FAILED" -eq 0 ]
