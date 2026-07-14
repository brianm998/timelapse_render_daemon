#!/usr/bin/env python3
"""
Inspect timelapse videos for embedded EXIF/XMP metadata and, optionally,
restore it from the original raw image sequence.

Detection
---------
A video is considered "missing metadata" when `exiftool -G1` returns no
tags in the XMP-aux, XMP-exif, or XMP-tiff groups for it.  This matches
what the Swift daemon's XMPEmbedder writes; older videos rendered by the
legacy Perl daemon never had this XMP block.

Resolution of video -> raw sequence
-----------------------------------
We don't try to parse the date/camera/N pattern out of filenames.  Instead
we build a registry of known raw-sequence base names from the .7z archives
and existing .json sidecars under the configured search paths, then for
each video pick the longest base name that is a separator-bounded prefix
of the video filename.  Variants like "-aurora", "-aurora-topaz" map to
the same base archive automatically that way.

Generation
----------
When no exif json exists for a video's base sequence, we extract every
raw frame from the .7z to a scratch dir, run exiftool over them in
parallel, fold the results into the same `{tag: {value: count}}` histogram
the Swift daemon produces, and write the json next to the .7z archive.

Embedding
---------
The XMP embed step is a direct port of XMPEmbedder.swift / XMPFieldMapper.
Same field selection, same enum-int conversions, same earliest-DateTime
behaviour, same `#`-suffix raw-value handling for ExposureProgram /
ExposureMode / WhiteBalance.

A few old sidecars written by the legacy inc/Exiftool.pm are flat
`{tag: value}` maps rather than histograms; load_sidecar_histogram
normalizes those so the embed path only ever sees one shape.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path


# --- Configuration ---------------------------------------------------------


@dataclass
class Config:
    video_paths: list[Path]
    raw_archive_paths: list[Path]
    exif_json_paths: list[Path]
    raw_types: list[str] = field(default_factory=lambda: ["ARW", "ORF"])
    video_extensions: list[str] = field(default_factory=lambda: ["mov", "mp4"])
    scratch_dir: Path | None = None
    exiftool_parallelism: int = 8

    @classmethod
    def load(cls, path: Path) -> "Config":
        with path.open() as f:
            data = json.load(f)
        return cls(
            video_paths=[Path(p).expanduser() for p in data["video_paths"]],
            raw_archive_paths=[Path(p).expanduser() for p in data["raw_archive_paths"]],
            exif_json_paths=[Path(p).expanduser() for p in data["exif_json_paths"]],
            raw_types=data.get("raw_types", ["ARW", "ORF"]),
            video_extensions=data.get("video_extensions", ["mov", "mp4"]),
            scratch_dir=Path(data["scratch_dir"]).expanduser() if data.get("scratch_dir") else None,
            exiftool_parallelism=data.get("exiftool_parallelism", 8),
        )


# --- Discovery -------------------------------------------------------------


def _safe_rglob(root: Path, pattern: str):
    """rglob that yields nothing and prints a warning if the mount disappears
    mid-walk (e.g., NFS/SMB hiccup) instead of crashing the whole script."""
    try:
        yield from root.rglob(pattern)
    except OSError as err:
        print(f"warning: scan of {root} aborted: {err}", flush=True)


def find_videos(paths: list[Path], exts: list[str]) -> list[Path]:
    ext_set = {"." + e.lower().lstrip(".") for e in exts}
    out: list[Path] = []
    for root in paths:
        if not root.is_dir():
            continue
        for p in _safe_rglob(root, "*"):
            try:
                if p.is_file() and p.suffix.lower() in ext_set:
                    out.append(p)
            except OSError:
                continue
    return sorted(out)


def find_raw_archives(paths: list[Path]) -> dict[str, Path]:
    """basename (without .7z) -> archive path. Longest-prefix-match wins later."""
    out: dict[str, Path] = {}
    for root in paths:
        if not root.is_dir():
            continue
        for p in _safe_rglob(root, "*.7z"):
            try:
                if p.is_file():
                    out[p.stem] = p
            except OSError:
                continue
    return out


def find_exif_jsons(paths: list[Path]) -> dict[str, Path]:
    """basename (without .json) -> json path."""
    out: dict[str, Path] = {}
    for root in paths:
        if not root.is_dir():
            continue
        for p in _safe_rglob(root, "*.json"):
            try:
                # Only single-component basenames count (matches Perl daemon's
                # ^([^.]+)\.json$ rule).
                if p.is_file() and "." not in p.stem:
                    out[p.stem] = p
            except OSError:
                continue
    return out


# --- Metadata detection ----------------------------------------------------


def has_xmp_metadata(video: Path) -> bool:
    """True if the video has any XMP-aux / XMP-exif / XMP-tiff tags."""
    try:
        result = subprocess.run(
            ["exiftool", "-G1", "-s", "-XMP-aux:all", "-XMP-exif:all", "-XMP-tiff:all", str(video)],
            capture_output=True, text=True, check=False,
        )
    except FileNotFoundError:
        sys.exit("exiftool not found in PATH")
    return bool(result.stdout.strip())


def xmp_summary(video: Path) -> dict[str, str]:
    """Return present XMP tags as {tag: value}, for `--verbose` reporting."""
    result = subprocess.run(
        ["exiftool", "-s", "-s", "-s",
         "-XMP-tiff:Make", "-XMP-tiff:Model", "-XMP-aux:Lens",
         "-XMP-exif:ISO", "-XMP-exif:FNumber", "-XMP-exif:ExposureTime",
         "-XMP-exif:FocalLength", "-XMP-exif:DateTimeOriginal",
         "-args", str(video)],
        capture_output=True, text=True, check=False,
    )
    out: dict[str, str] = {}
    for line in result.stdout.splitlines():
        if line.startswith("-") and "=" in line:
            k, _, v = line[1:].partition("=")
            out[k] = v
    return out


# --- Base-name resolution --------------------------------------------------


_SEP_CHARS = {"_", "-", "."}


def resolve_base_name(video_stem: str, registry: list[str]) -> str | None:
    """Longest base name from `registry` that is a separator-bounded prefix of `video_stem`."""
    candidates: list[str] = []
    for base in registry:
        if not video_stem.startswith(base):
            continue
        if len(video_stem) == len(base):
            candidates.append(base)
            continue
        nxt = video_stem[len(base)]
        if nxt in _SEP_CHARS:
            candidates.append(base)
    if not candidates:
        return None
    return max(candidates, key=len)


# --- Exif JSON generation --------------------------------------------------


class SevenZipError(Exception):
    """Raised when 7za fails.  Carries the captured stdout+stderr."""
    def __init__(self, message: str, output: str, returncode: int):
        super().__init__(message)
        self.output = output
        self.returncode = returncode


def extract_raw_frames(archive: Path, raw_types: list[str], dest: Path) -> list[Path]:
    """Extract every raw image from the .7z into `dest` (flat) and return the file list.

    Previously this did a `7za l -slt` listing, parsed every entry, and then
    invoked `7za e` with the matching paths as positional arguments.  For
    real timelapse archives the listing was ~165k lines, the extract command
    line was hundreds of KB long (close to ARG_MAX), and the whole thing was
    sensitive to any kind of stream / network hiccup.

    Switching to `-ir!<pattern>` lets 7za do the filtering internally: one
    process call, no listing step, no per-file argv blob.  Patterns are
    case-sensitive on macOS/Linux so we pass both upper- and lower-case
    variants of each raw extension.

    The archive path is `os.path.realpath`'d before being passed to 7za.
    On macOS, `/mammoth` is a symlink into `/System/Volumes/Data/mammoth`;
    going through the canonical path avoids a class of intermittent
    "No such file or directory" failures we observed on the SMB-mounted
    backing store.  Those same hiccups can still happen on the resolved
    path though, so we retry up to `max_attempts` times when 7za reports
    the file missing but Python still sees it.
    """
    dest.mkdir(parents=True, exist_ok=True)

    patterns: list[str] = []
    seen: set[str] = set()
    for ext in raw_types:
        ext_clean = ext.lstrip(".")
        for variant in {ext_clean.upper(), ext_clean.lower()}:
            if variant in seen:
                continue
            seen.add(variant)
            patterns.append(f"-ir!*.{variant}")

    archive_arg = os.path.realpath(str(archive))
    cmd = ["7za", "e", "-y", f"-o{dest}", archive_arg] + patterns

    max_attempts = 3
    for attempt in range(1, max_attempts + 1):
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode == 0:
            return sorted(p for p in dest.iterdir() if p.is_file())

        combined = ((result.stdout or "") + (result.stderr or "")).strip()
        transient_smb = (
            "No such file or directory" in combined
            and archive.exists()
            and attempt < max_attempts
        )
        if transient_smb:
            # SMB stat hiccup during 7za's "Scanning the drive" step — the
            # file exists per the kernel but the open() racing against an
            # invalidated stat cache failed.  Wipe whatever partial output
            # landed and try again with a brief delay.
            delay = 2 * attempt
            print(
                f"  7za reported \"No such file or directory\" on attempt "
                f"{attempt} but {archive} exists; retrying in {delay}s",
                flush=True,
            )
            shutil.rmtree(dest, ignore_errors=True)
            dest.mkdir(parents=True, exist_ok=True)
            time.sleep(delay)
            continue
        raise SevenZipError(
            f"7za extract failed (exit {result.returncode}) for {archive}",
            output=combined,
            returncode=result.returncode,
        )

    # Defensive: loop exited without return or raise (shouldn't happen given
    # the structure above), surface as a definite failure.
    raise SevenZipError(
        f"7za extract failed for {archive} after {max_attempts} attempt(s)",
        output="",
        returncode=-1,
    )


def exiftool_csv_one(image: Path) -> dict[str, str]:
    """`exiftool -csv FILE` -> {tag: value} for a single image."""
    result = subprocess.run(
        ["exiftool", "-csv", str(image)],
        capture_output=True, text=True, check=False,
    )
    if not result.stdout.strip():
        return {}
    reader = csv.reader(io.StringIO(result.stdout))
    rows = list(reader)
    if len(rows) < 2:
        return {}
    return dict(zip(rows[0], rows[1]))


_SKIP_KEYS = {"Thumbnail Image", "Preview Image", "Tiff Metering Image", "SourceFile"}


def build_histogram(images: list[Path], parallelism: int) -> dict[str, dict[str, int]]:
    """Fold per-image exif maps into {tag: {value: count}}.  Mirrors ExifAccumulator.accumulate."""
    totals: dict[str, dict[str, int]] = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=parallelism) as ex:
        for one in ex.map(exiftool_csv_one, images):
            for key, value in one.items():
                if key in _SKIP_KEYS:
                    continue
                if value == "":
                    continue
                totals.setdefault(key, {}).setdefault(value, 0)
                totals[key][value] += 1
    return totals


# --- XMP embedding (port of XMPEmbedder.swift) -----------------------------


# A handful of sidecars predate the histogram format.  The legacy
# inc/Exiftool.pm wrote a *flat* {tag: value} map sampled from one
# representative frame, plus a couple of bookkeeping fields.  We fold those
# into the normal {tag: {value: count}} shape so the rest of the pipeline
# never has to know the difference.
_LEGACY_BOOKKEEPING_KEYS = {"number_source_images", "MD5", "Megapixels"}

# Flat sidecars record the f-number under exiftool's composite "Aperture" tag;
# the histogram path (like XMPFieldMapper) reads "FNumber".  Same value.
_LEGACY_KEY_ALIASES = {"Aperture": "FNumber"}


def _normalize_legacy_histogram(data: dict) -> dict[str, dict[str, int]]:
    """Fold a flat {tag: value} sidecar into {tag: {value: count}} form.

    Histogram-shaped values (dicts) pass through untouched, so a sidecar that
    mixes the two shapes is tolerated even though none do today.
    """
    out: dict[str, dict[str, int]] = {}
    for key, value in data.items():
        if isinstance(value, dict):
            out[key] = value
            continue
        key = _LEGACY_KEY_ALIASES.get(key, key)
        if key in _LEGACY_BOOKKEEPING_KEYS or key in out:
            continue
        out[key] = {str(value): 1}
    return out


def load_sidecar_histogram(json_path: Path) -> dict[str, dict[str, int]] | None:
    """Read an exif sidecar and return it as a {tag: {value: count}} histogram.

    Modern sidecars (Swift daemon, current Perl ExiftoolRunner, this tool) are
    already histograms and pass straight through.  Legacy flat {tag: value}
    sidecars are normalized.  Returns None for a structurally unusable sidecar
    (e.g., not a JSON object), which the caller treats as a soft skip.
    """
    with json_path.open() as f:
        data = json.load(f)
    if not isinstance(data, dict):
        print(f"  sidecar {json_path} is not a JSON object; skipping",
              flush=True)
        return None
    if data and not all(isinstance(v, dict) for v in data.values()):
        hist = _normalize_legacy_histogram(data)
        print(f"  legacy flat-format sidecar; normalized {len(hist)} tag(s)",
              flush=True)
        return hist
    return data


def _mode(counts: dict[str, int] | None) -> str | None:
    if not counts:
        return None
    if not isinstance(counts, dict):
        # Safety net: sidecars are normalized to histograms by
        # load_sidecar_histogram before they reach here, so a bare scalar
        # should be impossible.  Skip the tag rather than crash a long batch
        # run if one ever slips through.
        return None
    return max(counts.items(), key=lambda kv: (kv[1], kv[0]))[0]
    # Note: Swift breaks ties with "key descending" -> larger key wins, which
    # `max` with the tuple does naturally.


def _earliest_key(counts: dict[str, int] | None) -> str | None:
    if not counts:
        return None
    if not isinstance(counts, dict):
        return None  # same safety net as _mode
    return min(counts.keys())


def _strip_focal_length_suffix(s: str) -> str:
    t = s.strip()
    if t.lower().endswith("mm"):
        return t[:-2].strip()
    return t


def _exposure_program_to_int(s: str) -> int:
    low = s.lower()
    if low == "manual":
        return 1
    if low in ("program ae", "normal program"):
        return 2
    if "aperture" in low:
        return 3
    if "shutter" in low:
        return 4
    if "creative" in low:
        return 5
    if "action" in low:
        return 6
    if low == "portrait":
        return 7
    if low == "landscape":
        return 8
    return 0


def _exposure_mode_to_int(s: str) -> int:
    low = s.lower()
    if low == "manual":
        return 1
    if low in ("auto bracket", "auto-bracket"):
        return 2
    return 0


def _white_balance_to_int(s: str) -> int:
    return 0 if s.lower() == "auto" else 1


def _to_iso8601(s: str) -> str:
    parts = s.split(" ", 1)
    if len(parts) != 2:
        return s
    date = parts[0].replace(":", "-")
    time = parts[1]
    if len(date.split("-")) != 3 or len(time.split(":")) != 3:
        return s
    return f"{date}T{time}"


def build_exiftool_assignments(histogram: dict[str, dict[str, int]]) -> list[str]:
    out: list[str] = []

    if (v := _mode(histogram.get("Make"))) is not None:
        out.append(f"-XMP-tiff:Make={v}")
    if (v := _mode(histogram.get("Model"))) is not None:
        out.append(f"-XMP-tiff:Model={v}")
    if (v := _mode(histogram.get("LensModel"))) is not None:
        out.append(f"-XMP-aux:Lens={v}")
    if (v := _mode(histogram.get("ISO"))) is not None:
        out.append(f"-XMP-exif:ISO={v}")
    if (v := _mode(histogram.get("FNumber"))) is not None:
        out.append(f"-XMP-exif:FNumber={v}")
    if (v := _mode(histogram.get("ExposureTime"))) is not None:
        out.append(f"-XMP-exif:ExposureTime={v}")
    if (v := _mode(histogram.get("FocalLength"))) is not None:
        out.append(f"-XMP-exif:FocalLength={_strip_focal_length_suffix(v)}")
    if (v := _mode(histogram.get("ExposureProgram"))) is not None:
        out.append(f"-XMP-exif:ExposureProgram#={_exposure_program_to_int(v)}")
    if (v := _mode(histogram.get("ExposureMode"))) is not None:
        out.append(f"-XMP-exif:ExposureMode#={_exposure_mode_to_int(v)}")
    if (v := _mode(histogram.get("WhiteBalance"))) is not None:
        out.append(f"-XMP-exif:WhiteBalance#={_white_balance_to_int(v)}")
    if (dto := _earliest_key(histogram.get("DateTimeOriginal"))) is not None:
        out.append(f"-XMP-exif:DateTimeOriginal={_to_iso8601(dto)}")

    return out


class EmbedError(Exception):
    """Raised when exiftool refuses to embed XMP.  Carries combined output."""
    def __init__(self, message: str, output: str):
        super().__init__(message)
        self.output = output


# Substrings in exiftool output that we treat as "the video file itself is
# structurally broken; needs re-render, retry won't help".  Matches the same
# classifier the Swift daemon uses in XMPEmbedder.detectStructuralCorruption.
_CORRUPTION_NEEDLES = (
    "Truncated mdat",
    "Bad atom size",
    "Invalid mdat",
    "Could not parse moov",
)


def embed_xmp(
    video: Path,
    json_path: Path,
    scratch_dir: Path,
    max_attempts: int = 3,
) -> bool:
    """Embed XMP into `video` via exiftool.

    Two paths, chosen automatically from `os.stat(...).st_dev`:

    * **Same filesystem as scratch_dir → in-place fast path.**
      Run exiftool directly on `video`.  Used when this script runs *on*
      the storage host (e.g., ssh'd into the Synology that owns the
      share).  No copy, no scratch use — just a single rewrite on
      whatever local disk holds the file.

    * **Different filesystem → local-scratch round-trip.**
      Used when `video` is on an SMB/NFS share mounted at the client.
      `exiftool -overwrite_original` reads the whole file, writes a full
      copy to a sidecar tmp, then renames; done in place over `smbfs`
      that pattern stalls the connection (the smbmounter daemon
      decides the mount is wedged and force-unmounts).  Instead we
      copy `video` → `scratch_dir` (local), run exiftool there, then
      stream the result back via a temp name beside the original
      followed by `os.replace` for an atomic swap.  Only SMB I/O is
      one sequential read down + one sequential write back — the
      pattern smbfs handles well.

    Returns:
        True  if XMP was embedded,
        False if the sidecar yields zero assignable tags (soft skip).
    Raises:
        EmbedError if every exiftool attempt fails.  `.output` is
        matched against `_CORRUPTION_NEEDLES` at the call site so
        corrupt-source cases stay separable from transient failures.
    """
    # Avoid copying a multi-GB video just to discover there's nothing to embed.
    histogram = load_sidecar_histogram(json_path)
    if not histogram:
        return False
    assignments = build_exiftool_assignments(histogram)
    if not assignments:
        return False

    scratch_dir.mkdir(parents=True, exist_ok=True)

    # Choose path based on whether the video and the scratch dir share a
    # filesystem.  We compare device ids of the video's *parent dir* (the
    # video may not exist yet at the path under test if a previous attempt
    # was interrupted) — but here video does exist, so stat it directly.
    try:
        same_fs = (
            os.stat(str(video)).st_dev == os.stat(str(scratch_dir)).st_dev
        )
    except OSError:
        same_fs = False

    if same_fs:
        # In-place: exiftool rewrites `video` directly; its sidecar tmp
        # lands next to `video` (also on local disk).  No copy, no
        # scratch use.
        target = video
        local_tmp = video.with_name(video.name + "_exiftool_tmp")
        local_copy: Path | None = None
        share_tmp: Path | None = None
    else:
        # Round-trip: copy down to local scratch, work there, then ship
        # back via a same-directory temp on the share.
        fd, local_name = tempfile.mkstemp(
            prefix=f"{video.stem}_",
            suffix=video.suffix,
            dir=str(scratch_dir),
        )
        os.close(fd)
        local_copy = Path(local_name)
        local_tmp = local_copy.with_name(local_copy.name + "_exiftool_tmp")
        share_tmp = video.with_name(video.name + ".embed_upload_tmp")
        # SMB → local: a single sequential read.
        shutil.copyfile(video, local_copy)
        target = local_copy

    try:
        cmd = ["exiftool", "-overwrite_original", "-q"] + assignments + [str(target)]
        last_output = ""
        last_code = 0
        ok = False
        for _ in range(max_attempts):
            # exiftool doesn't clean its own tmp on failure; sweep before
            # each try so a leftover from a previous attempt doesn't
            # trigger "Temporary file already exists".
            if local_tmp.exists():
                try:
                    local_tmp.unlink()
                except OSError:
                    pass
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode == 0:
                ok = True
                break
            last_code = result.returncode
            last_output = ((result.stdout or "") + (result.stderr or "")).strip()
            # Definitive corruption — retrying won't help.  Surface so the
            # caller records this in the "needs re-render" bucket via
            # _CORRUPTION_NEEDLES.
            if any(n in last_output for n in _CORRUPTION_NEEDLES):
                break
        if not ok:
            raise EmbedError(
                f"exit {last_code} after {max_attempts} attempt(s)",
                last_output,
            )

        if not same_fs:
            # local → SMB: a sequential streaming write.  Going via a
            # temp name in the same directory keeps the replace atomic
            # and means a crash mid-write leaves `video` untouched.
            assert local_copy is not None and share_tmp is not None  # for type checkers
            shutil.copyfile(local_copy, share_tmp)
            os.replace(share_tmp, video)
        return True
    finally:
        # Always remove exiftool's sidecar (it lives next to `target`),
        # plus the local copy and share temp if the round-trip path was
        # used.  These can be full-size video copies — leaking them
        # fills the scratch (or share) volume.
        for p in (local_copy, local_tmp, share_tmp):
            if p is None:
                continue
            try:
                if p.exists():
                    p.unlink()
            except OSError:
                pass


# --- Glue / main -----------------------------------------------------------


def write_histogram_json(histogram: dict[str, dict[str, int]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w") as f:
        json.dump(histogram, f, indent=3, sort_keys=True)


def ensure_exif_json(
    base: str,
    archives: dict[str, Path],
    existing_jsons: dict[str, Path],
    unreachable_bases: set[str],
    raw_types: list[str],
    scratch_root: Path,
    parallelism: int,
    log,
) -> Path | None:
    """Return a path to an exif json for `base`, generating from a 7z if needed.

    `unreachable_bases` is a per-run cache of bases whose backing storage
    (json or archive) went away during processing.  Without it, every video
    that depends on the same dead base re-pays the cost of a 7za call that
    we already know will fail.
    """
    if base in unreachable_bases:
        log(f"  base '{base}' already known unreachable this run; skipping")
        return None

    cached = existing_jsons.get(base)
    if cached is not None:
        if cached.exists():
            log(f"  using existing exif json {cached}")
            return cached
        # Indexed at startup but not reachable now (e.g., network mount went
        # away).  Drop and fall through to the archive path.
        log(f"  indexed exif json {cached} is no longer reachable; falling through")
        del existing_jsons[base]

    if base not in archives:
        log(f"  no .7z archive found for base sequence '{base}'")
        unreachable_bases.add(base)
        return None

    archive = archives[base]
    out_json = archive.with_suffix(".json")
    try:
        if out_json.exists():
            log(f"  found exif json next to archive: {out_json}")
            existing_jsons[base] = out_json
            return out_json
    except OSError as err:
        log(f"  stat of {out_json} failed ({err}); treating archive as unreachable")
        unreachable_bases.add(base)
        return None

    log(f"  extracting raw frames from {archive}")
    scratch = scratch_root / f"video_metadata_tool_{base}"
    try:
        if scratch.exists():
            shutil.rmtree(scratch)
        try:
            images = extract_raw_frames(archive, raw_types, scratch)
        except SevenZipError as err:
            # 7za failed.  Surface its own stderr so we can tell whether it
            # was a network glitch, a structurally bad archive, or some other
            # cause — the bare exit code by itself isn't actionable.  Mark
            # the base unreachable so the next dozen videos that share it
            # don't each pay another 7za round-trip.
            log(f"  {err}; skipping")
            for line in (err.output or "(no output)").splitlines():
                log(f"    | {line}")
            unreachable_bases.add(base)
            return None
        except OSError as err:
            log(f"  could not read {archive}: {err}; skipping")
            unreachable_bases.add(base)
            return None
        if not images:
            log(f"  no raw images ({raw_types}) found in {archive}")
            unreachable_bases.add(base)
            return None
        log(f"  scanning {len(images)} frames with exiftool")
        histogram = build_histogram(images, parallelism)
        if not histogram:
            log("  exiftool produced no usable tags")
            unreachable_bases.add(base)
            return None
        write_histogram_json(histogram, out_json)
        existing_jsons[base] = out_json
        log(f"  wrote {out_json}")
        return out_json
    finally:
        if scratch.exists():
            shutil.rmtree(scratch, ignore_errors=True)


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--config", required=True, type=Path, help="Path to config json")
    p.add_argument("--apply", action="store_true",
                   help="Actually embed metadata. Without this flag, the script only reports.")
    p.add_argument("--generate-only", action="store_true",
                   help="Generate any missing exif json files but do not embed into videos.")
    p.add_argument("--filter", type=str, default=None,
                   help="Only process videos whose path matches this substring.")
    p.add_argument("--clean-stale-tmp", action="store_true",
                   help="Walk video paths and delete any *_exiftool_tmp files left behind by "
                        "a previous killed run; then exit. Combine with --filter to scope.")
    p.add_argument("--verbose", "-v", action="store_true")
    args = p.parse_args()

    if args.apply and args.generate_only:
        sys.exit("--apply and --generate-only are mutually exclusive")

    cfg = Config.load(args.config)

    if not shutil.which("exiftool"):
        sys.exit("exiftool not found in PATH")
    if not shutil.which("7za"):
        sys.exit("7za not found in PATH")

    scratch_root = cfg.scratch_dir or Path(tempfile.gettempdir())
    scratch_root.mkdir(parents=True, exist_ok=True)

    def log(msg: str) -> None:
        print(msg, flush=True)

    log(f"scanning videos under {len(cfg.video_paths)} path(s)...")
    videos = find_videos(cfg.video_paths, cfg.video_extensions)
    if args.filter:
        videos = [v for v in videos if args.filter in str(v)]
    log(f"  found {len(videos)} video file(s)")

    if args.clean_stale_tmp:
        removed = 0
        for root in cfg.video_paths:
            if not root.is_dir():
                continue
            for tmp in root.rglob("*_exiftool_tmp"):
                if args.filter and args.filter not in str(tmp):
                    continue
                try:
                    size = tmp.stat().st_size
                    tmp.unlink()
                    log(f"  removed {tmp} ({size:,} bytes)")
                    removed += 1
                except OSError as err:
                    log(f"  could not remove {tmp}: {err}")
        log(f"removed {removed} stale temp file(s)")
        return 0

    log("indexing raw archives and exif json sidecars...")
    archives = find_raw_archives(cfg.raw_archive_paths)
    existing_jsons = find_exif_jsons(cfg.exif_json_paths)
    log(f"  {len(archives)} archive(s), {len(existing_jsons)} json sidecar(s)")

    registry = sorted(set(archives) | set(existing_jsons), key=len, reverse=True)

    has_meta: list[Path] = []
    missing: list[tuple[Path, str | None]] = []
    unresolved: list[Path] = []

    for video in videos:
        if has_xmp_metadata(video):
            has_meta.append(video)
            continue
        base = resolve_base_name(video.stem, registry)
        if base is None:
            unresolved.append(video)
            missing.append((video, None))
        else:
            missing.append((video, base))

    log("")
    log(f"summary: {len(has_meta)} with metadata, {len(missing)} missing, {len(unresolved)} unresolved")

    if args.verbose:
        for v in has_meta:
            log(f"  OK  {v}")

    for video, base in missing:
        if base is None:
            log(f"  MISSING  {video}  (no base sequence match in registry)")
        else:
            json_path = existing_jsons.get(base)
            origin = f"json={json_path}" if json_path else f"archive={archives.get(base)}"
            log(f"  MISSING  {video}  base={base}  {origin}")

    if not args.apply and not args.generate_only:
        log("")
        log("(report only - re-run with --apply to embed, or --generate-only to create json files)")
        return 0

    log("")
    log("processing missing videos...")
    embedded = 0
    # Per-run cache of base names whose backing storage has already been
    # proven dead this run (network mount disappeared, archive truncated,
    # whatever).  Lets us short-circuit the 6-12 follow-up videos that share
    # the same base without re-running 7za each time.
    unreachable_bases: set[str] = set()
    failures: dict[str, list[Path]] = {
        "no_base": [],
        "no_json": [],
        "no_tags": [],
        "corrupt_video": [],
        "write_error_retries_exhausted": [],
        "exiftool_other": [],
        "json_read": [],
    }
    for video, base in missing:
        if base is None:
            log(f"SKIP {video}: no base sequence")
            failures["no_base"].append(video)
            continue
        log(f"{video}")
        json_path = ensure_exif_json(
            base=base,
            archives=archives,
            existing_jsons=existing_jsons,
            unreachable_bases=unreachable_bases,
            raw_types=cfg.raw_types,
            scratch_root=scratch_root,
            parallelism=cfg.exiftool_parallelism,
            log=log,
        )
        if json_path is None:
            log("  could not obtain exif json; skipping")
            failures["no_json"].append(video)
            continue

        if args.generate_only:
            continue

        try:
            log("  embedding XMP metadata via exiftool...")
            if embed_xmp(video, json_path, scratch_root):
                log("  done")
                embedded += 1
            else:
                log("  json produced no assignable tags; skipping embed")
                failures["no_tags"].append(video)
        except EmbedError as err:
            # `Truncated mdat atom` and friends are definitive corruption in
            # the .mov itself.  Surface separately so the user can re-render
            # those videos.
            if any(n in err.output for n in _CORRUPTION_NEEDLES):
                log("  source video is structurally corrupt; needs re-render")
                for line in err.output.splitlines():
                    log(f"    | {line}")
                failures["corrupt_video"].append(video)
            elif "Error writing output file" in err.output:
                # Empirically: same files alternate between this and
                # "Truncated mdat" across runs (SMB-mounted output dir +
                # borderline-corrupt container).  After N retries this is
                # almost certainly also corruption-flavored.
                log("  write failed after retries; probably corrupt or SMB-flaky, treat as needs-re-render")
                for line in err.output.splitlines():
                    log(f"    | {line}")
                failures["write_error_retries_exhausted"].append(video)
            else:
                log(f"  exiftool failed: {err}")
                for line in err.output.splitlines():
                    log(f"    | {line}")
                failures["exiftool_other"].append(video)
        except OSError as err:
            log(f"  failed to read json {json_path}: {err}")
            failures["json_read"].append(video)

    total_failed = sum(len(v) for v in failures.values())
    log("")
    log(f"done: {embedded} embedded, {total_failed} skipped")
    summary_labels = {
        "no_base":                       "no matching base sequence in registry",
        "no_json":                       "could not obtain exif json",
        "no_tags":                       "exif json produced no assignable tags",
        "corrupt_video":                 "corrupt video (truncated mdat) - needs re-render",
        "write_error_retries_exhausted": "write failed after retries (probably corrupt or SMB-flaky) - needs re-render",
        "exiftool_other":                "exiftool failed for other reasons",
        "json_read":                     "could not read exif json",
    }
    for key, label in summary_labels.items():
        bucket = failures[key]
        if not bucket:
            continue
        log("")
        log(f"  {label} ({len(bucket)}):")
        for v in bucket:
            log(f"    {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
