# ExifTool bundle (spec 1 §6.15, §9.3, TRD-23)

This component fetches one pinned ExifTool for Unix and one for Windows, checks each archive's
SHA-256, extracts the files `trd` looks for, smoke-tests the Unix bundle, and publishes both bundles
as CI artifacts. Nothing here installs anything system-wide.

## Pinned version and checksums

`versions.env` holds the pin:

| Variable | Meaning |
|---|---|
| `EXIFTOOL_VERSION` | The newest production release on the day of the pin (`exiftool -ver` must print it). |
| `EXIFTOOL_UNIX_URL` | `https://github.com/exiftool/exiftool/archive/refs/tags/<version>.tar.gz` (the GitHub tag archive). |
| `EXIFTOOL_UNIX_SHA256` | SHA-256 of the file downloaded from `EXIFTOOL_UNIX_URL`. |
| `EXIFTOOL_WINDOWS_URL` | `https://exiftool.org/exiftool-<version>_64.zip` (the official Windows build). |
| `EXIFTOOL_WINDOWS_SHA256` | SHA-256 of the file downloaded from `EXIFTOOL_WINDOWS_URL`. |
| `EXIFTOOL_WINDOWS_MIRROR` | `https://sourceforge.net/projects/exiftool/files/exiftool-<version>_64.zip/download`, tried only when the primary download fails. |

The checksums are taken from the downloaded files, never copied from a page. A download that does
not match its checksum is refused, whether it came from the primary URL or the mirror.

**Status:** `versions.env` is not pinned yet. Its values are empty, and `fetch.sh` exits 2 with
"not pinned yet" until they are filled in. The pin needs the network, so it is done by hand (below).

## How to bump

1. Find the newest production release on <https://exiftool.org/ver.txt> and check that the GitHub
   tag `<version>` exists.
2. Download the two files, hash them and record the hashes:

   ```sh
   curl -fsSLO "https://github.com/exiftool/exiftool/archive/refs/tags/<version>.tar.gz"
   curl -fsSLO "https://exiftool.org/exiftool-<version>_64.zip"
   sha256sum <version>.tar.gz exiftool-<version>_64.zip   # or: shasum -a 256 ...
   ```
3. Update `versions.env` with the version, both URLs, both SHA-256 values and the mirror.
4. Run `make -C v3/exiftool test` (or `make -C v3/exiftool run` for a full fetch), then
   `bash v3/scripts/suite.sh exiftool-smoke`.

## Layout `trd` expects (spec 1 §6.15)

`fetch.sh unix <dest>` writes:

```
<dest>/exiftool              the Perl script, run as <perl> <dest>/exiftool
<dest>/lib/                  ExifTool's Perl modules, found beside the script
<dest>/LICENSE-exiftool.txt  the licence text found in the archive
```

`fetch.sh windows <dest>` writes:

```
<dest>/exiftool.exe          renamed from exiftool(-k).exe
<dest>/exiftool_files/       the runtime the exe needs
<dest>/LICENSE-exiftool.txt
```

`fetch.sh` downloads into `v3/exiftool/.work/`, extracts into a fresh directory there, and replaces
old outputs in `<dest>` only after the archive has been verified and extracted. A checksum mismatch
exits 2 with `checksum mismatch` and leaves `<dest>` untouched.

Environment hooks, for tests and offline use: `EXIFTOOL_VERSIONS_FILE` (another versions file),
`EXIFTOOL_LOCAL_ARCHIVE` (use a local archive instead of downloading; its checksum is still checked)
and `EXIFTOOL_WORK_DIR` (another work directory).

## Smoke test

`smoke_test.sh <dir>` runs these cases and prints `PASS <case>` or `FAIL <case>`; any failure exits 1.
Each case works in a `mktemp` directory under `TMPDIR`.

- `version`: `-ver` equals `EXIFTOOL_VERSION`.
- `exif_round_trip`: Make, Model and DateTimeOriginal are written into a copy of `fixtures/tiny.tif`
  and read back with `-csv`.
- `xmp_round_trip`: the TRD-10 XMP assignments (`-XMP-tiff:Make`, `-XMP-tiff:Model`,
  `-XMP-aux:Lens`, `-XMP-exif:ISO`, `-XMP-exif:FNumber`, `-XMP-exif:FocalLength`,
  `-XMP-exif:DateTimeOriginal`) are written into a copy of `fixtures/tiny.mov` and read back with
  `-XMP:all`.
- `checksum_mismatch`: a tampered copy of a local archive is refused by `fetch.sh` (non-zero exit,
  `checksum mismatch`, nothing under the destination), and the untampered control gets past the
  checksum step and extracts.

On Unix the script runs through `/usr/bin/perl` when it exists, else `perl`.

## Fixtures

The fixtures were made once with ffmpeg and are committed. Regenerate them only if they must
change, with these commands (run from `v3/exiftool/`):

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=16x16:rate=1 \
  -frames:v 1 -pix_fmt rgb24 -y fixtures/tiny.tif
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=16x16:rate=16 \
  -t 1 -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le -y fixtures/tiny.mov
```

`tiny.tif` is one 16×16 RGB frame. `tiny.mov` is one second of 16×16 ProRes 422 HQ (16 frames).

## Make targets

| Target | What it does |
|---|---|
| `run`, `develop` | Fetch the bundle for this host into `dist/<host>` and smoke-test it. `<host>` is `windows` under Git Bash, else `unix`. |
| `release` | Fetch both bundles into `dist/unix` and `dist/windows`. |
| `test` | Fetch the host bundle when it is missing, then smoke-test it. |
| `check` | `bash -n` on every script. |
| `clean` | Remove `.work` and `dist`. |
| `ci` | Prints that the bundles are built by their own workflow. |

## CI

`.github/workflows/v3-exiftool.yml` (spec 1 §9.3) runs three jobs: `unix` on Ubuntu fetches, smoke-tests
and uploads `v3-exiftool-unix`; `unix-macos` downloads that artifact and smoke-tests it with
`/usr/bin/perl`; `windows` fetches, smoke-tests under Git Bash and uploads `v3-exiftool-windows`.
The bundle cache key is `v3-exiftool-<kind>-` plus the hash of `versions.env` and `fetch.sh`.

## Not in this component

Installers. The bundles are plain files that `trd` finds through §6.15.
