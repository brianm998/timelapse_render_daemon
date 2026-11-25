# 1079: engine, snapshot and problem choices in trd-core (TRD-16)

## Decision

- The snapshot revision is compared on content only. Every publish rebuilds the snapshot, and a new
  revision is sent only when it differs from the last published one. `generated_at_ms`, the
  revision and the last-scan timing (`last_scan_completed_ms`, `last_scan_duration_ms`) are left out
  of the comparison, so a scan that changes nothing is not a step and an idle daemon publishes
  nothing. The timing reaches clients with the next real change. A tick that changes something, or a
  control that applies, is one step; a refused control and an idle tick are none.
- Start-up does not publish. The first tick publishes, so no client sees "config missing" for a
  config that the same tick loads.
- Start-up is split. `startup` resolves the tools and nothing else. The steps that read the
  configured folders or need the config's settings (`begin`: re-adoption, the Swift history import,
  the stale partial sweep) run once, at the first tick that has Full Disk Access and a loaded config.
  Retention runs only once a config is loaded, so it never prunes on the default window.
- Full Disk Access is checked first in every tick. While it is missing the tick reads nothing: it
  does not reload the config (loading stats the configured folders), scan, reap, promote, collect
  EXIF or probe a file. Running renders are not signalled; their completion is picked up once access
  is back, so a render that finishes during the block waits in the database. The rows of the recently
  rendered list keep the last `file_exists` answer held; a row with no held answer shows `false`
  until access is back.
- Phase precedence: starting (until the first tick has run), waiting for disk access, no config,
  paused, rendering, exporting, idle. Waiting comes before no config. The spec's list puts no config
  first, but the config cannot be read while access is missing, so at launch without access the
  phase must be waiting for disk access, not no config.
- Starting ends at the first tick, not at the first scan as §6.10 says. A tick with no config, or with
  access missing, never scans, so holding starting until a scan would leave `PHASE_NO_CONFIG`
  unreachable and the phases of A5 unreached. This is the deviation from §6.10.
- The config problems (`config`, `config-issues`) are not reported until a tick has read the config
  file. A config never read is not "missing".
- `EVENT_CONFIG_RELOADED` is written only when a reload leaves a loaded config (`Status::Loaded`). A
  missing or invalid file writes no event; its `config` problem says what is wrong.
- A sequence whose export has more frames than its raw sequence is withheld from `Renders::on_scan`.
  It is listed as `STAGE_COUNT_MISMATCH` with the §6.10 note, and any of its queued jobs is cancelled
  as "its sequence is gone" by the queue's own removal.
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
- Histograms: a complete sequence without a sidecar or cached histogram starts an EXIF collection on a
  blocking thread. A failed collection is tried again after a minute. Its result is written by
  `exif::write_sidecar` and kept in memory per raw directory.
- Tests need the fake ffmpeg to hold: a render is held only when its command line carries a
  `-progress` argument, so the foreign-render fixture passes one.

## Scope

The ticket's listed files are `engine.rs`, `snapshot.rs`, `problems.rs`, `events.rs` and
`tests/engine.rs`. Two more files are outside that list, and both are required by mason: the stage
note `docs/stages/s1090.md` and this record. The one code change outside the list is `controls.rs`
(the pause-all guard above, which A10 needs). No other file outside the list changes.
