# FFmpeg for trd

trd renders and reads video through `ffmpeg` and `ffprobe`. This directory builds the pair it
ships: static, GPL, with hardware acceleration compiled in, for macOS (arm64 + x86_64 in one
universal binary), Linux and Windows.

They are **built, not committed and not downloaded**. `build.sh` produces them into
`dist/<target>/`; `.github/workflows/v3-ffmpeg.yml` builds and smoke-tests every platform in CI.

## Provenance

This is a port of the ffmpeg build recipe from the star repository
(`brianm998/starry_timelapse_airplane_remover`), taken at commit
`ae7ae6f2c2687c3c892832ce0c241a4e41d8f0d1`: its `ffmpeg/build.sh`, `lib/`, `versions.env` and
`smoke_test.sh`, and its `.github/workflows/ffmpeg-gpu.yml`. Left out of the port are star's
table-generation targets and its licence-variant build, which trd does not use, along with the
old build script and the NVENC table checks that only star's Swift code needs. The version suffix
is `--extra-version=trd`, and `BUILDINFO.txt` points at this repository.

Differences from star's `build.sh` that a reader might trip over:

* With no target, `build.sh` builds this machine's native slice (`macos-arm64` or `macos-x64` on a
  Mac, where star's default was the universal binary). Ask for `macos-universal` explicitly for the
  shipping build; the Makefile and the mason suite both use the native slice.
* `--install` copies into `dist/<target>`, the one place the smoke test and CI read from, instead of
  into a consuming application's tree.

## What is in them

FFmpeg **9.0.2** (pinned in `versions.env`), `--enable-gpl`, linked fully statically (macOS: system
frameworks only; Windows: nothing but Windows; Linux: glibc only).

| | |
|---|---|
| H.264 | x264 (8- and 10-bit) |
| HEVC | x265 (8-, 10- and 12-bit) |
| AV1 | SVT-AV1 (encode), dav1d (decode: FFmpeg's own AV1 decoder only works through hardware) |
| VP8 / VP9 | libvpx |
| ProRes, DNxHD/HR, FFV1, MJPEG, PNG/TIFF/EXR, ... | FFmpeg's own |

Left out on purpose: audio libraries, subtitle and filter libraries, the capture devices, and
`libfdk-aac` (non-free: it would make the binary unredistributable).

## Hardware

Every GPU API is loaded **at run time** (`dlopen` / `LoadLibrary`), so the same binary runs on a
machine with the hardware and one without; building needs only the vendors' headers. The header
versions are pinned for the oldest drivers that work (NVENC: driver 530.41.03 / 531.61).

| | macOS | Windows x64 | Linux |
|---|---|---|---|
| Encode | VideoToolbox: H.264, HEVC, ProRes | NVENC (NVIDIA) and AMF (AMD): H.264, HEVC, AV1 | NVENC: H.264, HEVC, AV1 |
| Built in, not driven by trd yet | – | Media Foundation encoders, Direct3D 11/12 and DXVA2 decode, Vulkan | Vulkan |

Hardware decode (`-hwaccel`) is not bit-identical to the software decoders, so trd decodes in
software unless asked otherwise.

Left out on purpose:

* **VAAPI** (Linux Intel/AMD): FFmpeg links libva rather than loading it, so a binary that has it will
  not start on a machine without libva, and a statically linked libva cannot find its drivers.
* **Intel Quick Sync / `libvpl`**: Media Foundation reaches Intel's encoder on Windows already.
* **Metal filters**: need Apple's separately downloaded Metal toolchain, and add only filters trd does not use.
* **Windows on Arm** runs the x64 build under emulation; a native one needs separate toolchain work.

## Building

```bash
./v3/ffmpeg/build.sh                    # this machine's native target (one slice on a Mac)
./v3/ffmpeg/build.sh macos-universal    # the shipping macOS binary (both slices, joined)
./v3/ffmpeg/build.sh --install          # ... and make sure it is in dist/<target>
./v3/ffmpeg/build.sh linux-x64          # also linux-arm64, windows-x64, macos-arm64, macos-x64
./v3/ffmpeg/smoke_test.sh --target macos-universal v3/ffmpeg/dist/macos-universal
```

Results go to `v3/ffmpeg/dist/<target>/` (`ffmpeg`, `ffprobe`, `LICENSE-ffmpeg.txt`, `BUILDINFO.txt`);
sources and build trees go to `v3/ffmpeg/.work/` (both git-ignored). Steps are stamped, so a re-run
only rebuilds what changed. `make -C v3/ffmpeg run` builds the native target and smoke-tests it.

Prerequisites: a C/C++ compiler, `git`, `curl`, `make`, `cmake`, `meson`, `ninja`, `pkg-config`, and `nasm` for
x86_64 targets (without it the codecs fall back to C and encode several times slower).

| | |
|---|---|
| macOS | Xcode command line tools, `brew install nasm pkgconf meson ninja cmake`. An arm64 Mac cross-compiles the x86_64 slice. |
| Linux | `sudo apt-get install build-essential nasm pkg-config cmake meson ninja-build git curl xz-utils zlib1g-dev` |
| Windows | an MSYS2 **MINGW64** shell with `base-devel git curl xz diffutils file` and the `mingw-w64-x86_64-` `toolchain nasm cmake ninja meson pkgconf` packages |

## The smoke test

`smoke_test.sh [--target T] [--hw] DIR` prints `PASS`, `FAIL` and `skip` lines and exits non-zero on
any failure. It checks that both tools start and report the `-trd` suffix, that `BUILDINFO.txt`
beside them names FFmpeg 9.0.2 and this repository, the architecture (with `--target`), linkage,
the encoders, decoders, muxers and hardware accelerators trd needs, and then renders the way trd
does (`1-trd-core-daemon.md` §6.5 and §6.16): 48 `LRT_%05d.tif` frames at 600×400, ProResUHQ at the
original size, ProResMQ at 1080×720, h264 at width 720 and libx265 at the original size. Each output
must have 48 frames, the expected size, SAR 1:1, DAR 3:2, the expected `pix_fmt`, `color_range=pc`,
`color_primaries=bt709`, a `tmcd` stream and a `-progress` file ending in `progress=end`; each must
decode to the end, and each yields a 480-wide JPEG thumbnail.

Hardware encoders are reported, not failed, unless `--hw` is given: a skip means the build has the
encoder but this machine has no device for it.

## CI

`.github/workflows/v3-ffmpeg.yml` builds and smoke-tests `macos-universal` (macos-15, with the
x86_64 slice also smoke-tested under Rosetta, not fatal), `linux-x64`, `linux-arm64` and
`windows-x64`, caches each target on a hash of `versions.env`, `build.sh` and `lib/**`, and uploads
`ffmpeg`, `ffprobe`, `LICENSE-ffmpeg.txt` and `BUILDINFO.txt` as artifacts. Build logs are uploaded on
failure.

## Changing it

* **A new FFmpeg or codec library**: edit `versions.env` (a checksum, or for x264/SVT-AV1/libvpx a git
  commit). The cache key changes with it, so CI rebuilds.
* **A new codec**: add the dependency recipe in `lib/deps.sh`, enable it in `ffmpeg_common_args` in
  `lib/targets.sh`, and add it to the checks in `smoke_test.sh`.
* **A GPU API**: `lib/targets.sh` (flags) and `versions.env` (headers).
