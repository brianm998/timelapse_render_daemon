# 1006: render planning choices in trd-core (TRD-07)

## Decision

- A render that needs the source size (original-size, or width only) probes the first frame with
  ffprobe. A failed probe is `NotQueueable` with "could not read the frame size of <file>", so
  the render is refused rather than guessed. The Swift daemon fell back to a 16:9 aspect here.
- Original-size renders take their `-aspect` from the source size, not from 16:9 (§6.5, §3
  difference 1).
- `PlanError::UnknownCodec` is an error value. Its failure kind is `FAILURE_NOT_QUEUEABLE`, as
  for the other planning failures.
- The `no pattern` failure carries the summary "no numbered image sequence in <dir>". The longer
  input analysis is TRD-11's, so the detail here only lists the inputs that analysis needs.
- `JobStatus` is defined in `plan.rs` with the `render_jobs` labels, so TRD-12 reuses it instead
  of defining a second copy.

## Why

The spec gives the probe for width-only and original-size renders and a failure kind for it, so
the planner must not invent a size. Keeping the codec error separate from the probe error lets
the caller tell "no such codec" from "the file is unreadable".

## What still differs

- The ffprobe call is not bounded by `PROBE_TIMEOUT` yet. `Tools` defines the timeout; applying it
  needs a spawn-and-wait helper that is shared with the other tool calls.
- Logging a pattern that differs from the configured prefix is the engine's job (TRD-08), not
  the planner's.
