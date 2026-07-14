# video_metadata_tool

Inspect timelapse videos for embedded XMP-EXIF metadata and restore it
from the original raw image sequence when missing.

The metadata model is the same as the Swift daemon's `XMPEmbedder`:
canonical XMP-tiff (Make/Model), XMP-aux (Lens), and XMP-exif (ISO,
FNumber, ExposureTime, FocalLength, ExposureProgram, ExposureMode,
WhiteBalance, DateTimeOriginal) packed into the video container via
`exiftool`.

## How a video is matched to its raw archive

The tool builds a registry of base sequence names from every `.7z`
archive under `raw_archive_paths` and every `.json` sidecar under
`exif_json_paths`.  For each video, the longest base name that is a
separator-bounded prefix of the video filename wins.  Variants like
`-aurora`, `-aurora-topaz`, `-aurora-topaz-star-v-0_10_6` all collapse
back to the same base archive without any pattern-matching on the date
or camera fields.

## Configuration

```json
{
    "video_paths":         ["/path/to/videos"],
    "raw_archive_paths":   ["/path/to/raws"],
    "exif_json_paths":     ["/path/to/sidecars"],
    "raw_types":           ["ARW", "ORF"],
    "video_extensions":    ["mov", "mp4"],
    "scratch_dir":         "/path/to/scratch",
    "exiftool_parallelism": 8
}
```

`scratch_dir` is optional — defaults to `$TMPDIR`.  Generating an exif
json from a `.7z` extracts every raw frame into the scratch dir
temporarily, so the scratch dir needs enough free space for the
uncompressed sequence (typically 1.5-2x the archive size).

`video_paths`, `raw_archive_paths`, and `exif_json_paths` are searched
recursively.

## Modes

- **report only (default)**: scan, list which videos are missing metadata, and
  whether a matching base sequence exists in the registry.  Touches nothing.

```bash
./inspect_video_metadata.py --config config.json
```

- **`--generate-only`**: for every missing video, ensure an exif json exists
  (generating one from the `.7z` if needed).  Writes the json next to the
  archive.  Does not modify videos.

```bash
./inspect_video_metadata.py --config config.json --generate-only
```

- **`--apply`**: do the full pipeline — find missing videos, generate any
  needed exif json files, and embed the XMP packet into each video with
  `exiftool -overwrite_original`.

```bash
./inspect_video_metadata.py --config config.json --apply
```

- **`--filter SUBSTR`**: process only videos whose full path contains
  `SUBSTR`.  Useful for restricting work to a single sequence.

## Where to run the script

The same script handles both cases.  `embed_xmp` compares `os.stat().st_dev`
of the video against the scratch dir to pick the path automatically.

**Client (videos on an SMB/NFS mount).**  `embed_xmp` copies the video
down to `scratch_dir` (local), runs exiftool there, and streams the
finished file back to the share via a temp + `os.replace`.  Cross-FS
detection picks this path when `video.st_dev != scratch_dir.st_dev`.
Required because `exiftool -overwrite_original` performed in place over
`smbfs` triggers a read + full-rewrite + rename pattern that wedges
the SMB connection; the round-trip turns it into one sequential read
and one sequential write, which `smbfs` handles fine.

**On the storage host (e.g., ssh'd into the Synology that owns the
share).**  When `video` and `scratch_dir` are on the same filesystem,
the script skips the round-trip entirely and runs exiftool in place
on the video.  No copy, no scratch use — typically a 10×+ speedup
since you're not pushing GB of video over the network.

### Synology setup

If you ssh into the Synology and run the script there directly, you
need three things on the box:

1. **Python 3.9+** — install the *Python 3* package from Package Center
   (DSM 7.2+ ships 3.9+).  The script uses `from __future__ import
   annotations` so its 3.10-style type hints work fine on 3.9.

2. **exiftool and p7zip** — install via SynoCommunity or Entware (Synology
   doesn't include either by default).  With Entware:
   ```
   opkg install perl-image-exiftool p7zip-full
   ```
   Verify: `which exiftool` and `which 7za` should both resolve before
   running the script.

3. **A config with local paths.**  Replace `/mammoth/Photos/timelapses/...`
   in your client config with the underlying Synology paths
   (`/volume1/Photos/timelapses/...` or wherever the share is rooted),
   and put `scratch_dir` on the same volume so the fast path triggers:

   ```json
   {
       "video_paths":       ["/volume1/Photos/timelapses/2026/2026-video"],
       "raw_archive_paths": ["/volume1/Photos/timelapses/2026"],
       "exif_json_paths":   ["/volume1/Photos/timelapses/2026"],
       "scratch_dir":       "/volume1/Photos/timelapses/scratch",
       "raw_types":         ["ARW", "ORF"]
   }
   ```

The script logs no special marker for which path it took — if you want
to confirm the fast path is active, watch the scratch dir: it stays
empty during embeds when video and scratch share a filesystem.

## Detection rule

A video is "missing metadata" if `exiftool -G1 -XMP-aux:all -XMP-exif:all
-XMP-tiff:all` returns no output.  Videos rendered by the new Swift
daemon (commit `4e5810e` onward) have these groups populated; older
videos rendered by the legacy Perl daemon do not, even if they may have
a bare `Make` / `Model` from ffmpeg's `-metadata` flag.

## Requirements

- Python 3.9+ (uses the walrus operator and `from __future__ import
  annotations` to allow 3.10-style type union syntax)
- `exiftool` in `$PATH`
- `7za` in `$PATH` (macOS: `brew install p7zip`; Synology/Entware:
  `opkg install p7zip-full`)
