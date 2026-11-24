# 1049: render queue, pipeline and control choices in trd-core (TRD-12)

## Decision

- A failure card blocks requeueing its output until it is dismissed or retried. Dismiss only
  removes the card, so the next scan renders the variant again and a recurrence is a new card,
  as the overview §6.9 wording "until the same render fails again" implies. Retry also clears the
  pause and the corrupt strikes, and asks for a scan.
- Auto-pause (§6.6 step 5) needs three corrupt renders in a row. Because a corrupt render leaves a
  card, the strikes accumulate only through dismissals, not through Retry, which clears them.
- Queued jobs that are no longer wanted become `cancelled`. The store has no delete, and a
  cancelled row is no longer queued. The same applies to a queued job whose variant is paused.
- Suspended jobs count toward both caps (`max_processes.ffmpeg` and `max_cuncurrent_renders`), since
  a suspended job keeps its slot (§6.8, overview §6.9).
- "Blocked" in §6.7 is not defined beyond "paused". A sequence is queued when it is complete, not
  paused and has its histogram. The histogram arrives through `plan_jobs`, keyed by raw directory.
- `EVENT_*` names are log rows with the `event` field set to the `trd.proto` enum names. Only the
  six this ticket emits are defined, in `events.rs`.
- `first_seen` and the histograms are inputs from the engine, not stored tables.
- `EVENT_SEQUENCE_FINISHED` counts the newest TRD row of each configured variant that was rendered
  at or after the time this process first saw the sequence complete (`complete_since`, in memory,
  §6.6 step 7). Rows from before a restart are not counted, and an older row of a variant is not
  counted twice.
- A control that applies writes one INFO row with subject `controls`. A rejected control writes
  nothing.
- Pause sequence, pause variant and Stop end their in-flight renders first, then write
  `config.json`. A render that cannot be ended fails the control with the config unchanged; the
  renders already ended stay ended, and the next scan requeues their variant.
- A process is signalled only when this queue holds it, or when the row's pid still has the
  recorded start time (`live_pid`). Suspend and resume refuse a row with no live process
  (FAILED_PRECONDITION; Stop still ends it). Pause and Resume rendering skip such a row with a WARN
  row and leave it as it was.
- A completion that fails part way through (a store or file error after the process has ended)
  marks the row failed with a WARN row (`release_job`), so it no longer owns its output or a slot.
- Failure cards are removed when last seen more than 30 days ago (`store::FAILURE_RETENTION_MS`,
  applied by `prune_with`).

## Why

The overview's dismiss semantics and the spec's "not re-queued" rule only agree if a card blocks
requeueing while it is shown. Auto-pause needs repeated renders, so corrupt strikes have to build
up across dismissals.

## What still differs

- The XMP step's exiftool runs through a `/bin/sh` stand-in in `tests/queue.rs`, so that file is
  `cfg(unix)`. The fake ffmpeg is the `trd-testtools` binary, passed its script through the
  process environment of the queue, not the test process's environment.
- Adoption of running renders after a restart is TRD-13, so a job left `running` by a previous
  daemon is not picked up here. Until then such a row has no live process and counts as in flight;
  Stop ends it.
- The error paths of `release_job` and of a failed `terminate` in Stop/Pause have no test: the
  tests have no way to make the store or a signal fail.
