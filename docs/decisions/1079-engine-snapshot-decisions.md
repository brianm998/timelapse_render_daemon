# 1079: engine, snapshot and problem choices in trd-core (TRD-16)

## Decision

- The snapshot revision is compared on content only. Every publish rebuilds the snapshot, and a new
  revision is sent only when it differs from the last published one (`generated_at_ms` and the
  revision are left out of the comparison). A tick that scans, or a control that applies, is one
  step; a refused control and an idle tick are none. A scan changes `last_scan_completed_ms`, so a
  scan that runs is always a step.
- Start-up does not publish. The first tick publishes, so no client sees "config missing" for a
  config that the same tick loads.
- Phase precedence follows the §6.10 list: starting (until a tick has run), no config, waiting for
  disk access, paused, rendering, exporting, idle. Waiting for disk access therefore needs a config
  that is loaded; with no config loaded the phase is no config.
- While Full Disk Access is missing the engine clears the scan (so no configured path is listed),
  keeps reaping running renders, and does not reload the config, promote or start EXIF jobs.
- A sequence whose export has more frames than its raw sequence is withheld from `Renders::on_scan`.
  It is listed as `STAGE_COUNT_MISMATCH`, and any of its queued jobs is cancelled as "its sequence
  is gone" by the queue's own removal.
- Sequences whose variants are all done or already rendered are not listed. A sequence listed as
  `STAGE_PAUSED` shows its variants as paused.
- Stage, in order: paused, count mismatch, exporting, rendering (running or suspended variant), queued,
  collecting metadata (complete, histogram unknown), else blocked. Rank 0 of the `in_progress` order
  is by the highest running variant's frame fraction, rank 2 by exported over expected frames.
- `counts.exporting` counts listed cards in `STAGE_EXPORTING`; `counts.paused_sequences` counts
  listed cards with the paused flag; `counts.rendering` and `counts.queued` count the store's
  running and queued rows (adopted renders included); `rendered_last_24h` counts all rows dated in
  the last 24 hours; `recently_rendered_total` counts rows within `recent_render_history_days`.
- Unknown codecs: the loader drops the variant and raises `PROBLEM_UNKNOWN_CODEC`, so the name is
  read back from config.json on each reload that changed it, and reported as `codec:<name>`.
- Problem severities: fda, config missing and config invalid, ffmpeg and ffprobe tools, and an
  ignored config entry (`config-issues`) are ERROR; exiftool missing or broken is WARNING; an
  unreadable configured folder (`path:<path>`) is WARNING. The fda text is the constants of
  `fda.rs`; the other titles and fixes are the engine's own wording.
- Tools are resolved at start-up and every five minutes; a tool that is missing or broken raises
  `tool:<name>` until a later check finds it.
- Pause-all: `controls.rs` skips a `running` row whose `foreign` flag is set. Without this a global
  pause suspended an adopted ffmpeg the daemon does not own, against §6.11 step 3 and TRD-16 A10.
  This is the one change outside the ticket's listed files.
- Histograms: a complete sequence without a sidecar or cached histogram starts an EXIF collection on a
  blocking thread. A failed collection is tried again after a minute. Its result is written by
  `exif::write_sidecar` and kept in memory per raw directory.
- Tests need the fake ffmpeg to hold: a render is held only when its command line carries a
  `-progress` argument, so the foreign-render fixture passes one.
