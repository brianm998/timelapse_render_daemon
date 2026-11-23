# 1010: EXIF collection uses a seam for batches and a sh wrapper for the real run

## Decision

`trd-core/src/exif.rs` splits collection in two: `collect_with` takes the batches and a closure that runs one batch and returns exiftool's stdout, so the concurrency test needs no process. `collect` supplies the real closure, which writes an argfile in the system temporary directory and runs `exiftool -csv -@ <argfile>`.

The sidecar is written to a hidden temporary file beside it and renamed into place. A failure of any step sends the histogram to `exif_cache` with a warning.

## Why

- A counter-based concurrency test needs no sleeps or timing (overview §10 rule 5).
- The real-process test wraps the fake exiftool in `/bin/sh`, which sets `TRD_FAKE_EXIFTOOL` for that one child, so no test changes the process environment.
- Blocking the sidecar with a directory of the same name makes the rename fail on every OS and as root, unlike `chmod`.

## Choices

- A non-zero exit from exiftool is accepted when it printed stdout (exit 1 means warnings); it is an error only when it printed nothing.
- Hidden files are not excluded from the raw listing; only the extension and the `raw_types` list decide, as §6.4 says.
