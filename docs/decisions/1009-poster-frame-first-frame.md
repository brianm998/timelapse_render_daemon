# 1009: Poster frame is the first frame when the duration is unknown

Status: owner to confirm.

## Decision

A rendered video whose `duration_seconds` is 0 gets its poster frame from `-ss 0` (the first
frame) in the §6.16 thumbnail command, in place of `-ss <duration/2>`. The first frame is also
taken for any non-finite or negative duration.

This applies to imported rows from the Swift history (§6.17): the Swift history carries no
duration, and no ffprobe is run for imported files, so their `duration_seconds` is stored as 0.

## Why

The §6.16 command needs a midpoint. Without a duration there is no midpoint to seek to, and
running ffprobe on every imported file just for a poster frame was not worth it at import time.
The first frame is always present, so an imported row can still get a thumbnail on demand
(§6.16, `thumbnail_on_demand`).

## Deviation

This departs from the spec text of §6.16 ("at the middle of the file"). The spec was not edited
to bless the deviation: the owner decides whether the spec or the code changes.

## How to apply

If the owner wants the midpoint for imported rows, the import must probe each file's duration
first, or the thumbnail must be made after the file's duration is known. Change `thumbnail_args`
in `v3/trd/crates/trd-core/src/thumbs.rs` and the expected values in the tests
`thumbnail_args_match_spec` and `thumbnail_args_first_frame_without_duration` together.

Owner to confirm: keep the first frame for unknown durations, or require a probed duration.
