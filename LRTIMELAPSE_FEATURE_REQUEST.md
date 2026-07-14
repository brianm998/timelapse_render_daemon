# Feature Request: Embed source EXIF metadata as XMP (and other supported containers) into rendered timelapse videos

## Summary

When LRTimelapse renders a timelapse to a video file, **the rich EXIF metadata
from the original RAW source frames is lost**. The output `.mov`/`.mp4` carries
almost nothing about the camera, lens, or exposure that produced it.

I'd like LRTimelapse to **collect as much EXIF metadata as possible from the
source image sequence and embed it into the rendered video** — primarily as an
**XMP packet** (the cross-vendor standard that Premiere, Lightroom, Bridge,
exiftool, and DAM/cataloging tools already read), and additionally into the
other metadata containers a video file supports where appropriate.

This is low-risk and high-value: LRTimelapse already embeds ffmpeg and already
has the RAW sequence in hand, so all the inputs exist. The work is to read the
EXIF, reduce it across the sequence, and write it into the output container.

---

## Motivation

A timelapse is the one kind of footage where the per-frame still metadata is
both **uniform** (same camera/lens, near-constant exposure for holy-grail-aside
clips) and **completely discarded** by the render. A photographer who shoots
500 RAW frames on a Sony FX3 + 14mm GM, renders an MOV, and drops it into a
video library has no way to later answer "show me everything shot at 14mm" or
"everything from this camera body" — because none of it survived the render.

Every other camera-native video file (iPhone, GoPro, DJI, Sony pro clips)
carries camera/lens/exposure/GPS metadata that video cataloging and editing
tools index and filter on. A rendered timelapse should be a first-class citizen
in that ecosystem instead of an anonymous blob.

**Concrete downstream consumer:** video DAM/cataloging tools such as ReelVault
(and Adobe Bridge/Premiere) read exactly this metadata to power filtering and
search. If LRTimelapse embeds it, the rendered timelapse immediately becomes
filterable by camera, lens, focal length, ISO, aperture, shutter, and capture
date — see "Why this works downstream" below.

---

## There is a working reference implementation

This is not hypothetical. A standalone timelapse render daemon already does
exactly this end-to-end, and it can serve as a blueprint. Its pipeline:

1. **Extract EXIF from every RAW frame** in the source sequence with
   `exiftool -csv <file>`, parsing the CSV into field→value maps. (Binary tags
   such as `ThumbnailImage`, `PreviewImage`, `TiffMeteringImage` are skipped.)

2. **Aggregate across the sequence** into a per-field histogram
   (`{field: {value: count}}`) — e.g. `{"ISO": {"3200": 150, "6400": 50}}`.
   A JSON sidecar is persisted next to the sequence.

3. **Reduce each field to a single value**:
   - **Mode (most frequent value)** for camera/lens/exposure fields — robust to
     a few outlier frames, ties broken deterministically.
   - **Earliest timestamp** for `DateTimeOriginal`, so the embedded capture date
     marks the *start* of the timelapse.
   - Only fields that resolve to a single representative value are written.

4. **Render with ffmpeg** (image sequence → `.mov`/`.mp4`).

5. **Embed XMP after render** with exiftool, writing into the correct atom/box
   for the container (see below). The same exiftool write also acts as a
   structural validation pass — a truncated/corrupt output fails the embed and
   is re-queued.

The mapping it uses today (EXIF tag → XMP property):

| Source EXIF tag    | Reduction        | XMP target                  | Notes |
|--------------------|------------------|-----------------------------|-------|
| `Make`             | mode             | `XMP-tiff:Make`             | |
| `Model`            | mode             | `XMP-tiff:Model`            | |
| `LensModel`        | mode             | `XMP-aux:Lens`              | |
| `ISO`              | mode             | `XMP-exif:ISO`              | |
| `FNumber`          | mode             | `XMP-exif:FNumber`          | aperture, e.g. `1.8` |
| `ExposureTime`     | mode             | `XMP-exif:ExposureTime`     | e.g. `1/250` |
| `FocalLength`      | mode             | `XMP-exif:FocalLength`      | `"14.0 mm"` → `14.0` |
| `ExposureProgram`  | mode             | `XMP-exif:ExposureProgram#` | integer enum 0–8 |
| `ExposureMode`     | mode             | `XMP-exif:ExposureMode#`    | integer enum 0–2 |
| `WhiteBalance`     | mode             | `XMP-exif:WhiteBalance#`    | integer enum 0–1 |
| `DateTimeOriginal` | **earliest**     | `XMP-exif:DateTimeOriginal` | EXIF `:` form → ISO-8601 |

(The `#` suffix on the enum tags tells exiftool to write the raw integer rather
than the human-readable PrintConv string.)

This is a sound, minimal baseline. **This feature request asks LRTimelapse to do
the same — and to go further on coverage** (below).

---

## What I'm asking for

### 1. Collect as much EXIF as possible (not just the baseline set)

The reference baseline writes ~11 fields. LRTimelapse should aim to capture the
**full useful EXIF surface** of the source frames, including fields the baseline
omits:

- **Camera body**: Make, Model, **SerialNumber / BodySerialNumber**, FirmwareVersion
- **Lens**: LensModel, **LensMake**, **LensSerialNumber**, **LensInfo / LensID**,
  **FocalLengthIn35mmFormat**
- **Exposure**: ISO, FNumber, ExposureTime/ShutterSpeed, FocalLength,
  **ExposureProgram, ExposureMode, MeteringMode, WhiteBalance, Flash,
  ExposureCompensation**
- **Capture time**: DateTimeOriginal (earliest = start), and consider
  **also recording the latest** so the clip's covered time span is preserved
  (e.g. as a custom XMP property and/or as the clip duration's implied end)
- **GPS** (the baseline skips this entirely): **GPSLatitude, GPSLongitude,
  GPSAltitude** — extremely valuable for map-based browsing, and many timelapse
  rigs do record it. Should be written both to XMP (`exif:GPSLatitude` etc.)
  **and** to the QuickTime ISO-6709 location tag (see container notes).
- **Color/profile hints**: ColorSpace, and any camera profile / picture-profile
  / gamma fields the RAW exposes.

Where a field genuinely varies across the sequence (e.g. holy-grail ramps where
ISO/shutter change frame to frame), the tool should still be able to record a
**representative value (mode)** plus optionally a **min/max range** in a custom
XMP property, rather than dropping the field. A timelapse with ramped exposure
is precisely where "ISO 100→3200 over the clip" is interesting metadata.

### 2. Sensible aggregation across the sequence

Mirror the reference approach:
- **Mode** for categorical/steady fields (camera, lens, color space, and steady
  exposure).
- **Earliest** for capture date (clip start); optionally **latest** as span end.
- **Min/max (or mode + range)** for fields that ramp.
- A representative **GPS fix** (e.g. first valid fix, or median) for the clip.

### 3. Embed into the right container locations

XMP-in-video has well-defined, standard storage locations
([Adobe XMP Specification Part 3: Storage in Files](https://archimedespalimpsest.net/Documents/External/XMP/XMPSpecificationPart3.pdf)):

- **`.mov` (QuickTime):** XMP packet in the **`moov/udta/XMP_`** atom.
- **`.mp4` (ISO BMFF):** XMP packet in a **top-level `uuid` box** whose 16-byte
  UUID is the Adobe XMP GUID **`BE7ACFCB-97A9-42E8-9C71-999491E3AFAC`**
  (the same GUID JPEG 2000 uses). At most one such box per file.

Since LRTimelapse already embeds ffmpeg, embedding can be done either by
shelling out to exiftool (as the reference does) or by writing the box/atom
directly. Either is acceptable; the key is the packet lands in the standard
location so downstream tools find it.

### 4. Also populate the other supported metadata containers

XMP is the priority, but a `.mov`/`.mp4` supports several metadata groups, and
different consumers prefer different ones. Where appropriate, **also** write:

- **QuickTime Keys atom** (`moov/meta/keys`+`ilst`) — reverse-DNS keyed
  metadata. This is **the group Apple's own apps prefer**, and it's where
  Apple-ecosystem tools look first. Notably:
  - `com.apple.quicktime.make`, `com.apple.quicktime.model`
  - `com.apple.quicktime.camera.lens_model`
  - `com.apple.quicktime.location.ISO6709` for GPS (format
    `±DD.DDDD±DDD.DDDD±AAAA/`)
  - `com.apple.quicktime.creationdate`
- **Container-level tags** (the `udta`/`ilst` "UserData"/"ItemList" group, i.e.
  what `ffmpeg -metadata key=value -movflags use_metadata_tags` writes):
  `make`, `model`, `lens_model`, `iso`, `fnumber`, `exposure_time`,
  `focal_length`, `creation_time`. These are the cheapest to write (ffmpeg flags
  during the existing render) and are read very broadly.

Per the metadata-container landscape, in QuickTime-based `.mov`/`.mp4`/`.m4v`,
metadata most often lives in the **Keys**, **UserData**, or **ItemList** groups,
with XMP a less common but more portable option
([Apple QuickTime metadata keys](https://developer.apple.com/documentation/quicktime-file-format/quicktime_metadata_keys),
[ExifTool QuickTime Tags](https://exiftool.sourceforge.net/TagNames/QuickTime.html)).
Writing **both** XMP and the native QuickTime groups maximizes how many tools
can read it.

For non-MP4 outputs LRTimelapse may produce, the appropriate native container
applies (e.g. **Matroska/WebM tags + embedded XMP/Exif** for `.mkv`/`.webm`;
HEIF-style sequences support XMP/Exif/MPEG-7). XMP coverage should follow
whatever the output container supports.

### 5. Make it configurable

A simple toggle ("Embed source metadata in rendered video", on by default) plus
optional control over GPS inclusion (some users may not want location embedded)
would cover the privacy/size concerns.

---

## Why this works downstream (validation)

Video cataloging tools already read precisely these locations. As a concrete
example, ReelVault (a video DAM) extracts camera/lens/exposure metadata from
videos with this priority chain — and an LRTimelapse render that follows this
request would light up **every** one of these paths:

- **XMP packet** in `moov/udta/XMP_` (MOV) or the Adobe-GUID `uuid` box (MP4),
  reading `tiff:Make`/`tiff:Model`, `exifEX:LensModel`/`aux:Lens`,
  `exif:ISOSpeedRatings`, `exif:FNumber`, `exif:ExposureTime`,
  `exif:FocalLength`, `exif:ExposureMode/Program`, `exif:WhiteBalance`,
  `exif:DateTimeOriginal`, plus `dc:` / `photoshop:` editorial fields.
- **QuickTime Keys** including the per-track `com.apple.quicktime.camera.lens_model`
  (a key ffprobe itself drops — confirmed read from real iPhone clips).
- **Container tags** (`make`, `model`, `iso`, `fnumber`, `focal_length`, …) with
  generous name matching.
- **GPS** via the `com.apple.quicktime.location.ISO6709` tag.

These fields are stored in its database and **indexed for filtering** (camera,
lens, focal length, ISO, aperture, shutter, capture date, location-on-map). In
other words: the metadata isn't embedded for its own sake — there are shipping
tools that turn it directly into search facets the moment it's present.

XMP specifically is the right primary target because it's also what
Adobe Premiere, After Effects, Audition, Bridge, and Lightroom read/write for
video ([Adobe: XMP metadata in After Effects](https://helpx.adobe.com/after-effects/using/xmp-metadata.html)).

---

## Suggested acceptance criteria

1. Rendering a timelapse from a RAW/source sequence produces an output whose
   metadata, when inspected with `exiftool output.mov`, contains the camera
   make/model, lens, focal length, ISO, aperture, shutter, exposure
   mode/program, white balance, and capture date derived from the source frames.
2. The same data is present as a valid **XMP packet** in the standard location
   for the container (`moov/udta/XMP_` for MOV; Adobe-GUID `uuid` box for MP4),
   verifiable by a third-party XMP reader (e.g. Adobe Bridge).
3. GPS, when present in the source frames, is embedded both as XMP and as the
   QuickTime ISO-6709 location tag (subject to the user's GPS toggle).
4. Fields that ramp across the sequence are represented by a mode (and,
   ideally, a min/max range) rather than being silently dropped.
5. The feature is toggleable and defaults on; embedding does not corrupt the
   rendered file.

---

## Notes / nice-to-haves

- LRTimelapse renders from TIFF intermediates produced from the RAW frames. The
  EXIF should be read from the **original RAW** frames (richest metadata), not
  the intermediate TIFFs, which may carry a reduced tag set.
- Reading EXIF from a few hundred frames is fast and trivially parallelizable
  (the reference daemon runs 8 exiftool jobs concurrently).
- exiftool is already the de-facto tool for both reading the source EXIF and
  writing the XMP into the video, and bundles cleanly; ffmpeg `-metadata` flags
  can cover the cheap container-tag group during the existing render pass.

---

## Sources

- [Adobe XMP Specification Part 3: Storage in Files](https://archimedespalimpsest.net/Documents/External/XMP/XMPSpecificationPart3.pdf) — XMP storage in MOV (`moov/udta/XMP_`) and MP4 (top-level `uuid` box, GUID `BE7ACFCB-97A9-42E8-9C71-999491E3AFAC`)
- [Extensible Metadata Platform — Wikipedia](https://en.wikipedia.org/wiki/Extensible_Metadata_Platform)
- [Apple Developer: QuickTime metadata keys](https://developer.apple.com/documentation/quicktime-file-format/quicktime_metadata_keys) and [Metadata atoms and types](https://developer.apple.com/documentation/quicktime-file-format/metadata_atoms_and_types)
- [ExifTool QuickTime Tags](https://exiftool.sourceforge.net/TagNames/QuickTime.html)
- [Adobe: XMP metadata in After Effects](https://helpx.adobe.com/after-effects/using/xmp-metadata.html)
- [FFmpeg Metadata — MultimediaWiki](https://wiki.multimedia.cx/index.php/FFmpeg_Metadata)
