# 0979 — The ExifTool pin is filled in by the owner

Status: accepted (TRD-23, stage s986)

## Context

`v3/exiftool/versions.env` must hold the newest production ExifTool release, its download URLs,
the SHA-256 of each downloaded file and the Windows mirror. `fetch.sh` refuses to run while
`EXIFTOOL_VERSION` is empty, and `exiftool-smoke` runs `fetch.sh` first, so it stays red until the
pin is committed.

The implementation environment has no network access. A checksum must be computed from the file
as it is actually downloaded, so it cannot be copied from a web page or guessed.

## Decision

The owner fills in the pin on a networked machine by following the "How to bump" steps in
`v3/exiftool/README.md`, commits the filled `versions.env`, and re-runs
`bash v3/scripts/suite.sh exiftool-smoke`.

No agent invents, copies or pre-fills the version, URLs or hashes.

## Consequences

- TRD-23 acceptance criterion A3 (`exiftool-smoke` passes on the owner's Mac) waits on that commit.
- A1 (`checksum_mismatch`) and A2 (`v3-exiftool.yml`) do not depend on the pin.
- The suite is not made to pass while unpinned. Making it pass without a pin would hide that the
  bundle has never been fetched or verified.

Related: stage note `docs/stages/s986.md`.
