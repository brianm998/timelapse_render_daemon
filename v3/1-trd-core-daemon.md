<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Spec 1 — trd, the core daemon

Read `v3/0-overview.md` first; its decisions (§2), names (§4), information model (§6), Makefile
contract (§9), rules (§10) and ticket format (§11–§12) apply here and are not repeated.

Every ticket in specs 2–4 depends on `TRD-GATE`, which depends on every ticket in this spec.

## 1. Outcome

When this spec is done:

- `trd`, written in Rust, does everything the Perl and Swift daemons do (§3), as a background
  program that starts at login as the user, on macOS, Linux and Windows.
- It exposes the `trd.v1` gRPC API (§7) on `127.0.0.1:50071` without authentication and on
  `0.0.0.0:50072` over pinned TLS with per-device tokens, and advertises itself as `_trd._tcp`.
- It ships with its own ffmpeg and ffprobe (built from source in CI from the star recipe) and
  exiftool (pinned, fetched and verified in CI).
- A `v3.*` tag produces, through GitHub Actions: a signed and notarized macOS `.pkg` that
  installs `Timelapse Render Daemon.app` and its LaunchAgent; Linux `.deb` packages (amd64,
  arm64) with a systemd user unit, and `.tar.gz` archives; a per-user Windows setup `.exe` that
  registers a logon task, and a `.zip`; all attached to a draft GitHub Release.
- The repository is integrated with mason: `.mason/`, the issue template, the CI gate and
  `CLAUDE.md` exist, and every later ticket runs through mason.
- The API is frozen at `api_version = 1`; the clients in specs 2–4 build against it.

## 2. What to read

- In this repository: the Perl daemon `timelapse_render_daemon` and `inc/*.pm`; the Swift rewrite
  `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/*.swift`, its tests
  `rewrite/TimelapseRenderCore/Tests/TimelapseRenderCoreTests/CoreTests.swift`, and its CLI and
  GUI under `rewrite/cli` and `rewrite/gui`; the real configurations `config.json`,
  `gui_config.json` and `video_codecs.json`.
- ReelVault (reference for the daemon's networking and packaging), public repository
  `brianm998/ReelVault` at commit `228e3c8b7937291b1d53314eb3df92132f6757d6`:
  `core/src/discovery.rs`, `core/src/identity.rs`, `core/src/pairing.rs`, `core/src/auth.rs`,
  `core/src/main.rs`, `core/dist/`, `release-core.sh`, `.github/workflows/release.yml`.
  URL form: `https://github.com/brianm998/ReelVault/blob/228e3c8b7937291b1d53314eb3df92132f6757d6/<path>`.
- star (the ffmpeg recipe), public repository `brianm998/starry_timelapse_airplane_remover` at
  commit `ae7ae6f2c2687c3c892832ce0c241a4e41d8f0d1`: `ffmpeg/` and
  `.github/workflows/ffmpeg-gpu.yml`. URL form:
  `https://raw.githubusercontent.com/brianm998/starry_timelapse_airplane_remover/ae7ae6f2c2687c3c892832ce0c241a4e41d8f0d1/<path>`.
- When the local checkouts `~/git/reelvault` and `~/git/nighttime_timelapse_airplane_remover`
  exist, `git -C <checkout> show <sha>:<path>` gives the same files offline.

## 3. Behaviour that must be preserved

`trd` behaves like the Swift rewrite in all of these, except where §6 says otherwise:

| Behaviour | Swift source |
|---|---|
| `config.json` keys and defaults, `$YEAR` expansion, config reload on mtime change, invalid JSON keeps the last good config | `Config.swift`, `PathExpander.swift`, `RenderDaemon.swift` (`reloadConfigIfChanged`) |
| `video_codecs.json` next to the config file, else the built-in table (copied verbatim, including its profile and pixel-format values) | `Config.swift` (`codecsPath`, `defaultCodecs`) |
| raw sequence counting in `source_paths`, backup sidecar counts in `backup_paths` (`^([^.]+)\.json$`, `ImageSize` first key, else `number_source_images`, zero counts skipped) | `Scanner.swift` |
| export discovery: dirname contains `image_sequence_dirname_prefix` + raw name; `ignore_sequence_regexes`; frame counting by `image_extensions`; complete when counts are equal | `Scanner.swift` |
| EXIF histogram sidecar `<raw parent>/<raw name>.json`, `{tag: {value: count}}`; single-valued tags passed as `-metadata` | `ExiftoolRunner.swift`, `RenderDaemon.swift` |
| output file name `<name>_<codec>-<pixfmt>_Rec.709F_<res>_<fps>_<quality>.mov` with prefixes stripped; `<res>` = `<H>p`, `<W>w`, or `OriRes` | `RenderDaemon.swift` (`outputVideoFilename`) |
| already-rendered check across every `master_videos` and `export_paths` entry | `RenderDaemon.swift` (`shouldRender`) |
| frame pattern detection (prefix, digit width, suffix, start index; `-start_number` only from index 5) | `ImageSequencePattern.swift` |
| width-only resolution: height from the source aspect, rounded and made even | `RenderDaemon.swift` (`queueRender`) |
| ffmpeg arguments (crop/scale/setsar filter, codec, pix_fmt, threads, profile, extra args, `+write_colr`, `-an`, colour tags, `-timecode 00:00:00:00`, sorted `-metadata`) | `FFmpegRunner.swift` |
| `max_processes.ffmpeg` as the global running cap; `max_cuncurrent_renders` as the per-sequence running cap | `RenderDaemon.swift` (`promoteQueuedRenders`) |
| XMP-EXIF embedding with exiftool: the field mapping, mode per field, earliest `DateTimeOriginal` | `XMPEmbedder.swift` |
| corrupt-render detection from exiftool's write errors; delete and re-queue; auto-pause after 3 consecutive corrupt renders | `XMPEmbedder.swift`, `RenderDaemon.swift` |
| failure diagnosis: banner stripping, signatures, input-sequence analysis, messages, occurrence merging | `RenderFailure.swift`, `RenderFailureDiagnosis.swift` |
| `delete_sequence_after_render_regexes` | `RenderDaemon.swift` (`markRenderFinished`) |
| pause and resume of sequences and variants; suspend, resume and stop of renders | `RenderDaemon.swift` |
| adoption of ffmpeg renders left running by an earlier daemon | `OrphanScanner.swift` |
| render history kept `recent_render_history_days` (default 30) | `RenderDaemon.swift` |

Deliberate differences (each is specified in §6):

1. Original-resolution renders get their aspect ratio from the real frame size (§6.5).
2. ffmpeg writes to a hidden partial file that is renamed on success (§6.6).
3. ffmpeg's output goes to a log file and its progress to a `-progress` file, so renders survive
   `trd` restarts and are re-adopted with full progress (§6.6, §6.11).
4. When several raw sequence names match an export directory, the longest name wins (§6.3); the
   Swift choice depended on dictionary order.
5. exiftool reads up to 64 raw frames per process, `max_processes.exiftool` processes at a time
   (§6.4); the Swift code ran one process per frame, 8 at a time, ignoring the setting.
6. Pauses requested through the API are written to `config.json` at once (§6.8).
7. A sequence is deleted by `delete_sequence_after_render_regexes` only when every variant's
   output exists (§6.6); the Swift code could delete it while a variant was paused or failed.
8. If the sidecar cannot be written next to the raw sequence, the histogram is kept in the
   database and rendering proceeds (§6.4); the Swift code never rendered such a sequence.
9. Binary EXIF tags are skipped by their real `-csv` names and by any value starting with
   `(Binary data` (§6.4).

## 4. Workspace layout and dependencies

```
v3/trd/
  Cargo.toml                 workspace; [workspace.package] version = contents of v3/VERSION
  Cargo.lock                 committed
  rust-toolchain.toml        channel = "1.96.0", components rustfmt + clippy
  Makefile                   run, develop, develop-daemon, release, clean, test, check, ci
  scripts/ci.sh              what v3-gate.yml's trd job runs
  scripts/ci-media.sh        real-ffmpeg/exiftool tests (TRD-GATE)
  crates/trd-proto/          build.rs + generated code for v3/proto/trd/v1/trd.proto
  crates/trd-core/           all daemon logic (no networking)
  crates/trd/                the trd and trdw binaries: CLI, gRPC server, TLS, pairing, mDNS, service install
  crates/trd-testtools/      fake-ffmpeg, fake-ffprobe, fake-exiftool test binaries
  packaging/macos/ linux/ windows/
  sandbox/sandbox.py         the `make develop` sandbox (TRD-28)
```

Module files (created empty, with a doc comment, by TRD-01 so later tickets never edit `lib.rs`):

- `trd-core/src/`: `lib.rs`, `config.rs`, `codecs.rs`, `paths.rs`, `dirs.rs`, `logging.rs`,
  `store/mod.rs`, `store/schema.rs`, `tools.rs`, `scan.rs`, `plan.rs`, `pattern.rs`, `aspect.rs`,
  `process.rs`, `progress.rs`, `exif.rs`, `xmp.rs`, `diagnosis.rs`, `queue.rs`, `pipeline.rs`,
  `controls.rs`, `adopt.rs`, `thumbs.rs`, `history_import.rs`, `fda.rs`, `engine.rs`,
  `snapshot.rs`, `problems.rs`, `events.rs`.
- `trd/src/`: `main.rs`, `cli.rs`, `server.rs`, `auth.rs`, `tls.rs`, `pairing.rs`, `mdns.rs`,
  `status_view.rs`, `service/mod.rs`, `service/macos.rs`, `service/linux.rs`,
  `service/windows.rs`, `bin/trdw.rs`.

Dependencies (all declared by TRD-01 in `[workspace.dependencies]`; newest stable of the named
major version unless a version is given):

| Purpose | Crates |
|---|---|
| async | `tokio` 1 (`full`), `tokio-stream`, `futures` |
| gRPC | `tonic` 0.14 with `tonic-prost`, `prost` 0.14; build: `tonic-prost-build` 0.14, `protoc-bin-vendored` 3 (no system `protoc`). If a newer tonic minor exists, use it with its matching prost crates. |
| TLS | `rustls` 0.23 with the `ring` provider only (no `aws-lc-rs`, which needs CMake and NASM on Windows), `rcgen` 0.13 (`ring`), `sha2`, `hex` |
| mDNS | `mdns-sd` |
| storage | `rusqlite` (`bundled`), `rusqlite_migration` |
| data | `serde`, `serde_json`, `csv`, `regex`, `uuid` (v4), `chrono` |
| CLI and logs | `clap` 4 (`derive`), `tracing`, `tracing-subscriber`, `tracing-appender`, `crossterm` |
| processes | `sysinfo`, `nix` (unix: `signal`, `process`), `windows-sys` (Windows: threading, process, JobObjects), `libc` |
| misc | `dirs`, `rand` (`OsRng` for pairing codes and tokens), `anyhow`, `thiserror` |
| dev | `tempfile`, `assert_matches` |

`trd-core` must not depend on tonic, rustls or mdns-sd. Release profile: `opt-level = 3`,
`lto = "thin"`, `codegen-units = 1`, `strip = true`.

## 5. Command line

`trd` with no arguments prints help. Subcommands:

| Command | Effect |
|---|---|
| `trd daemon [flags]` | run the daemon (what the LaunchAgent, unit and task run) |
| `trd status [--watch] [--plain]` | print the Activity view as text; `--watch` redraws twice a second on the alternate screen until Ctrl-C |
| `trd pause <sequence> [--variant <key>]` / `trd resume <sequence> [--variant <key>]` | pause or resume a sequence or variant |
| `trd render suspend|resume|stop <render-id>` | per-render controls |
| `trd retry <sequence> <variant-key>` | retry a variant |
| `trd pause-all` / `trd resume-all` | global pause |
| `trd rescan` | scan now |
| `trd failures [--dismiss <id> | --dismiss-all]` | list or dismiss failures |
| `trd config show|path|validate [FILE]|set-path <PATH>` | show the effective config, its path, validate a file offline, or change the path |
| `trd pair [--json]` | issue a pairing code (loopback) and print it with its expiry, server name, LAN addresses, port and the first 16 hex digits of the fingerprint; `--json` prints one object `{"code", "expires_at_ms", "server_name", "lan_addresses", "lan_port", "fingerprint", "fingerprint_prefix"}` for scripts |
| `trd devices [--revoke <id>]` | list or revoke paired devices |
| `trd log [-f] [--level info|warn|error] [--limit N]` | print the log, newest last; `-f` follows |
| `trd scan --once [--config PATH]` | offline dry run without a daemon: print raw sequences, export sequences with counts, and for complete ones the output files that would be rendered |
| `trd service install|uninstall|status|restart [--dry-run] [--config PATH] [--binary PATH]` | manage the per-user autostart (§10) |
| `trd version` | print `trd <version> (<platform>)` |

Client commands connect to `--address` (default `127.0.0.1:50071`, plaintext) and exit 2 with
"trd is not running on <address>" when nothing answers.

`trd daemon` flags:

| Flag | Default | Meaning |
|---|---|---|
| `--config PATH` | stored setting, else the default path (§6.13) | config file; overrides the stored setting for this run |
| `--codecs PATH` | `video_codecs.json` beside the config, else built-in | codec table |
| `--data-dir PATH` | §6.13 | database, identity, thumbnails, render logs |
| `--log-dir PATH` | §6.13 | `trd.log` |
| `--port N` | `50071` | loopback port; `0` = OS-assigned |
| `--lan-bind ADDR:PORT` or `off` | `0.0.0.0:50072` | LAN TLS listener; tests use `127.0.0.1:0` so no firewall prompt appears |
| `--advertise-name NAME` | host name | mDNS instance and `ServerInfo.name` |
| `--no-mdns` | off | do not advertise |
| `--no-adopt` | off | skip §6.11 entirely: neither re-adopt its own renders from earlier runs nor adopt foreign ones |
| `--no-adopt-foreign` | off | re-adopt only its own renders recorded in its database (§6.11 step 1); never adopt, watch or signal a process it did not start (§6.11 step 2 skipped) |
| `--no-fda-check` | off | skip the macOS Full Disk Access guard |
| `--sandbox` | off | implies `--no-adopt-foreign --no-fda-check`; sets `ServerInfo.sandbox` |
| `--log-level debug|info` | `info` | |

When both listeners are bound, `trd daemon` prints exactly one line to stdout and flushes it:
`TRD_LISTENING loopback=127.0.0.1:<port> lan=<addr>:<port>` (`lan=off` when disabled). Tests and
the clients' `develop` targets parse it.

`trdw` (Windows) is the same program built with the Windows subsystem; it runs `trd daemon`
with the given flags and has no console.

## 6. Behaviour (normative)

### 6.1 Configuration

- Path resolution: `--config` flag; else the `config_path` setting in the database (set by
  `SetConfigPath` or `trd config set-path`); else the default path (§6.13).
- A missing file is not an error: phase `PHASE_NO_CONFIG`, problem `PROBLEM_CONFIG_MISSING`,
  nothing is scanned. `SetConfigPath` with `create_if_missing` writes a config with all defaults.
- Keys, JSON types and defaults (Swift `DaemonConfig`):

| JSON key | Type | Default |
|---|---|---|
| `export_paths`, `source_paths`, `backup_paths`, `master_videos` | `[string]` | `[]` |
| `max_processes` | `{ "ffmpeg": int, "exiftool": int }` | `{1, 10}` (each key defaults separately) |
| `max_cuncurrent_renders` | int | `1` |
| `image_name_prefix`, `image_sequence_dirname_prefix` | string | `""` |
| `raw_types` | `[string]` | `["ARW","ORF","DNG"]` |
| `image_extensions` | `[string]` | `["tif","tiff"]` |
| `rendering` | `[{codec, frame_rate, resolution?}]` | `[]` |
| `log_in_color` | bool | `true` |
| `sleep_time_seconds` | number | `12` (read and written, never used) |
| `scan_interval_seconds` | number | `2` |
| `progress_bar_width` | int | `40` |
| `ignore_sequence_regexes`, `delete_sequence_after_render_regexes` | `[string]` | `[]` |
| `paused_sequence_names` | `[string]` | `[]` |
| `paused_variants` | `[{sequence_name, variant_key}]` | `[]` |
| `recent_render_history_days` | int | `30` |
| `render_history_path` | string | absent (used only to find a Swift history to import, §6.17) |

- The file is parsed as a JSON object. Unknown keys are kept and written back unchanged. Writing
  produces pretty-printed JSON with sorted keys, writes every known key, and replaces the file
  atomically (write a sibling temp file, fsync, rename). After a write, `trd` records the new
  mtime so it does not reload its own write.
- Reload: every scan tick compares the file's mtime and size with the last load. A file that
  does not parse keeps the previous config, logs a warning once per change, and raises
  `PROBLEM_CONFIG_INVALID` until a good version is loaded (Perl: "won't barf on invalid json").
- `$YEAR`: an entry containing `$YEAR` expands to one path per year from the current year down to
  1970, keeping only existing directories, newest first (`PathExpander.swift`). Expansions are
  cached for 60 s.
- Validation (`ConfigIssue`, `field` is a JSON pointer):
  - ERROR: a path entry that is not absolute; a regex that does not compile; a `rendering` entry
    with an empty codec, `frame_rate` outside 1–240, or a `resolution` with more than 2 entries or
    a zero entry; `max_processes.ffmpeg`, `max_processes.exiftool` or `max_cuncurrent_renders`
    below 1; `scan_interval_seconds` below 0.5; `recent_render_history_days` below 1; a
    `rendering` codec not in the codec table (ERROR in `ValidateConfig`/`UpdateConfig`; for a
    file loaded from disk the variant is skipped and `PROBLEM_UNKNOWN_CODEC` is raised instead);
    two `rendering` entries with the same `variant_key`.
  - WARNING: a path without `$YEAR` that does not exist; a `$YEAR` path that expands to nothing;
    an empty `export_paths` or `source_paths`; an empty `rendering`.
  - On load from disk, an entry with an ERROR (e.g. a bad regex) is ignored and
    `PROBLEM_CONFIG_ISSUES` is raised; the rest of the config is used.

### 6.2 Codec table

`video_codecs.json` maps a codec name to `{ffmpeg: {codec, pixel_format, color_range, profile?,
extra_args?}, filename: {pixel_format, codec, quality}}`. It is read from `--codecs`, else
`video_codecs.json` beside the config file, else the built-in table, which is a verbatim copy of
`ConfigLoader.defaultCodecs` in `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/Config.swift`
(`ProResUHQ`, `ProResMQ`, `h264`). `UpdateCodecs` writes `video_codecs.json` beside the config
file (creating it when the built-in table was in use) with the same atomic write. A codec that
a `rendering` entry uses cannot be removed or renamed (ERROR). The codec table reloads on mtime
change like the config.

### 6.3 Scanning

Each tick (every `scan_interval_seconds`, or at once on `RescanNow`, a control, or a config
change):

1. **Raw sequences.** For each expanded `source_paths` entry, list its subdirectories (not
   hidden); for each, count regular files whose extension, case-insensitively, is in
   `raw_types`. A subdirectory is re-listed only when its mtime changed, or every 10 minutes.
2. **Backup sidecars.** Each expanded `backup_paths` entry is read in the background at start
   and then every 10 minutes: every file matching `^([^.]+)\.json$` gives a raw sequence named
   by the capture, with the count from the `ImageSize` object's first key in sorted order, else
   `number_source_images`; zero counts are skipped. A raw sequence found in a source path
   overrides a backup entry with the same name.
3. **Export sequences.** For each expanded `export_paths` entry, list its subdirectories (not
   hidden); skip any whose name matches an `ignore_sequence_regexes` entry; find the raw
   sequences whose `image_sequence_dirname_prefix + name` is a substring of the directory name;
   if several match, the longest raw name wins, then a source-path entry over a backup entry,
   then the lexicographically smallest parent path. Directories matching no raw sequence are
   ignored. Count regular files (not hidden) whose extension is in `image_extensions`
   (case-insensitive). A sequence directory is re-counted only when its mtime changed, or every
   60 s.
4. Each path is listed in a blocking task with a 10 s timeout. A path that fails or times out
   raises `PROBLEM_PATH_UNREACHABLE` (warning) for that path and contributes nothing this tick;
   a path whose previous listing has not returned is not listed again until it does. One slow
   path never delays the others.
5. Sequences with 0 exported frames are ignored (as the Perl and Swift daemons did).
6. Export rate: per sequence, keep `(time, count)` samples from the last 5 minutes;
   `export_frames_per_minute` = growth from the oldest sample to now; unknown (0) with fewer than
   two samples or no growth; `export_eta_seconds` = remaining frames ÷ rate, else -1.
7. Export size: the recursive size of each in-progress sequence directory, measured in the
   background at most every 60 s per sequence, at most 2 measurements at once; -1 until measured.

### 6.4 EXIF collection and sidecars

- A complete sequence needs its histogram. The sidecar is `<raw parent>/<raw name>.json`. For a
  raw sequence that came from a backup sidecar, that sidecar is the histogram.
- When it is missing (and not in the `exif_cache` table), an EXIF job runs: list the raw files
  of the raw directory (extensions in `raw_types`, case-insensitive, sorted by name); split them
  into batches of at most 64; run up to `max_processes.exiftool` batches at once, each as
  `exiftool -csv -@ <argfile>` (one path per line in a temporary argfile); parse stdout as CSV
  (header row, then one row per file); for every non-empty cell add one to
  `histogram[column][value]`, skipping the columns `ThumbnailImage`, `PreviewImage`,
  `TiffMeteringImage`, `Thumbnail Image`, `Preview Image`, `Tiff Metering Image` and any value
  that starts with `(Binary data`. `SourceFile` is kept (the Swift sidecars contain it).
- Write the sidecar as pretty-printed JSON with sorted keys, atomically. If the write fails,
  store the histogram in `exif_cache` keyed by the raw directory, log a warning, and continue.
- The card shows `STAGE_COLLECTING_METADATA` while the job runs.
- ffmpeg metadata: every histogram tag with exactly one observed value becomes
  `-metadata <tag>=<value>`, sorted by tag (`ExifAccumulator.collapseToSingleValues`).

### 6.5 Render planning

- `variant_key` = `<codec>_<res>_<frame_rate>` where `<res>` is `<W>x<H>` for two numbers, `<W>w`
  for one, `ori` for none (`RenderType.variantKey`). `resolution_label` is `<H>p`, `<W>w` or
  `OriRes`.
- Output file name (`outputVideoFilename`): start from the export directory name; while it
  contains `image_sequence_dirname_prefix` (when non-empty), remove it; then the same for
  `image_name_prefix`; append
  `_<filename.codec>-<filename.pixel_format>_Rec.709F_<resolution_label>_<frame_rate>_<filename.quality>.mov`.
  For an unknown codec the name is `<name>_<codec>_<resolution_label>_<frame_rate>.mov`.
- Already rendered: the output file name exists in any expanded `master_videos` or
  `export_paths` directory, or a render for that output is queued, running or suspended.
- Frame pattern: `ImageSequencePattern.detect` ported exactly (group by prefix, digit width and
  suffix; the group with the configured `image_name_prefix` wins, else the largest; ties by the
  smallest example name). No pattern → failure `FAILURE_NOT_QUEUEABLE` with the
  `analyzeInputSequence` explanation. A pattern that differs from the configured prefix or 5
  digits is logged once per sequence.
- Source size: `ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of
  default=noprint_wrappers=1 <first frame>`. Needed for width-only and original-size variants.
  If it fails → `FAILURE_NOT_QUEUEABLE` "could not read the frame size of <file>".
- Final size: `[W,H]` → W×H; `[W]` → W × round(W·srcH/srcW), plus one if odd; none → source size.
- Aspect: `AspectRatio.string(width:height:)` ported exactly, applied to the final size (for
  original-size renders, the source size — this is difference 1 in §3).
- Arguments, in this order:
  ```
  -nostdin -hide_banner -nostats -y
  -progress <data>/renders/<render-id>.progress -stats_period 0.5
  -r <frame_rate>
  [-start_number <first index>]                       (first index ≥ 5)
  -i <export path>/<dir>/<prefix>%0<digits>d<suffix>
  -aspect <aspect>
  [-filter_complex crop=floor(iw/2)*2:floor(ih/2)*2,scale=<W>:<H>:in_range=full:out_range=full,setsar=1/1]
                                                      (when a resolution is configured)
  -c:v <ffmpeg.codec> -pix_fmt <ffmpeg.pixel_format> -threads 0
  [-profile:v <ffmpeg.profile>]
  <ffmpeg.extra_args…>
  -movflags +write_colr -an
  -color_range <ffmpeg.color_range> -color_primaries bt709 -colorspace bt709 -color_trc bt709
  -timecode 00:00:00:00
  -metadata <tag>=<value> …                           (sorted by tag)
  <export path>/.<output stem>.partial.mov
  ```
  `<output stem>` is the output file name without `.mov`. Arguments are passed as an argv vector,
  never through a shell.

### 6.6 Running a render

1. Insert a `render_jobs` row (§6.13) with the plan and status `queued`.
2. Promotion (§6.7) starts ffmpeg: stdin null; stdout and stderr appended to
   `<data>/renders/<render-id>.log`; in its own session and process group (`pre_exec` calling `setsid` on
   unix, so a render that is stopped when the daemon dies is not SIGHUPped as an orphaned group; on Windows `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`,
   retried without `CREATE_BREAKAWAY_FROM_JOB` if that fails with access denied). Record the pid
   and the process start time (from `sysinfo`) in the row; status `running`.
3. Progress: read the `-progress` file every 500 ms; the latest complete block (ending in
   `progress=continue` or `progress=end`) gives `frame` and `fps`;
   `eta_seconds` = (total − frame) ÷ fps when fps > 0, else -1.
4. On exit: success needs exit code 0 and a non-empty partial file. Otherwise diagnose (§6.9)
   from the last 200 log lines, delete the partial file, record the failure, status `failed`.
5. On success, embed XMP into the partial file (TRD-10, the behaviour of the Swift
   `XMPEmbedder`):
   - `succeeded` → `xmp_embedded = true`;
   - `skipped(reason)` → log at info; `xmp_embedded = false`;
   - `structuralCorruption(reason)` → delete the partial file, increment the variant's corrupt
     strikes; at 3, add the variant to `paused_variants` (persisted) and set
     `variant_auto_paused`; record `FAILURE_STRUCTURAL_CORRUPTION`; status `failed`;
   - `toolError` → log a warning, keep the render, `xmp_embedded = false`.
6. Rename the partial file to the final name. If the final name already exists, delete the
   partial, log a warning, and treat the variant as already rendered.
7. Reset the variant's corrupt strikes, clear its failures, insert a `rendered_videos` row
   (`source = TRD`, `render_seconds` = wall time), make its thumbnail (§6.16), emit
   `EVENT_RENDER_FINISHED`; when it was the last missing variant of the sequence, emit
   `EVENT_SEQUENCE_FINISHED` with `video_count` and `total_bytes` of the videos `trd` rendered for
   that sequence since it became complete.
8. When the export directory path (`<export path>/<dir>`) matches a
   `delete_sequence_after_render_regexes` entry and every variant of the sequence that does not
   have an unknown codec now has its output in an export or `master_videos` directory, delete
   the export directory recursively and emit `EVENT_SEQUENCE_DELETED`.
9. Delete the progress file; keep the render log. Keep the logs of the newest 200 render jobs and
   delete older ones.

Stale partial files: at start-up and every 10 minutes, a `.*.partial.mov` file in an export
directory that no queued, running or suspended job owns is deleted.

### 6.7 Queue and concurrency

- A complete sequence that is not paused, has its histogram and is not blocked gets a queued job
  for every variant that is not already rendered, not paused, and has a known codec.
- Promotion runs after every scan and whenever a job ends, and is skipped entirely while global
  pause is on. Queued jobs are considered in order of their sequence's `first_seen_ms`, then the
  variant's position in `rendering`. A job starts if the number of running jobs (suspended jobs
  count) is below `max_processes.ffmpeg` and the number of running jobs of its sequence is below
  `max_cuncurrent_renders`.
- A queued job whose variant is no longer wanted (config changed, output appeared, sequence
  paused or gone) is removed.

### 6.8 Controls

As in overview §6.9, with these implementation rules:

- Pause and resume of sequences and variants edit `paused_sequence_names` and `paused_variants`
  in `config.json` with the atomic write of §6.1 (the config's other content is preserved).
- Ending a render sends SIGTERM (Windows: `TerminateProcess`); a process that has not exited 10 s
  later gets SIGKILL. A suspended process is resumed (SIGCONT) before SIGTERM.
- Suspend and resume use SIGSTOP/SIGCONT on unix and `NtSuspendProcess`/`NtResumeProcess` on
  Windows (looked up with `GetProcAddress` on `ntdll.dll`). The job row records
  `suspended_by = user | global`.
- Global pause is stored in the `settings` table (`global_pause`); it survives restarts. Turning
  it on suspends every running job with `suspended_by = global`; turning it off resumes exactly
  those.
- A control that does not apply (e.g. resume a running job) fails with `FAILED_PRECONDITION`;
  an unknown id with `NOT_FOUND`.
- Every control writes an INFO log entry naming the action and target.

### 6.9 Failures

- Port `RenderFailure.Kind` (labels unchanged), `RenderFailureDiagnosis` (`significantLines`,
  `decompose`, `analyzeInputSequence`, `describe`, `diagnose`) and the merging rule: a failure
  with the same output path and kind as a stored one increments `occurrences` and updates the
  other fields; the id stays.
- Failures are stored in the `failures` table. They are removed by Dismiss (until the same
  failure happens again, which creates a new card), by a successful render of the same output,
  and after 30 days.
- A failed variant is not re-queued by the next scan; Retry re-queues it. (The Swift daemon
  re-queued failed variants on every tick, which only produced repeats.)
- The first 3 occurrences of a failure are logged at ERROR with the full report; after that every
  25th.

### 6.10 Snapshots

The engine owns the state and publishes a new `Snapshot` (revision + 1) on every change. The
`in_progress` order is:

| Rank | Stage | Order within rank |
|---|---|---|
| 0 | `STAGE_RENDERING` | highest variant progress first, then name |
| 1 | `STAGE_QUEUED`, `STAGE_COLLECTING_METADATA` | `first_seen_ms` ascending |
| 2 | `STAGE_EXPORTING` | `exported_frames / expected_frames` descending, then name |
| 3 | `STAGE_COUNT_MISMATCH` | name |
| 4 | `STAGE_BLOCKED` | name |
| 5 | `STAGE_PAUSED` | name |

- A sequence leaves `in_progress` when every variant is `VARIANT_DONE` or
  `VARIANT_ALREADY_RENDERED`. Sequences that are already fully rendered when first seen never
  appear.
- `STAGE_COUNT_MISMATCH` (`exported > expected`) sets `note` to
  `"<exported> frames exported but the raw sequence has <expected>; trd renders only when the counts are equal"`.
- `recently_rendered` is the newest 30 `rendered_videos` rows; `recently_rendered_total` counts
  rows within the retention window. `file_exists` is checked at most every 60 s per row.
- `phase`: `PHASE_STARTING` until the first scan; `PHASE_NO_CONFIG`;
  `PHASE_WAITING_FOR_DISK_ACCESS`; `PHASE_PAUSED` when global pause is on; `PHASE_RENDERING` when
  a job is running; `PHASE_EXPORTING` when a sequence is exporting; else `PHASE_IDLE`.
- Problems (§6.12, §6.15, §6.1, §6.3, §8) carry stable ids: `fda`, `tool:<name>`, `config`,
  `config-issues`, `codec:<name>`, `path:<path>`, `lan`, `mdns`.

### 6.11 Re-adoption and foreign renders

At start-up, unless `--no-adopt` (`--no-adopt-foreign`, which `--sandbox` implies, skips step 2
only):

1. For every `render_jobs` row in `running` or `suspended`: the process is alive, its start time
   matches the recorded one within 2 s, and its command line contains the partial path → adopt it
   (status unchanged, progress from its progress file). Otherwise, if the progress file's last
   block says `progress=end` and the partial file is non-empty, finish it as a success (§6.6
   steps 5–9); else record `FAILURE_PRODUCED_NO_OUTPUT` with the log tail.
2. Foreign renders: every process whose executable name is `ffmpeg` and whose arguments contain
   `-i` followed by a pattern with `%` and end with a path ending in `.mov`, `.mp4`, `.mkv`,
   `.avi` or `.m4v` that is not a `.partial.mov` (`OrphanScanner.parse`) becomes a job with
   `adopted = true`, `progress_known = false`, and no plan. While it runs, its variant row shows
   indeterminate progress. When it exits, the output's existence decides success (history
   `source = ADOPTED`, no XMP embed, a thumbnail) or `FAILURE_PRODUCED_NO_OUTPUT`.
3. Adopted processes are never signalled except by an explicit control.

### 6.12 macOS Full Disk Access guard

On macOS, unless `--no-fda-check`: before the first scan and then every 30 s, try to open
`~/Library/Application Support/com.apple.TCC/TCC.db` read-only. `EPERM`/`EACCES` → Full Disk
Access is missing: phase `PHASE_WAITING_FOR_DISK_ACCESS`, problem `PROBLEM_FULL_DISK_ACCESS_MISSING`
(ERROR) with title "Full Disk Access is needed", detail "trd reads and writes your export, source,
backup and video folders, which macOS protects. Until access is granted trd scans nothing, so
macOS never asks for permission while you are away.", fix "Open System Settings → Privacy &
Security → Full Disk Access, add /Applications/Timelapse Render Daemon.app (the + button), and
turn it on. trd checks again every 30 seconds." No configured path is touched, no job is
promoted and no EXIF job starts while access is missing. Already-running renders continue.
Opening `TCC.db` never shows a prompt. `ENOENT` counts as granted. On other platforms the guard
does nothing.

### 6.13 Files, directories and the database

- Paths per OS: overview §4. `--data-dir` and `--log-dir` override them. The data directory holds
  `trd.sqlite3`, `identity/` (§8.1), `thumbnails/`, `renders/`.
- Logging: `tracing` to `trd.log` with daily rotation keeping 14 files; also to stderr when
  stderr is a terminal. In daemon mode the process's own stderr is redirected to
  `trd-stderr.log` in the log directory at start-up, so a panic message is kept.
- SQLite with WAL mode and `rusqlite_migration` migrations. Tables:

| Table | Columns |
|---|---|
| `settings` | `key TEXT PRIMARY KEY`, `value TEXT NOT NULL` — keys: `server_id`, `config_path`, `global_pause`, `swift_history_imported` |
| `render_jobs` | `id TEXT PK`, `sequence_name`, `export_path`, `variant_key`, `output_filename`, `output_path`, `partial_path`, `plan_json`, `status` (`queued|running|suspended|done|failed|cancelled`), `suspended_by`, `pid`, `pid_start_time`, `queued_at`, `started_at`, `finished_at`, `progress_path`, `log_path`, `adopted INTEGER`, `foreign INTEGER` |
| `rendered_videos` | `id TEXT PK`, `rendered_at`, `sequence_name`, `export_path`, `output_filename`, `output_path`, `variant_key`, `codec`, `resolution_label`, `frame_rate`, `frame_count`, `size_bytes`, `width`, `height`, `duration_seconds`, `render_seconds`, `thumbnail_path`, `xmp_embedded INTEGER`, `source`; index on `rendered_at DESC` |
| `failures` | `id TEXT PK`, `output_path`, `kind`, `first_seen`, `last_seen`, `occurrences`, `sequence_name`, `export_path`, `output_filename`, `variant_key`, `summary`, `detail`, `exit_code` (nullable), `command_line`, `tool_output_json`, `variant_auto_paused INTEGER`, `render_id`; unique (`output_path`, `kind`) |
| `variant_strikes` | `sequence_name`, `variant_key`, `corrupt_strikes`; primary key (`sequence_name`, `variant_key`) |
| `exif_cache` | `raw_dir TEXT PK`, `histogram_json`, `collected_at` |
| `devices` | `id TEXT PK`, `name`, `platform`, `token_sha256 TEXT UNIQUE`, `paired_at`, `last_seen`, `revoked INTEGER` |
| `log` | `seq INTEGER PRIMARY KEY AUTOINCREMENT`, `at`, `level`, `subject`, `message`, `event`, `sequence_name`, `output_filename`, `rendered_video_id`, `failure_id`, `video_count`, `total_bytes` |

- Retention: `rendered_videos` older than `recent_render_history_days` (and their thumbnails) and
  `log` rows beyond the newest 20 000 or older than 30 days are deleted at start-up and hourly.
- Timestamps are milliseconds since the Unix epoch.

### 6.14 Log and events

Every user-visible message goes through one function that writes `tracing` and inserts a `log`
row; WARN and ERROR always, INFO for render, sequence, control and config events, DEBUG only in
`trd.log`. Events (`EventKind`) are log rows with structured fields. `WatchLog(after_seq)` first
sends stored rows with `seq > after_seq` (oldest first), then live rows.

### 6.15 Tools

Resolution per tool (`ffmpeg`, `ffprobe`, `exiftool`), first hit wins:

1. `TRD_FFMPEG_DIR` (ffmpeg and ffprobe) or `TRD_EXIFTOOL_DIR` (exiftool), if set;
2. beside the executable: `ffmpeg[.exe]`, `ffprobe[.exe]`; exiftool at
   `<exe dir>/../Resources/exiftool/exiftool` (macOS app bundle), `<exe dir>/exiftool/exiftool`
   (Linux), `<exe dir>\exiftool\exiftool.exe` (Windows);
3. the development tree: walk up from the executable to the directory containing `v3/VERSION`,
   then `v3/ffmpeg/dist/<host target>/` and `v3/exiftool/dist/unix/exiftool` or
   `v3/exiftool/dist/windows/exiftool.exe`;
4. `PATH`.

The Unix exiftool is a Perl script run as `<perl> <script> …` with `/usr/bin/perl` when it
exists, else `perl` from `PATH`. Each tool's version is read at start-up and every 5 minutes
(`ffmpeg -version`, `ffprobe -version`, `exiftool -ver`). A missing tool raises
`PROBLEM_TOOL_MISSING` naming the paths tried; a tool whose version command fails raises
`PROBLEM_TOOL_BROKEN`. Without ffmpeg or ffprobe nothing is promoted; without exiftool no EXIF job
or XMP embed runs (sequences needing a histogram wait).

### 6.16 Thumbnails

After a render (and for adopted renders that succeed), run
`ffmpeg -nostdin -hide_banner -loglevel error -y -ss <duration/2> -i <final file> -frames:v 1 -vf scale=480:-2 -q:v 3 <data>/thumbnails/<rendered-video-id>.jpg`,
at most 2 at a time. A failure is logged and leaves `has_thumbnail = false`. `GetThumbnail` for an
imported row without a thumbnail generates it on demand when the file exists.

### 6.17 Importing the Swift render history

Once (setting `swift_history_imported`), read the Swift history file — `render_history_path` from
the config if set, else `~/Library/Application Support/TimelapseRenderDaemon/render_history.json`
on macOS (the Swift default; nothing on other platforms) — and insert each entry
(`timestamp`, `sequenceName`, `exportPath`, `outputFilename`, `outputPath`, `frameCount`,
`fileSize`) as a `rendered_videos` row with `source = IMPORTED`, variant fields parsed from the
file name when possible, `render_seconds = -1`. Entries outside the retention window are skipped.

## 7. The API

`v3/proto/trd/v1/trd.proto` is exactly this file (TRD-02 writes it; changes after TRD-GATE need a
new `api_version`):

```proto
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors
//
// The trd API, version 1. Timestamps are milliseconds since the Unix epoch (UTC); sizes are bytes;
// a signed numeric field documented as "-1 = unknown" uses -1 for unknown.
syntax = "proto3";

package trd.v1;

option java_multiple_files = true;
option java_package = "com.timelapserender.trd.v1";

service TrdService {
  // Callable without a token on both listeners.
  rpc GetServerInfo(GetServerInfoRequest) returns (ServerInfo);
  rpc Pair(PairRequest) returns (PairResponse);

  // State.
  rpc GetSnapshot(GetSnapshotRequest) returns (Snapshot);
  rpc WatchSnapshots(WatchSnapshotsRequest) returns (stream Snapshot);
  rpc ListRenderedVideos(ListRenderedVideosRequest) returns (ListRenderedVideosResponse);
  rpc GetThumbnail(GetThumbnailRequest) returns (Thumbnail);
  rpc GetRenderDetails(GetRenderDetailsRequest) returns (RenderDetails);
  rpc ListLog(ListLogRequest) returns (ListLogResponse);
  rpc WatchLog(WatchLogRequest) returns (stream LogEntry);

  // Controls.
  rpc SetGlobalPause(SetGlobalPauseRequest) returns (Ack);
  rpc PauseSequence(SequenceRef) returns (Ack);
  rpc ResumeSequence(SequenceRef) returns (Ack);
  rpc PauseVariant(VariantRef) returns (Ack);
  rpc ResumeVariant(VariantRef) returns (Ack);
  rpc SuspendRender(RenderRef) returns (Ack);
  rpc ResumeRender(RenderRef) returns (Ack);
  rpc StopRender(RenderRef) returns (Ack);
  rpc RetryVariant(VariantRef) returns (Ack);
  rpc DismissFailure(FailureRef) returns (Ack);
  rpc DismissAllFailures(DismissAllFailuresRequest) returns (Ack);
  rpc RescanNow(RescanNowRequest) returns (Ack);

  // Configuration.
  rpc GetConfig(GetConfigRequest) returns (ConfigState);
  rpc ValidateConfig(ValidateConfigRequest) returns (ValidateConfigResponse);
  rpc UpdateConfig(UpdateConfigRequest) returns (ConfigState);
  rpc UpdateCodecs(UpdateCodecsRequest) returns (ConfigState);
  rpc SetConfigPath(SetConfigPathRequest) returns (ConfigState);
  rpc ListDirectory(ListDirectoryRequest) returns (ListDirectoryResponse);

  // Devices: loopback listener only (PERMISSION_DENIED on the LAN listener).
  rpc StartPairing(StartPairingRequest) returns (StartPairingResponse);
  rpc ListPairedDevices(ListPairedDevicesRequest) returns (ListPairedDevicesResponse);
  rpc RevokeDevice(RevokeDeviceRequest) returns (Ack);
}

message Ack {}
message SequenceRef { string sequence_name = 1; }                       // export directory name
message VariantRef { string sequence_name = 1; string variant_key = 2; }
message RenderRef { string render_id = 1; }
message FailureRef { string failure_id = 1; }

// ── server info and pairing ─────────────────────────────────────────────────

message GetServerInfoRequest {}

message ToolInfo {
  string name = 1;          // "ffmpeg", "ffprobe", "exiftool"
  string path = 2;
  string version = 3;
  bool available = 4;
  string problem = 5;
}

message ServerInfo {
  string server_id = 1;                  // stable UUID
  string name = 2;                       // advertised name
  string version = 3;                    // e.g. "3.0.0"
  uint32 api_version = 4;                // 1
  string platform = 5;                   // "macos-universal" | "linux-x64" | "linux-arm64" | "windows-x64"
  string cert_fingerprint_sha256 = 6;    // lowercase hex of the TLS certificate's DER
  uint32 lan_port = 7;                   // 0 when the LAN listener is off
  bool caller_is_local = 8;              // the call came through the loopback listener
  bool caller_is_paired = 9;             // a LAN call carried a valid token
  repeated ToolInfo tools = 10;
  bool sandbox = 11;
}

enum DevicePlatform {
  DEVICE_PLATFORM_UNSPECIFIED = 0;
  DEVICE_PLATFORM_IOS = 1;
  DEVICE_PLATFORM_ANDROID = 2;
  DEVICE_PLATFORM_DESKTOP = 3;
}

message PairRequest { string code = 1; string device_name = 2; DevicePlatform platform = 3; }
message PairResponse { string token = 1; string device_id = 2; string server_id = 3; string server_name = 4; }

message StartPairingRequest {}
message StartPairingResponse {
  string code = 1;                       // 6 digits
  int64 expires_at_ms = 2;
  string server_id = 3;
  string server_name = 4;
  string cert_fingerprint_sha256 = 5;
  uint32 lan_port = 6;
  repeated string lan_addresses = 7;     // non-loopback IPv4 addresses
}

message ListPairedDevicesRequest {}
message PairedDevice {
  string device_id = 1;
  string name = 2;
  DevicePlatform platform = 3;
  int64 paired_at_ms = 4;
  int64 last_seen_ms = 5;
}
message ListPairedDevicesResponse { repeated PairedDevice devices = 1; }
message RevokeDeviceRequest { string device_id = 1; }

// ── snapshots ───────────────────────────────────────────────────────────────

message GetSnapshotRequest {}
message WatchSnapshotsRequest { uint32 max_per_second = 1; }   // 0 = 4; clamped to 1..10

enum Phase {
  PHASE_UNSPECIFIED = 0;
  PHASE_STARTING = 1;
  PHASE_NO_CONFIG = 2;
  PHASE_WAITING_FOR_DISK_ACCESS = 3;
  PHASE_IDLE = 4;
  PHASE_EXPORTING = 5;
  PHASE_RENDERING = 6;
  PHASE_PAUSED = 7;
}

message Snapshot {
  uint64 revision = 1;
  int64 generated_at_ms = 2;
  Phase phase = 3;
  bool global_pause = 4;
  repeated Problem problems = 5;                 // errors first, then by id
  repeated FailureCard failures = 6;             // newest first
  repeated SequenceCard in_progress = 7;         // display order
  repeated RenderedVideo recently_rendered = 8;  // newest first, at most 30
  uint32 recently_rendered_total = 9;
  Counts counts = 10;
  int64 last_scan_completed_ms = 11;             // 0 before the first scan
  uint32 last_scan_duration_ms = 12;
  string config_path = 13;
}

message Counts {
  uint32 rendering = 1;
  uint32 suspended = 2;
  uint32 queued = 3;
  uint32 exporting = 4;
  uint32 paused_sequences = 5;
  uint32 failures = 6;
  uint32 rendered_last_24h = 7;
}

enum Severity {
  SEVERITY_UNSPECIFIED = 0;
  SEVERITY_WARNING = 1;
  SEVERITY_ERROR = 2;
}

enum ProblemKind {
  PROBLEM_KIND_UNSPECIFIED = 0;
  PROBLEM_FULL_DISK_ACCESS_MISSING = 1;
  PROBLEM_TOOL_MISSING = 2;
  PROBLEM_TOOL_BROKEN = 3;
  PROBLEM_CONFIG_MISSING = 4;
  PROBLEM_CONFIG_INVALID = 5;
  PROBLEM_CONFIG_ISSUES = 6;
  PROBLEM_UNKNOWN_CODEC = 7;
  PROBLEM_PATH_UNREACHABLE = 8;
  PROBLEM_LAN_LISTENER_FAILED = 9;
  PROBLEM_MDNS_FAILED = 10;
}

message Problem {
  string id = 1;
  Severity severity = 2;
  ProblemKind kind = 3;
  string title = 4;
  string detail = 5;
  string fix = 6;
  repeated string paths = 7;
  int64 since_ms = 8;
}

enum Stage {
  STAGE_UNSPECIFIED = 0;
  STAGE_EXPORTING = 1;
  STAGE_COUNT_MISMATCH = 2;
  STAGE_COLLECTING_METADATA = 3;
  STAGE_QUEUED = 4;
  STAGE_RENDERING = 5;
  STAGE_BLOCKED = 6;            // complete, but every remaining variant is paused, failed or has an unknown codec
  STAGE_PAUSED = 7;             // the whole sequence is paused
}

enum VariantStatus {
  VARIANT_STATUS_UNSPECIFIED = 0;
  VARIANT_WAITING = 1;          // the sequence is not complete
  VARIANT_ALREADY_RENDERED = 2; // the output existed before trd rendered it
  VARIANT_QUEUED = 3;
  VARIANT_RUNNING = 4;
  VARIANT_SUSPENDED = 5;
  VARIANT_DONE = 6;             // rendered by trd since the sequence became complete
  VARIANT_FAILED = 7;
  VARIANT_PAUSED = 8;           // the variant or its sequence is paused (by the user or automatically)
  VARIANT_UNKNOWN_CODEC = 9;
}

message VariantRow {
  string variant_key = 1;
  string codec = 2;
  string resolution_label = 3;  // "OriRes", "720w", "2160p"
  uint32 frame_rate = 4;
  string output_filename = 5;
  string output_path = 6;
  VariantStatus status = 7;
  string render_id = 8;         // set for QUEUED, RUNNING, SUSPENDED, DONE, FAILED
  uint32 frame = 9;
  uint32 total_frames = 10;
  double fps = 11;
  int64 eta_seconds = 12;       // -1 = unknown
  bool progress_known = 13;
  bool adopted = 14;
  int64 size_bytes = 15;        // DONE / ALREADY_RENDERED: file size; RUNNING / SUSPENDED: partial size; else -1
  string message = 16;
  string failure_id = 17;       // set for FAILED
}

message SequenceCard {
  string id = 1;                       // "<export_path>/<name>"
  string name = 2;                     // export directory name
  string export_path = 3;
  string raw_sequence_name = 4;
  string raw_sequence_location = 5;    // the source_paths or backup_paths entry
  bool raw_from_backup = 6;
  uint32 expected_frames = 7;
  uint32 exported_frames = 8;
  Stage stage = 9;
  bool paused = 10;
  double export_frames_per_minute = 11;  // 0 = unknown
  int64 export_eta_seconds = 12;         // -1 = unknown
  int64 export_size_bytes = 13;          // -1 = unknown
  int64 first_seen_ms = 14;
  int64 last_frame_ms = 15;
  repeated VariantRow variants = 16;     // in the config's rendering order
  string note = 17;
}

enum RenderedSource {
  RENDERED_SOURCE_UNSPECIFIED = 0;
  RENDERED_SOURCE_TRD = 1;
  RENDERED_SOURCE_ADOPTED = 2;
  RENDERED_SOURCE_IMPORTED = 3;
}

message RenderedVideo {
  string id = 1;
  int64 rendered_at_ms = 2;
  string sequence_name = 3;
  string export_path = 4;
  string output_filename = 5;
  string output_path = 6;
  string variant_key = 7;
  string codec = 8;
  string resolution_label = 9;
  uint32 frame_rate = 10;
  uint32 frame_count = 11;
  int64 size_bytes = 12;
  uint32 width = 13;
  uint32 height = 14;
  double duration_seconds = 15;
  int64 render_seconds = 16;    // -1 = unknown
  bool has_thumbnail = 17;
  bool file_exists = 18;
  bool xmp_embedded = 19;
  RenderedSource source = 20;
}

enum FailureKind {
  FAILURE_KIND_UNSPECIFIED = 0;
  FAILURE_INPUT_FRAMES_NOT_FOUND = 1;
  FAILURE_INPUT_UNREADABLE = 2;
  FAILURE_SOURCE_FRAME_UNREADABLE = 3;
  FAILURE_FFMPEG_REJECTED_ARGUMENTS = 4;
  FAILURE_OUTPUT_NOT_WRITABLE = 5;
  FAILURE_DISK_FULL = 6;
  FAILURE_TERMINATED_BY_SIGNAL = 7;
  FAILURE_PRODUCED_NO_OUTPUT = 8;
  FAILURE_STRUCTURAL_CORRUPTION = 9;
  FAILURE_LAUNCH_FAILED = 10;
  FAILURE_NOT_QUEUEABLE = 11;
  FAILURE_UNKNOWN = 12;
}

message FailureCard {
  string id = 1;
  int64 first_seen_ms = 2;
  int64 last_seen_ms = 3;
  string sequence_name = 4;
  string export_path = 5;
  string output_filename = 6;
  string output_path = 7;
  string variant_key = 8;
  FailureKind kind = 9;
  string kind_label = 10;
  string summary = 11;
  string detail = 12;
  bool has_exit_code = 13;
  int32 exit_code = 14;
  string command_line = 15;
  repeated string tool_output = 16;     // at most 20 significant lines
  uint32 occurrences = 17;
  bool variant_auto_paused = 18;
  string render_id = 19;
}

// ── history, thumbnails, details ────────────────────────────────────────────

message ListRenderedVideosRequest {
  uint32 limit = 1;             // 0 = 100; at most 500
  string before_id = 2;         // "" = start from the newest; else rows older than this one
  string search = 3;            // case-insensitive substring of sequence_name or output_filename
}
message ListRenderedVideosResponse { repeated RenderedVideo videos = 1; bool more = 2; }

message GetThumbnailRequest { string rendered_video_id = 1; }
message Thumbnail { bytes jpeg = 1; uint32 width = 2; uint32 height = 3; }

message GetRenderDetailsRequest {
  oneof target {
    string render_id = 1;
    string rendered_video_id = 2;
    string failure_id = 3;
  }
}

message RenderDetails {
  string render_id = 1;
  string sequence_name = 2;
  string export_path = 3;
  string variant_key = 4;
  string input_pattern = 5;
  int32 start_number = 6;       // -1 when not passed
  uint32 total_frames = 7;
  string output_path = 8;
  string partial_path = 9;
  string command_line = 10;
  repeated string log_tail = 11;  // last 200 lines of the render log
  int64 started_at_ms = 12;       // 0 = not started
  int64 finished_at_ms = 13;      // 0 = not finished
  int32 pid = 14;                 // 0 = not running
  bool adopted = 15;
  VariantStatus status = 16;
}

// ── log ─────────────────────────────────────────────────────────────────────

enum LogLevel {
  LOG_LEVEL_UNSPECIFIED = 0;
  LOG_DEBUG = 1;
  LOG_INFO = 2;
  LOG_WARN = 3;
  LOG_ERROR = 4;
}

enum EventKind {
  EVENT_NONE = 0;
  EVENT_SEQUENCE_READY = 1;
  EVENT_RENDER_STARTED = 2;
  EVENT_RENDER_FINISHED = 3;
  EVENT_SEQUENCE_FINISHED = 4;
  EVENT_RENDER_FAILED = 5;
  EVENT_SEQUENCE_DELETED = 6;
  EVENT_CONFIG_RELOADED = 7;
}

message LogEntry {
  uint64 seq = 1;               // monotonic, persisted across restarts
  int64 at_ms = 2;
  LogLevel level = 3;
  string subject = 4;
  string message = 5;
  EventKind event = 6;
  string sequence_name = 7;
  string output_filename = 8;
  string rendered_video_id = 9;
  string failure_id = 10;
  uint32 video_count = 11;      // EVENT_SEQUENCE_FINISHED
  int64 total_bytes = 12;       // EVENT_SEQUENCE_FINISHED
}

message ListLogRequest {
  uint64 before_seq = 1;        // 0 = newest
  uint32 limit = 2;             // 0 = 200; at most 1000
  LogLevel min_level = 3;       // UNSPECIFIED = INFO
}
message ListLogResponse { repeated LogEntry entries = 1; bool more = 2; }   // newest first

message WatchLogRequest {
  uint64 after_seq = 1;         // stored rows with seq > after_seq first (oldest first), then live
  LogLevel min_level = 2;       // UNSPECIFIED = INFO
}

// ── controls ────────────────────────────────────────────────────────────────

message SetGlobalPauseRequest { bool paused = 1; }
message DismissAllFailuresRequest {}
message RescanNowRequest {}

// ── configuration ───────────────────────────────────────────────────────────

message GetConfigRequest {}

message MaxProcesses { uint32 ffmpeg = 1; uint32 exiftool = 2; }

message RenderVariant {
  string codec = 1;
  uint32 frame_rate = 2;
  repeated uint32 resolution = 3;   // [] original size, [W] width, [W, H] exact size
  string variant_key = 4;           // output only
  string example_filename = 5;      // output only: the name for a sequence "LRT_example"
}

message PausedVariant { string sequence_name = 1; string variant_key = 2; }

message Config {
  repeated string export_paths = 1;
  repeated string source_paths = 2;
  repeated string backup_paths = 3;
  repeated string master_videos = 4;
  MaxProcesses max_processes = 5;
  uint32 max_concurrent_renders = 6;          // JSON key "max_cuncurrent_renders"
  string image_name_prefix = 7;
  string image_sequence_dirname_prefix = 8;
  repeated string raw_types = 9;
  repeated string image_extensions = 10;
  repeated RenderVariant rendering = 11;
  bool log_in_color = 12;
  double sleep_time_seconds = 13;
  double scan_interval_seconds = 14;
  uint32 progress_bar_width = 15;
  repeated string ignore_sequence_regexes = 16;
  repeated string delete_sequence_after_render_regexes = 17;
  repeated string paused_sequence_names = 18;
  repeated PausedVariant paused_variants = 19;
  uint32 recent_render_history_days = 20;
  string render_history_path = 21;            // "" = absent
}

message CodecFfmpeg {
  string codec = 1;
  string pixel_format = 2;
  string color_range = 3;
  bool has_profile = 4;
  string profile = 5;
  repeated string extra_args = 6;
}
message CodecFilename { string pixel_format = 1; string codec = 2; string quality = 3; }
message VideoCodec { string name = 1; CodecFfmpeg ffmpeg = 2; CodecFilename filename = 3; }

enum IssueSeverity {
  ISSUE_SEVERITY_UNSPECIFIED = 0;
  ISSUE_WARNING = 1;
  ISSUE_ERROR = 2;
}
message ConfigIssue { IssueSeverity severity = 1; string field = 2; string message = 3; }

enum ConfigPathSource {
  CONFIG_PATH_SOURCE_UNSPECIFIED = 0;
  CONFIG_PATH_FLAG = 1;
  CONFIG_PATH_SETTING = 2;
  CONFIG_PATH_DEFAULT = 3;
}

message ConfigState {
  string path = 1;
  ConfigPathSource path_source = 2;
  bool exists = 3;
  Config config = 4;                 // with defaults applied
  string revision = 5;               // lowercase hex SHA-256 of the file's bytes; "" when !exists
  repeated ConfigIssue issues = 6;
  repeated VideoCodec codecs = 7;    // sorted by name
  string codecs_path = 8;            // "" = built-in table
  string codecs_revision = 9;        // SHA-256 of video_codecs.json; "" = built-in
  string raw_json = 10;
  int64 loaded_at_ms = 11;
}

message ValidateConfigRequest { Config config = 1; repeated VideoCodec codecs = 2; }   // codecs empty = current table
message ValidateConfigResponse { repeated ConfigIssue issues = 1; repeated RenderVariant rendering = 2; }
message UpdateConfigRequest { Config config = 1; string expected_revision = 2; }
message UpdateCodecsRequest { repeated VideoCodec codecs = 1; string expected_revision = 2; }
message SetConfigPathRequest { string path = 1; bool create_if_missing = 2; }

message ListDirectoryRequest {
  string path = 1;              // "" = the roots
  bool include_files = 2;
  bool show_hidden = 3;
}
message DirEntry { string name = 1; string path = 2; bool is_dir = 3; int64 size_bytes = 4; int64 modified_ms = 5; }
message ListDirectoryResponse {
  string path = 1;
  string parent = 2;            // "" at a root
  repeated DirEntry entries = 3;  // directories first, then by name
  repeated string roots = 4;    // macOS: "/", every /Volumes/* and the home directory; Linux: "/" and home; Windows: every drive and home
}
```

Server rules:

- Errors use gRPC status codes: `NOT_FOUND` (unknown id), `FAILED_PRECONDITION` (revision
  mismatch, a control that does not apply), `INVALID_ARGUMENT` (an `UpdateConfig`/`UpdateCodecs`
  with ERROR issues; the message lists them), `PERMISSION_DENIED` (a loopback-only call on the LAN
  listener), `UNAUTHENTICATED` (missing or invalid token), `RESOURCE_EXHAUSTED` (pairing attempts
  exceeded), `UNAVAILABLE` (shutting down).
- `WatchSnapshots` sends the current snapshot at once, then the latest one whenever the revision
  changed, at most `max_per_second` per second; slow consumers get the newest snapshot, never a
  backlog.
- `UpdateConfig` with an empty `expected_revision` succeeds only when the file does not exist.
- `ListDirectory` runs with a 5 s timeout (`DEADLINE_EXCEEDED`), lists directories only unless
  `include_files`, hides dot entries unless `show_hidden`.
- `GetThumbnail` returns `NOT_FOUND` when no thumbnail exists or can be made.

## 8. Networking and pairing

### 8.1 TLS identity

On first start, generate a self-signed certificate (rcgen, ECDSA P-256, valid 20 years, subject
alt names: advertised name, `<name>.local`, `localhost`, every LAN IPv4) and its key; store
`identity/cert.pem`, `identity/key.pem` (mode 0600) and `identity/fingerprint.txt` (lowercase hex
SHA-256 of the DER) in the data directory; reuse them afterwards so the fingerprint never changes.
Serialize the DER once and derive both the PEM and the fingerprint from those bytes (ReelVault
`identity.rs` explains why).

### 8.2 Listeners and authorization

- Loopback `127.0.0.1:<port>`: plaintext, every RPC allowed, `caller_is_local = true`. If the port
  is in use, log an error and retry the bind every 30 s without exiting.
- LAN `--lan-bind`: TLS with the identity above. Every RPC except `GetServerInfo` and `Pair`
  requires `authorization: Bearer <token>` metadata whose SHA-256 matches a non-revoked `devices`
  row (`UNAUTHENTICATED` otherwise); `last_seen` is updated at most once a minute. Device RPCs
  return `PERMISSION_DENIED`. A bind failure raises `PROBLEM_LAN_LISTENER_FAILED` and is retried
  every 30 s.

### 8.3 Pairing

- `StartPairing` (loopback only) issues a 6-digit code from `OsRng`, valid 5 minutes, replacing any
  previous code. Only one code is valid at a time. It also logs "pairing code issued" at INFO
  (never the code itself).
- `Pair(code)` on either listener: a correct, unexpired code creates a device (`id` UUID, 32 random
  bytes base64url as the token, stored only as SHA-256), consumes the code, and returns the token.
  Five wrong codes for the current code invalidate it (`RESOURCE_EXHAUSTED`); further attempts
  need a new code.
- `RevokeDevice` marks the device revoked; its token stops working at once, including on open
  streams (they end with `UNAUTHENTICATED`).

### 8.4 mDNS

Unless `--no-mdns`, register `_trd._tcp.local.` with the `mdns-sd` crate: instance name = the
advertised name with dots and whitespace replaced by `-`; host `<instance>.local.`; port = the
LAN port; all non-loopback IPv4 addresses (re-registered when they change, checked every 30 s);
TXT records `id` (server id), `name`, `fp` (fingerprint), `port`, `ver` (trd version), `api` (`1`).
Unregister on shutdown. A failure raises `PROBLEM_MDNS_FAILED`. No mDNS when the LAN listener is
off.

## 9. GitHub Actions

All workflows use `actions/checkout@v5`, `actions/cache@v5`, `actions/upload-artifact@v6`,
`actions/download-artifact@v7`, `Swatinem/rust-cache@v2`, and set `permissions: contents: read`
unless they publish. Runner labels: `macos-15` (arm64), `ubuntu-24.04`, `ubuntu-24.04-arm`,
`windows-2022`. Linux binaries therefore need glibc 2.39 or newer (Ubuntu 24.04, Debian 13),
which also holds for the ffmpeg build.

### 9.1 `v3-gate.yml` — the CI gate (created by TRD-00)

- Triggers: `workflow_dispatch` (mason's direct-ci landing dispatches it with no inputs on a
  `mason/ci/<batch>` branch and fast-forwards `develop` only when it is green); `push` to
  `develop` and `pull_request` to `develop` with paths `v3/**`, `.github/workflows/v3-*.yml`,
  `.github/actionlint.yaml`, `.mason/**`.
- `concurrency: group: v3-gate-${{ github.ref }}`, `cancel-in-progress` only for `pull_request`.
- Jobs (each component job first checks that its component exists and, if not, prints
  "<component> does not exist yet" and skips its remaining steps, so the gate is green before the
  component is created):

| Job | Runner(s) | Steps after checkout |
|---|---|---|
| `t0` | `ubuntu-24.04` | `bash v3/scripts/check-spdx.sh`; `bash v3/scripts/check-workflows.sh`; `bash v3/scripts/check-makefiles.sh`; `bash v3/scripts/test-scripts.sh`; `python3 -I v3/scripts/check-test-names.py --completed` |
| `trd` | matrix `macos-15`, `ubuntu-24.04`, `windows-2022` (fail-fast off) | present if `v3/trd/Cargo.toml` exists; `Swatinem/rust-cache@v2` (`workspaces: v3/trd`); `bash v3/trd/scripts/ci.sh` (installs the pinned toolchain with `rustup show`, then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `bash scripts/test-check-system-tests.sh`, `bash scripts/check-system-tests.sh`, `cargo test --workspace --locked`, all with the fake tools) |
| `trd-coverage` | `ubuntu-24.04` | present if `v3/trd/scripts/coverage.sh` exists; `Swatinem/rust-cache@v2`; `taiki-e/install-action@v2` (`tool: cargo-llvm-cov`); `bash v3/trd/scripts/coverage.sh` (fails when a crate is below its floor in `v3/trd/coverage-floors.toml`) |
| `kotlin` | matrix `macos-15`, `ubuntu-24.04`, `windows-2022` | present if `v3/clients/kotlin/settings.gradle.kts` exists; `actions/setup-java@v5` (temurin 21); `gradle/actions/setup-gradle@v5`; `bash v3/clients/kotlin/scripts/ci-desktop.sh` (spec 2; includes the Kover coverage check) |
| `android` | `ubuntu-24.04` | present if `v3/clients/kotlin/android/build.gradle.kts` exists; setup-java 21; setup-gradle; `yes \| "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install "platforms;android-36" "build-tools;36.0.0"`; `bash v3/clients/kotlin/scripts/ci-android.sh` (spec 4; includes the Kover coverage check) |
| `ios` | `macos-15` | present if `v3/ios/project.yml` exists; `maxim-lobanov/setup-xcode@v1` (`latest-stable`); `brew install xcodegen`; `bash v3/ios/scripts/ci.sh` (spec 3; includes the coverage check) |
| `contract` | matrix `{ubuntu-24.04, kotlin, v3/clients/kotlin/scripts/contract-test.sh}` and `{macos-15, ios, v3/ios/scripts/contract-test.sh}` (fail-fast off) | present if the entry's script exists; `Swatinem/rust-cache@v2` (`workspaces: v3/trd`); for `kotlin` setup-java 21 and setup-gradle, for `ios` setup-xcode `latest-stable`; `bash <script>` (the script builds `trd` and the fake tools itself; spec 2 §5.7, spec 3 §4.6) |

- `TRD-GATE` adds one job, `trd-media` (§9.6).

### 9.2 `v3-ffmpeg.yml` — ffmpeg build (TRD-22)

A port of star's `ffmpeg-gpu.yml`:

- Triggers: `workflow_call` (input `platforms`, default `"macos linux windows"`),
  `workflow_dispatch`, `push`/`pull_request` to `develop` on paths `v3/ffmpeg/**` and the workflow
  file. Concurrency group `v3-ffmpeg-${{ github.ref }}-${{ inputs.platforms }}`.
- Cache key for every target: `v3-ffmpeg-<target>-${{ hashFiles('v3/ffmpeg/versions.env', 'v3/ffmpeg/build.sh', 'v3/ffmpeg/lib/**') }}` on path `v3/ffmpeg/dist/<target>`. On a hit the build steps are skipped and only the smoke test runs.
- Jobs:
  - `macos` on `macos-15`: `brew install nasm pkgconf meson ninja cmake`;
    `./v3/ffmpeg/build.sh macos-universal`; `./v3/ffmpeg/smoke_test.sh --target macos-universal v3/ffmpeg/dist/macos-universal`;
    the x86_64 slice under Rosetta with `continue-on-error: true` (as in star); upload
    `v3-ffmpeg-macos-universal`.
  - `linux` matrix `linux-x64` on `ubuntu-24.04` and `linux-arm64` on `ubuntu-24.04-arm` (both
    always; the repository is public, so arm64 runners cost nothing): apt install
    `build-essential nasm pkg-config cmake meson ninja-build git curl xz-utils ca-certificates zlib1g-dev`;
    build; smoke test; upload `v3-ffmpeg-linux-x64` / `v3-ffmpeg-linux-arm64`.
  - `windows` on `windows-2022` with `msys2/setup-msys2@v2` (MINGW64, packages as in star), only
    on a cache miss; build; smoke test (Git Bash on a cache hit, as in star); upload
    `v3-ffmpeg-windows-x64`.
  - On failure, upload `v3/ffmpeg/.work/logs` as `v3-ffmpeg-<target>-logs`.
- Each artifact holds `ffmpeg[.exe]`, `ffprobe[.exe]`, `LICENSE-ffmpeg.txt`, `BUILDINFO.txt`.

### 9.3 `v3-exiftool.yml` — exiftool (TRD-23)

- Triggers: `workflow_call`, `workflow_dispatch`, `push`/`pull_request` to `develop` on
  `v3/exiftool/**` and the workflow file.
- Cache key `v3-exiftool-<kind>-${{ hashFiles('v3/exiftool/versions.env', 'v3/exiftool/fetch.sh') }}`.
- Jobs: `unix` on `ubuntu-24.04` (`bash v3/exiftool/fetch.sh unix v3/exiftool/dist/unix`, smoke
  test, upload `v3-exiftool-unix`); `unix-macos` on `macos-15` (download `v3-exiftool-unix`,
  smoke test with `/usr/bin/perl`); `windows` on `windows-2022` (fetch `windows`, smoke test under
  Git Bash, upload `v3-exiftool-windows`).

### 9.4 `v3-trd-build.yml` — trd packages (TRD-27)

- Trigger: `workflow_call` only, inputs `version` (string) and `is_release` (boolean); secrets
  inherited from the caller.
- Every job downloads the ffmpeg artifact for its target and the exiftool artifact for its kind
  (they were produced earlier in the same run), installs the pinned Rust toolchain and targets,
  and builds with `cargo build --release --locked`.
- `macos-universal` on `macos-15`: build `aarch64-apple-darwin` and `x86_64-apple-darwin`, join
  with `/usr/bin/lipo`; import the Developer ID certificate into a temporary keychain (the
  ReelVault `release.yml` step, extracting `SIGN_APP` = "Developer ID Application: … (<team>)" and
  `SIGN_PKG` = "Developer ID Installer: … (<team>)"); stage the App Store Connect API key; run
  `bash v3/trd/packaging/macos/build-pkg.sh --version <v> --ffmpeg-dir … --exiftool-dir … --out dist/trd`
  with `--sign-app`, `--sign-installer` and `--notarize` when the certificate is present. When
  `is_release` is true and `APPLE_CERTIFICATE_P12_BASE64` is empty, the job fails with
  "::error::macOS release builds must be signed (Full Disk Access would not persist across
  updates)". Always delete the keychain and key at the end. Upload `release-asset-trd-macos-universal`.
- `linux-x64` on `ubuntu-24.04` and `linux-arm64` on `ubuntu-24.04-arm`: `bash v3/trd/packaging/linux/build-deb.sh --version <v> --arch amd64|arm64 …` and the `.tar.gz`; then `bash v3/trd/packaging/linux/build-deb.sh --self-check <deb>` and `bash v3/trd/packaging/linux/test-deb-in-container.sh <deb>` (TRD-25); upload `release-asset-trd-linux-x64` / `-linux-arm64`.
- `windows-x64` on `windows-2022`: install NSIS (`choco install nsis --yes --no-progress` if
  `makensis` is missing, then put it on `PATH`); `bash v3/trd/packaging/windows/build-installer.sh --version <v> …`; then `bash v3/trd/packaging/windows/test-installer.sh <setup.exe>` (TRD-26); upload `release-asset-trd-windows-x64`.
- Asset names: `trd-<v>-macos-universal.pkg`, `trd_<v>_amd64.deb`, `trd_<v>_arm64.deb`,
  `trd-<v>-linux-x64.tar.gz`, `trd-<v>-linux-arm64.tar.gz`, `trd-<v>-windows-x64-setup.exe`,
  `trd-<v>-windows-x64.zip`.

### 9.5 `v3-release.yml` — the release orchestrator (TRD-27)

- Triggers: `push: tags: ['v3.*']`; `workflow_dispatch`.
- `permissions: contents: write` at workflow level (the publish job needs it).
- Jobs:
  1. `version` (`ubuntu-24.04`): `v` = the tag without its leading `v` on a tag push, else the
     content of `v3/VERSION`; on a tag push, fail unless `v` equals `v3/VERSION`; outputs
     `version`, `is_release` (true only on a tag push) and `prerelease` (true when `v` contains
     `-`).
  2. `ffmpeg`: `uses: ./.github/workflows/v3-ffmpeg.yml`.
  3. `exiftool`: `uses: ./.github/workflows/v3-exiftool.yml`.
  4. `trd`: `needs: [version, ffmpeg, exiftool]`, `uses: ./.github/workflows/v3-trd-build.yml`,
     `with: version, is_release`, `secrets: inherit`.
  5. `system-tests` (added by TRD-29, which creates the workflow it calls): `needs: [version,
     trd]` (specs 2–4 append `desktop`, `ios`, `android`), `uses:
     ./.github/workflows/v3-system-tests.yml`, `with: build: false, version: <version>`,
     `secrets: inherit` — the §9.7 system tests run against the packages built above.
  6. `publish` (`ubuntu-24.04`): `needs: [version, trd]` plus `system-tests` once TRD-29 adds it
     (specs 2–4 append `desktop`, `ios`, `android`); runs only when `is_release` and every needed
     job succeeded;
     downloads every artifact matching `release-asset-*`
     with `merge-multiple: true` into `release/`; lists them; creates the release with
     `softprops/action-gh-release@v2`: `tag_name: v<version>`, `name: Timelapse Render Daemon <version>`,
     `draft: true`, `prerelease: <prerelease>`, `generate_release_notes: true`, `files: release/*`,
     and a body with install notes (macOS: run the .pkg, then grant Full Disk Access; Linux:
     `sudo apt install ./trd_<v>_<arch>.deb` then `systemctl --user start trd`; Windows: run the
     setup; first start may ask to allow network access).
- Specs 2–4 add their jobs; they never edit the jobs above except `system-tests.needs` and
  `publish.needs`.

### 9.6 `trd-media` gate job (TRD-GATE)

Added to `v3-gate.yml`: `needs` two jobs that call the reusable workflows
(`uses: ./.github/workflows/v3-ffmpeg.yml` with `platforms: "macos linux"` and
`uses: ./.github/workflows/v3-exiftool.yml`), then a matrix of `ubuntu-24.04` (artifacts
`v3-ffmpeg-linux-x64`, `v3-exiftool-unix`) and `macos-15` (`v3-ffmpeg-macos-universal`,
`v3-exiftool-unix`): download, `chmod +x`, set `TRD_FFMPEG_DIR`, `TRD_EXIFTOOL_DIR` and
`TRD_E2E=1`, run `bash v3/trd/scripts/ci-media.sh`.

### 9.7 `v3-system-tests.yml` — system tests (TRD-29; specs 2–4 add jobs)

The tests that need real installers, service managers, multicast, packaged apps, a simulator or
an emulator (overview §15). They run only on GitHub-hosted runners and are not part of the
landing gate, because an agent cannot reproduce them on the owner's Mac.

- Triggers: `push` to `develop` with paths `v3/**` and `.github/workflows/v3-*.yml`; `schedule`
  `cron: '17 9 * * *'`; `workflow_dispatch` (input `build`, boolean, default `true`);
  `workflow_call` (inputs `build`, boolean, default `true`, and `version`, string, default `""`;
  secrets inherited).
- `concurrency: group: v3-system-tests-${{ github.ref }}`, `cancel-in-progress: false`.
- Watching: mason's `[watch.ci]` (§11.3) files a ticket when a run on `develop` goes red;
  `v3-release.yml` calls this workflow with `build: false` and publishes only when it is green.
- Jobs:
  1. `version` (`ubuntu-24.04`): outputs `version` = `inputs.version` when non-empty, else the
     content of `v3/VERSION`.
  2. Build jobs, each with `if: ${{ inputs.build != false }}` (true for push and schedule, which
     have no inputs): `ffmpeg` (`uses: ./.github/workflows/v3-ffmpeg.yml`), `exiftool`
     (`uses: ./.github/workflows/v3-exiftool.yml`), `trd-packages` (`needs: [version, ffmpeg,
     exiftool]`, `uses: ./.github/workflows/v3-trd-build.yml`, `with: version, is_release: false`,
     `secrets: inherit`). With `build: false` they are skipped and the test jobs use the
     `release-asset-*` artifacts that the calling run already uploaded.
  3. Test jobs. Each `needs: [version, trd-packages]` and has
     `if: ${{ !cancelled() && needs.version.result == 'success' && contains(fromJSON('["success","skipped"]'), needs.trd-packages.result) }}`:

| Job | Runner | Steps |
|---|---|---|
| `trd-install-macos` | `macos-15` | download `release-asset-trd-macos-universal`; `sudo installer -pkg trd-*.pkg -target /`; check that the app, its `trd`, `ffmpeg`, `ffprobe` and exiftool, and `/Library/LaunchAgents/com.timelapserender.trd.plist` exist; wait up to 60 s for `launchctl print gui/$(id -u)/com.timelapserender.trd` to report `state = running` (if `postinstall` found no console user on the runner, bootstrap the agent with `launchctl bootstrap gui/$(id -u) …` and print a warning); `trd status --plain` answers within 60 s; `codesign --verify --strict --deep` on the app; when the package is signed, `spctl --assess --type install` on it; finally boot the agent out and remove the app and plist |
| `trd-install-linux` | matrix `ubuntu-24.04` (amd64), `ubuntu-24.04-arm` (arm64) | download the matching deb; `sudo apt-get install -y ./trd_*_<arch>.deb`; check `/usr/bin/trd version`; `sudo loginctl enable-linger "$USER"`, `export XDG_RUNTIME_DIR=/run/user/$(id -u)`, wait up to 60 s for `systemctl --user is-system-running` to answer; check `systemctl --user is-enabled trd` prints `enabled` (from `--global enable`); `systemctl --user start trd`; `trd status --plain` answers within 60 s; `systemctl --user stop trd`; `sudo apt-get remove -y trd`; check `/usr/lib/trd` is gone |
| `trd-install-windows` | `windows-2022` | download the setup; run `trd-*-setup.exe /S`; wait up to 60 s for `schtasks /Query /TN TimelapseRenderDaemon` to succeed and `trd.exe status --plain` (from `%LOCALAPPDATA%\Programs\TimelapseRenderDaemon`) to answer; run the uninstaller with `/S`; check the task and the directory are gone |
| `trd-live` | matrix `macos-15`, `ubuntu-24.04`, `windows-2022` | checkout; `Swatinem/rust-cache@v2`; on Linux the linger and `XDG_RUNTIME_DIR` setup above; `CI=true TRD_SERVICE_TEST=1 cargo test -p trd --test service_live -- --ignored --test-threads=1` (TRD-21); on Linux also `CI=true TRD_MDNS_TEST=1 cargo test -p trd --test mdns_live -- --ignored` (TRD-19). `--ignored` runs exactly the `system_test!` tests (TRD-05), which fail rather than skip if either variable is missing, so a green job means they really ran |

- Specs 2–4 add their own build jobs (with the same `if`) and test jobs to this workflow:
  `desktop-packages` and `desktop-selftest`, `kotlin-mdns` (DSK-13), `ios-app-tests` (IOS-09),
  `android-emulator` (AND-06).

## 10. Packaging and autostart

### 10.1 macOS

- App bundle `Timelapse Render Daemon.app`: `Contents/Info.plist` (`CFBundleIdentifier`
  `com.timelapserender.trd`, `CFBundleName` "Timelapse Render Daemon", `CFBundleExecutable` `trd`,
  `CFBundleShortVersionString` = version, `CFBundleVersion` = version without its pre-release
  suffix, `LSUIElement` true, `LSMinimumSystemVersion` `11.0`, `CFBundleIconFile` `trd`);
  `Contents/MacOS/trd` (universal), `Contents/MacOS/ffmpeg`, `Contents/MacOS/ffprobe`;
  `Contents/Resources/exiftool/` (the Unix exiftool tree); `Contents/Resources/trd.icns`;
  `Contents/Resources/licenses/` (`LICENSE` of v3, `LICENSE-ffmpeg.txt`, `BUILDINFO.txt`,
  exiftool's licence text).
- Signing (when identities are given): sign `ffmpeg`, `ffprobe`, then the bundle, each with
  `/usr/bin/codesign --force --options runtime --timestamp --sign "$SIGN_APP"`; no entitlements;
  verify with `codesign --verify --strict --deep`. Without identities, ad-hoc sign (`--sign -`).
- LaunchAgent `com.timelapserender.trd.plist`:
  ```xml
  <key>Label</key><string>com.timelapserender.trd</string>
  <key>ProgramArguments</key>
  <array><string>/Applications/Timelapse Render Daemon.app/Contents/MacOS/trd</string><string>daemon</string></array>
  <key>AssociatedBundleIdentifiers</key><string>com.timelapserender.trd</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Standard</string>
  <key>AbandonProcessGroup</key><true/>
  ```
  `AbandonProcessGroup` keeps renders alive across `trd` restarts and upgrades (they are
  re-adopted, §6.11).
- `.pkg`: payload `/Applications/Timelapse Render Daemon.app` and
  `/Library/LaunchAgents/com.timelapserender.trd.plist`; component plist with
  `BundleIsRelocatable = false` (from `pkgbuild --analyze`, so Installer never puts the app
  elsewhere); `preinstall` boots the agent out of the console user's GUI domain
  (`launchctl bootout gui/<uid>/com.timelapserender.trd`); `postinstall` sets the plist to
  `root:wheel 644` and bootstraps it into the console user's domain
  (`launchctl bootstrap gui/<uid> /Library/LaunchAgents/com.timelapserender.trd.plist`); both
  scripts exit 0 even when there is no console user. `pkgbuild --identifier com.timelapserender.trd.pkg --version <v> --install-location /`, signed with `SIGN_PKG`; notarized with `xcrun notarytool submit --wait` (API key) and stapled with `xcrun stapler staple`; a notarization result other than `Accepted` fails the job after printing `notarytool log`.
- After the first install the owner grants Full Disk Access (overview §8); macOS may also ask once
  to allow incoming connections and local network access — while the owner is at the installer,
  because `postinstall` starts `trd` immediately.

### 10.2 Linux

- `.deb` (`Package: trd`, `Architecture: amd64|arm64`, `Depends: libc6 (>= 2.39), perl`):
  `/usr/lib/trd/trd`, `/usr/lib/trd/ffmpeg`, `/usr/lib/trd/ffprobe`, `/usr/lib/trd/exiftool/`,
  `/usr/bin/trd` → `/usr/lib/trd/trd`, `/usr/share/doc/trd/` (licences),
  `/usr/lib/systemd/user/trd.service`:
  ```ini
  [Unit]
  Description=Timelapse Render Daemon
  [Service]
  ExecStart=/usr/lib/trd/trd daemon
  Restart=on-failure
  RestartSec=10
  KillMode=process
  [Install]
  WantedBy=default.target
  ```
  `KillMode=process` keeps renders alive across restarts. `postinst` runs
  `systemctl --global enable trd.service` (it starts at every user's login) and prints "run
  `systemctl --user start trd` to start it now"; `prerm` runs `systemctl --global disable trd.service`.
  There is no linger: `trd` starts at login (D2).
- `.tar.gz`: the same `/usr/lib/trd` tree plus the unit file and an `INSTALL.txt`.

### 10.3 Windows

- NSIS installer, per user, no administrator rights (`RequestExecutionLevel user`), installs to
  `%LOCALAPPDATA%\Programs\TimelapseRenderDaemon\`: `trd.exe`, `trdw.exe`, `ffmpeg.exe`,
  `ffprobe.exe`, `exiftool\exiftool.exe` + `exiftool\exiftool_files\`, `licenses\`; adds an
  uninstaller entry under HKCU; runs `trd.exe service install` then `schtasks /Run /TN TimelapseRenderDaemon`.
  The uninstaller runs `trd.exe service uninstall` first.
- `trd service install` on Windows creates the task from an XML definition: trigger "at log on" of
  the current user, action `"<dir>\trdw.exe" daemon` where `<dir>` is the directory of the running
  `trd.exe` (or of `--binary`), run only when the user is logged on, no execution time limit,
  `DisallowStartIfOnBatteries=false`, `StopIfGoingOnBatteries=false`, restart on failure every
  minute up to 999 times; the XML file is written as UTF-16 LE with a byte-order mark (schtasks
  rejects some UTF-8 files); then `schtasks /Create /TN TimelapseRenderDaemon /XML <file> /F`.
- `.zip`: the same files and an `INSTALL.txt`.
- Windows binaries are not code-signed; SmartScreen may warn on first run (stated in the release notes).

### 10.4 `trd service`

| | macOS | Linux | Windows |
|---|---|---|---|
| install | write `~/Library/LaunchAgents/com.timelapserender.trd.plist` (the §10.1 content, with the running executable's path or `--binary`, plus `--config` when given) and `launchctl bootstrap gui/<uid>`; if `/Library/LaunchAgents/com.timelapserender.trd.plist` exists (installed by the .pkg), print that it is already installed and do nothing | write `~/.config/systemd/user/trd.service` (unless `/usr/lib/systemd/user/trd.service` exists), `systemctl --user daemon-reload`, `systemctl --user enable --now trd` | the scheduled task (§10.3) |
| uninstall | `launchctl bootout` and remove the per-user plist (never the /Library one; print the command to remove it instead) | `systemctl --user disable --now trd`, remove the per-user unit | `schtasks /Delete /TN TimelapseRenderDaemon /F` |
| status | `launchctl print gui/<uid>/com.timelapserender.trd` → running/pid | `systemctl --user show trd -p ActiveState,MainPID` | `schtasks /Query /TN TimelapseRenderDaemon /V /FO CSV` |
| restart | `launchctl kickstart -k gui/<uid>/com.timelapserender.trd` | `systemctl --user restart trd` | `schtasks /End` then `/Run` |

Every function takes the home directory and a command runner as parameters, so tests use a
temporary home and a recording runner; `--dry-run` prints the files and commands instead of
acting.

## 11. Mason bootstrap (TRD-00)

### 11.1 Files `TRD-00` adds

- `v3/README.md`: what v3 is, the reading order of the specs, and `make` usage.
- `v3/VERSION`: `3.0.0`.
- `v3/LICENSE`: the full GNU GPL version 3 text.
- `v3/Makefile`: overview §9 (dynamic component discovery).
- `v3/scripts/check-spdx.sh`: checks every tracked file under `v3/`, `.mason/` and
  `.github/workflows/v3-*.yml` with the extensions of overview §10 rule 6 for both SPDX lines,
  skipping generated files, JSON and anything under a `fixtures/` directory (test inputs, some
  deliberately without headers); prints offending files and exits 1.
- `v3/scripts/check-workflows.sh`: downloads actionlint 1.7.x (newest patch, pinned by version and
  SHA-256 per platform in the script) into `v3/.tools/` (gitignored) unless present, and runs it on
  `.github/workflows/v3-*.yml`.
- `.github/actionlint.yaml`: `self-hosted-runner: labels: [macos-15-intel, ubuntu-24.04-arm]`
  (older actionlint versions do not know these GitHub-hosted labels).
- `v3/scripts/suite.sh <suite>`: runs one mason suite (§11.3). When the suite's component does not
  exist yet it prints one passing placeholder case in the suite parser's format (for the regex
  suites `test <suite>_not_created_yet ... ok`, or `PASS <suite>_not_created_yet` for the two
  smoke suites; for junit suites it writes the results file with one passing
  `<suite>_not_created_yet` case; for xctest it prints
  `Test Case '-[Placeholder.Placeholder testNotCreatedYet]' passed (0.001 seconds).`) and exits 0.
  A component exists when this file exists: `trd-check`, `trd-test` → `v3/trd/Cargo.toml`;
  `trd-e2e` → `v3/trd/scripts/ci-media.sh`; `ffmpeg-smoke` → `v3/ffmpeg/build.sh`;
  `exiftool-smoke` → `v3/exiftool/fetch.sh`; `kotlin-test` →
  `v3/clients/kotlin/scripts/mason-desktop.sh`; `android-test` →
  `v3/clients/kotlin/scripts/mason-android.sh`; `ios-kit-test` → `v3/ios/TRMKit/Package.swift`;
  `ios-build` → `v3/ios/scripts/mason-build.sh`. The `t0-*` suites always run. The native ffmpeg
  target is `macos-x64` when `uname -m` is `x86_64` on macOS, `macos-arm64` for `arm64` on macOS,
  `linux-x64` / `linux-arm64` on Linux.
- `v3/scripts/bump-version.sh X.Y.Z[-pre]`: writes `v3/VERSION`, `v3/trd/Cargo.toml`
  `[workspace.package] version`, and `MARKETING_VERSION` in `v3/ios/project.yml` (without the
  pre-release suffix, which App Store Connect does not accept), when those files exist (the
  Kotlin build reads `v3/VERSION`); prints the commit and tag commands.
- `v3/scripts/check-makefiles.sh`: for every component of overview §9 whose directory exists,
  `make -C <dir> -n <target>` must succeed for each of `run`, `develop`, `release`, `clean`,
  `test`, `check` and `ci` (a dry run, so nothing is built or started); it also checks `v3/Makefile`
  the same way; prints each missing target and exits 1.
- `v3/scripts/check-test-names.py [--spec N]… [--ticket ID]… [--completed]` (Python 3 standard
  library, run with `python3 -I`): proves that every test the specs name exists under that name.
  It reads `v3/<N>-*.md` for N in 1–4. A ticket's text runs from its `### <ID>` heading to the
  next `---` line, `## <n>.` heading or `### ` ticket heading (overview §12). Names come from the
  `check:` clause of each `- A<n>:` line under the ticket's `## Acceptance criteria`; with
  `--spec N` (and `--completed`) also from the "Automated tests" column of that spec's test-plan
  table (the table under its `## <n>. Test plan` heading). Three kinds of name, each found in the
  files under `v3/` outside the build directories that `v3/.gitignore` lists:
  - a Rust test function: a backticked `[a-z][a-z0-9_]*` token in a list introduced by the word
    `test` or `tests` (for example "tests `missing_tool_problem`, `broken_tool_problem`"). It
    exists when a `.rs` file under `v3/trd/` has `fn <name>(` within three lines after a
    `#[test]` or `#[tokio::test…]` attribute, or a `system_test!(<SWITCH>, async fn <name>()`
    line;
  - a test class: a backticked token matching `[A-Z][A-Za-z0-9]*Tests?`, optionally followed by
    `.<member>` (only the class is checked). It exists when a `.kt` or `.swift` file under a
    directory named `test`, `androidTest`, `testFixtures` or ending in `Tests` declares `class`,
    `struct` or `object` with that name;
  - a file: each word of a backticked token that ends in `.rs`, `.kt`, `.swift`, `.sh` or `.py`
    and contains no `*`, `<` or `$` (so both `tests/adopt.rs` and the script in
    `bash v3/trd/scripts/ci.sh` count). It exists when some file's path relative to the
    repository root ends with that word (so `tests/adopt.rs` matches
    `v3/trd/crates/trd-core/tests/adopt.rs`).

  `--completed` selects every spec whose final ticket has landed, recognised by a file that
  ticket creates: spec 1 `v3/trd/scripts/ci-media.sh` (TRD-GATE), spec 2
  `.github/workflows/v3-desktop-build.yml` (DSK-13), spec 3 `.github/workflows/v3-ios-build.yml`
  (IOS-09), spec 4 `.github/workflows/v3-android-build.yml` (AND-06). It prints one line
  `MISSING <ticket ID or "test plan N">: <name> (<kind>)` per absent name and exits 1, or prints
  `TEST-NAMES-OK <number of names checked>` and exits 0 (also when nothing is selected). It
  honours `V3_ROOT`; the repository root is the parent of `V3_ROOT`.
- `v3/scripts/test-scripts.sh`: tests the scripts above, each against a temporary copy of the
  files it touches (every script honours `V3_ROOT`, the directory that plays the role of `v3/`,
  defaulting to the real one): `check-spdx.sh` flags a fixture missing a header and passes one with
  both lines, a generated file and a JSON file; `suite.sh` prints the right placeholder for each
  parser kind when a component is absent and runs the real command when it is present (using
  stub components in the temporary tree); `bump-version.sh 3.1.0-beta.2` writes `3.1.0-beta.2`,
  the Cargo version and `MARKETING_VERSION: "3.1.0"`; `check-makefiles.sh` fails for a stub
  Makefile missing `develop` and passes for a complete one; `check-workflows.sh` fails for a
  fixture workflow with a syntax error; `check-test-names.py`, given a fixture spec
  `1-fixture.md` whose one ticket `TRD-01` names a Rust test, a test class and a file in its
  acceptance criteria, whose test-plan table names another of each, and a fixture tree that
  lacks the criteria's Rust test and the table's class and file, prints exactly those three
  `MISSING` lines with `--spec 1` and exits 1, prints only the Rust test's line with
  `--ticket TRD-01`, and prints `TEST-NAMES-OK 6` once the three are added; with no marker files
  `--completed` prints `TEST-NAMES-OK 0`. It prints
  `PASS <name>` / `FAIL <name>` lines and exits 1 on any failure. Fixtures live in
  `v3/scripts/fixtures/`.
- `v3/.gitignore`: `.tools/`, `ffmpeg/.work/`, `ffmpeg/dist/`, `exiftool/.work/`,
  `exiftool/dist/`, `trd/target/`, `clients/kotlin/**/build/`, `clients/kotlin/.gradle/`,
  `ios/*.xcodeproj/`, `ios/build/`, `ios/DerivedData/`.
- `.github/ISSUE_TEMPLATE/mason-ticket.md`: the overview §11 structure, with GitHub template front
  matter `name: Mason ticket (v3)`, `about: Work for the mason pipeline`, `labels: mason, v3`.
- `.github/workflows/v3-gate.yml`: §9.1.
- `.mason/mason.toml` and `.mason/landing.toml`: §11.3 and §11.4.
- `CLAUDE.md` at the repository root: §11.5.

### 11.2 Registration file (the owner installs it; it is not committed)

`~/.config/mason/projects/trd.toml`:

```toml
# MASON working on timelapse_render_daemon (v3). Pipeline: .mason/mason.toml on develop.
path = "/Users/brian/git/timelapse_render_daemon"
base = "develop"
remote = "origin"
test = "bash v3/scripts/suite.sh trd-test 2>&1"
claude_bin = "/Users/brian/.config/mason/bin/claude"
step_timeout_s = 14400
test_timeout_s = 7200
# Every ticket names its spec file, and many name read-only references in rewrite/ and
# v3/assets/; without this those tasks would wait for one another.
hold_back_ignore = ["v3/*.md", "rewrite/**", "v3/assets/**"]

[ci]
kind = "github-actions"
repo = "brianm998/timelapse_render_daemon"
token = "<the same token line as in star.toml>"
workflow = "v3-gate.yml"
poll_s = 60
timeout_s = 7200

# Public repository: only issues labelled `mason` are imported, and each waits for approval.
[tracker]
kind = "github"
repo = "brianm998/timelapse_render_daemon"
token = "<the same token line as in star.toml>"
label = "mason"
require_label = true
import_as = "proposed"
interval_s = 60

[agent_env]
PATH = "/Users/brian/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/local/bin:/Users/brian/Library/Android/sdk/platform-tools"
JAVA_HOME = "/Applications/Android Studio.app/Contents/jbr/Contents/Home"
ANDROID_HOME = "/Users/brian/Library/Android/sdk"
```

### 11.3 `.mason/mason.toml`

```toml
schema = 1
start = "explore"
explored_entry = "develop"

[agent.light]
tier = "light"
effort = "medium"

[agent.review]
tier = "light"
effort = "low"

[step.explore]
type = "agent"
category = "exploring"
agent = "light"
next = "develop"

[step.develop]
type = "agent"
category = "developing"
agent = "light"
budget = "4h"
next = "test"

[step.test]
type = "test"
category = "testing"
suites = ["t0-diff-check", "t0-spdx", "t0-workflows", "t0-scripts", "t0-makefiles", "t0-mason",
          "trd-check", "trd-test", "trd-e2e", "ffmpeg-smoke", "exiftool-smoke",
          "kotlin-test", "android-test", "ios-kit-test", "ios-build"]
max_rounds = 3
on_red = "develop"
next = "review"

[step.review]
type = "agent"
category = "review"
agent = "review"
max_rounds = 2
on_red = "develop"
next = "land"

[step.land]
type = "landing"
category = "landing"
max_rounds = 3
on_red = "develop"

[resource.compile]
count = 3
[resource.test-suite]
count = 2
[resource.exclusive]
count = 1

[binding.t0]
when = ["development", "test_step", "batch"]
[binding.t1]
when = ["test_step", "batch"]

# develop's own CI: a red v3-gate freezes landing until it is green again; a red run of any other
# workflow on develop (v3-system-tests above all) becomes a ticket through mason's long-red rule.
[watch.ci]
enabled = true
gate = "v3-gate"
exclude = []
```

Suites (each `command` is `bash v3/scripts/suite.sh <name> 2>&1`, so the commands below are what
`suite.sh` runs once the component exists):

| Suite | Level | Runs | Parser | Uses |
|---|---|---|---|---|
| `t0-diff-check` | t0 | `git diff --check $base && echo T0-DIFF-OK`, where `base` is `git merge-base HEAD R` for the first ref `R` in `origin/develop`, `develop` that resolves; when neither resolves the suite fails (exit 1, naming `origin/develop`), with no first-parent or HEAD-only fallback | regex `^T0-DIFF-OK$` | — |
| `t0-spdx` | t0 | `bash v3/scripts/check-spdx.sh && echo T0-SPDX-OK` | regex `^T0-SPDX-OK$` | — |
| `t0-workflows` | t0 | `bash v3/scripts/check-workflows.sh && echo T0-WORKFLOWS-OK` | regex `^T0-WORKFLOWS-OK$` | — |
| `t0-scripts` | t0 | `bash v3/scripts/test-scripts.sh` | regex pass `^PASS (.+)$`, fail `^FAIL (.+)$` | — |
| `t0-makefiles` | t0 | `bash v3/scripts/check-makefiles.sh && echo T0-MAKEFILES-OK` | regex `^T0-MAKEFILES-OK$` | — |
| `t0-mason` | t0 | `mason definition check && echo T0-MASON-OK` (only on the owner's Mac, where `mason` is installed; the gate has no equivalent) | regex `^T0-MASON-OK$` | — |
| `trd-check` | t1 | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `bash scripts/test-check-system-tests.sh` and `bash scripts/check-system-tests.sh` in `v3/trd`, each printed as `test trd_fmt ... ok|FAILED` / `test trd_clippy ... ok|FAILED` / `test trd_system_test_rules_selftest ... ok|FAILED` / `test trd_system_test_rules ... ok|FAILED` | regex pass `^test (.+) \.\.\. ok$`, fail `^test (.+) \.\.\. FAILED$` | compile 1 |
| `trd-test` | t1 | `cargo test --workspace --locked` in `v3/trd` | regex pass `^test (.+) \.\.\. ok$`, fail `^test (.+) \.\.\. FAILED$` | test-suite 1 |
| `trd-e2e` | t1 | `TRD_E2E=1 bash v3/trd/scripts/ci-media.sh` with the tools resolved by §6.15 (on the Mac: PATH ffmpeg/exiftool until the bundled ones are built) | regex as `trd-test` | test-suite 1 |
| `ffmpeg-smoke` | t1 | `v3/ffmpeg/build.sh` for the host's native target, then `v3/ffmpeg/smoke_test.sh --target <target> v3/ffmpeg/dist/<target>`, printing `PASS <name>` / `FAIL <name>` lines | regex pass `^\s*PASS\s+(.+)$`, fail `^\s*FAIL\s+(.+)$` | exclusive 1 |
| `exiftool-smoke` | t1 | `bash v3/exiftool/fetch.sh unix v3/exiftool/dist/unix && bash v3/exiftool/smoke_test.sh v3/exiftool/dist/unix` | regex as `ffmpeg-smoke` | — |
| `kotlin-test` | t1 | `bash v3/clients/kotlin/scripts/mason-desktop.sh` (spec 2), which writes `v3/clients/kotlin/build/mason-desktop-results.xml` | junit, `results_file` that path | test-suite 1 |
| `android-test` | t1 | `bash v3/clients/kotlin/scripts/mason-android.sh` (spec 4), which writes `v3/clients/kotlin/build/mason-android-results.xml` | junit | test-suite 1 |
| `ios-kit-test` | t1 | `swift test --package-path v3/ios/TRMKit` | xctest | test-suite 1 |
| `ios-build` | t1 | `bash v3/ios/scripts/mason-build.sh` (spec 3): xcodegen, then `build-for-testing` of the app and its unit-test target for the simulator (compiles, runs nothing), printing `test ios_build ... ok|FAILED` | regex as `trd-test` | compile 1 |

Every suite has `min_tests = 1` until `TRD-GATE`, `DSK-13`, `IOS-09` and `AND-06` raise their
floors to 90 % of the measured counts.

Impact map:

```toml
[impact]
"v3/trd/**" = ["trd-check", "trd-test", "trd-e2e"]
"v3/proto/**" = ["trd-check", "trd-test", "kotlin-test", "android-test", "ios-kit-test", "ios-build"]
"v3/ffmpeg/**" = ["ffmpeg-smoke"]
"v3/exiftool/**" = ["exiftool-smoke", "trd-e2e"]
"v3/clients/kotlin/**" = ["kotlin-test", "android-test"]
"v3/ios/**" = ["ios-kit-test", "ios-build"]
"v3/scripts/**" = ["t0-spdx", "t0-workflows", "t0-scripts", "t0-makefiles"]
"v3/assets/**" = []
"v3/*.md" = []
"v3/instructions" = []
"v3/Makefile" = ["t0-makefiles"]
"v3/**/Makefile" = ["t0-makefiles"]
".github/**" = ["t0-workflows"]
".mason/**" = ["t0-workflows", "t0-mason"]
"CLAUDE.md" = []
```

### 11.4 `.mason/landing.toml`

```toml
mode = "direct-ci"
base = "develop"
window = "20m"
max_size = 10
parallel = 1

[[tier]]
name = "batch"
suites = ["trd-test", "trd-e2e", "kotlin-test", "android-test", "ios-kit-test"]

[on_red]
ladder = ["rerun-failed", "blame-by-impact", "bisect", "ask"]
```

### 11.5 `CLAUDE.md` (repository root)

Contents, in this order: what the repository holds (Perl daemon, `rewrite/`, `v3/`); that new work
is v3 and starts from `origin/develop`; that `v3/0-overview.md` and the specs are the plan and win
over other text; the rules of overview §10 verbatim; how to run a suite (`bash v3/scripts/suite.sh
<name>`) and each component's `make test`; ports 50071/50072 belong to the owner's installed `trd`
(tests use port 0, the sandbox 50081/50082); and, last, mason's "Filing work for the pipeline"
section verbatim:

```
## Filing work for the pipeline

If you file a task that already contains the plan (the files to change, the
approach and the acceptance criteria), mark it explored so the pipeline skips
its exploring step: pass `explored: true` to `file_proposal`, or `--explored` to
`mason task add`, or put `explored: true` in the task's front matter. Only do
this when the task body really is the plan. Run `mason help filing` for details.
```

## 12. Sandbox (`make develop`, TRD-28)

- `python3 -I v3/trd/sandbox/sandbox.py init [--dir PATH] [--reset] [--check]` creates
  `$TMPDIR/trd-sandbox-<first 8 hex of SHA-1 of the worktree path>/` (or `--dir`) with
  `config.json`, `video_codecs.json` (the built-in table),
  `raw/`, `backup/`, `export/`, `master/`, `data/`, `logs/`, using the tools resolved by §6.15:
  - raw frames are TIFFs (`raw_types: ["tif"]`) 320×213 made with ffmpeg `testsrc2`, carrying EXIF
    written by exiftool (`Make=SandboxCam`, `Model=SC-1`, `LensModel=Sandbox 14mm F1.8`, `ISO=1600`,
    `FNumber=1.8`, `ExposureTime=1/8`, `FocalLength=14.0 mm`, `DateTimeOriginal` one second apart);
  - sequence A `2026_01_01-sandbox-a`, 60 raw frames, exported gradually by `drip`;
  - sequence B `2026_01_01-sandbox-b`, 30 frames, fully exported at init (renders at once);
  - sequence C `2026_01_01-sandbox-c`, 24 frames, frame 12 of the export truncated to 100 bytes;
    real ffmpeg renders all 24 frames anyway, with no failure card, and the render's log tail
    records the decode error (the accepted real behaviour; TRD-GATE);
  - sequence D `2026_01_01-sandbox-d`, 20 frames, every output already in `master/` (never shown);
  - sequence E `2026_01_01-sandbox-e-aligned`, ignored by the regex `-aligned$`;
  - sequence F `2026_01_01-sandbox-f`: no raw directory, only `backup/2026_01_01-sandbox-f.json`
    (`ImageSize` count 16), 16 frames exported;
  - sequence G `2026_01_01-sandbox-g-merge`, 12 frames, deleted after rendering by `-merge$`.
  Export directories are named `LRT_<name>` with frames `LRT_00001.tif` …; config:
  `image_name_prefix` and `image_sequence_dirname_prefix` `LRT_`, `rendering`
  `[{ProResMQ, 30, [1080,720]}, {h264, 30, [720]}, {ProResUHQ, 30}]`, `scan_interval_seconds` 1,
  `max_processes` `{ffmpeg: 2, exiftool: 4}`.
- `sandbox.py drip [--dir PATH] [--fps 2]` copies sequence A's frames into its export directory
  at that rate until complete; `sandbox.py path [--dir PATH]` prints the sandbox directory.
- `make develop` = `sandbox.py init` (idempotent, `--reset` recreates), `cargo build`, then
  `trd daemon --sandbox --config <sb>/config.json --data-dir <sb>/data --log-dir <sb>/logs --port 50081 --lan-bind 0.0.0.0:50082 --advertise-name trd-sandbox`
  and `drip` in the background; Ctrl-C stops both. `make develop-daemon` runs only the daemon (the
  clients' `develop` targets start it). The sandbox advertises over mDNS and binds the LAN, so
  macOS may prompt the owner; that is expected because only the owner runs it.

## 13. Tickets

Each ticket below is filed as one issue (overview §12). Its last line names the IDs it depends
on; when filing, replace that line with `Depends on #<n>, #<m>` using the issues' numbers, or omit
it when it says "none". `TRD-00` is not filed.

### TRD-00 — Bootstrap v3: mason pipeline, CI gate, repo rules (done by hand, not filed)

## Goal
The repository has everything mason needs to run the v3 tickets, and a CI gate that is green
before any component exists.

## Context
Overview §2 (D10), §9, §10, §11; this spec §9.1, §11. Done in an interactive session (mason is
not yet registered for this repository), merged to `develop`, then the owner installs §11.2 and
restarts masond.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §9.1 and §11)
- `v3/README.md` (new)
- `v3/VERSION` (new)
- `v3/LICENSE` (new)
- `v3/Makefile` (new)
- `v3/.gitignore` (new)
- `v3/scripts/check-spdx.sh` (new)
- `v3/scripts/check-workflows.sh` (new)
- `v3/scripts/suite.sh` (new)
- `v3/scripts/bump-version.sh` (new)
- `v3/scripts/check-makefiles.sh` (new)
- `v3/scripts/check-test-names.py` (new)
- `v3/scripts/test-scripts.sh` (new)
- `v3/scripts/fixtures/README.md` (new)
- `.github/actionlint.yaml` (new)
- `.github/ISSUE_TEMPLATE/mason-ticket.md` (new)
- `.github/workflows/v3-gate.yml` (new)
- `.mason/mason.toml` (new)
- `.mason/landing.toml` (new)
- `CLAUDE.md` (new)

## Approach
1. Write every file of §11.1 with the content §9.1, §11.3, §11.4 and §11.5 give, including the
   fixtures `test-scripts.sh` uses under `v3/scripts/fixtures/`.
2. Run `bash v3/scripts/test-scripts.sh`, `bash v3/scripts/check-spdx.sh`,
   `bash v3/scripts/check-workflows.sh`, `bash v3/scripts/check-makefiles.sh`,
   `python3 -I v3/scripts/check-test-names.py --completed` and
   `bash v3/scripts/suite.sh <name>` for every suite.
3. Push to a branch, dispatch `v3-gate.yml` on it, merge to `develop` when green.

## Acceptance criteria
- A1: every suite in §11.3 passes with its placeholder case — check: `for s in …; do bash v3/scripts/suite.sh $s; done`.
- A2: `v3-gate.yml` is green on `develop` with every component job skipped — check: the Actions run.
- A3: `mason definition check` accepts `.mason/mason.toml` — check: `mason definition check` in the repository root.
- A4: every case in `test-scripts.sh` passes, including the deliberately broken fixtures being reported — check: `bash v3/scripts/test-scripts.sh`.
- A5: `check-makefiles.sh` passes for `v3/Makefile` — check: `bash v3/scripts/check-makefiles.sh`.
- A6: `check-test-names.py` reports exactly the missing names of its fixture spec, and `--completed` passes on the bootstrap tree — check: `bash v3/scripts/test-scripts.sh` (its `check-test-names` cases) and `python3 -I v3/scripts/check-test-names.py --completed`.

## Tests
`v3/scripts/test-scripts.sh` covers every script of §11.1 (cases listed there).

## CI
`v3-gate.yml` (new).

## Out of scope
Any Rust, Kotlin or Swift code.

Depends on: none

---

### TRD-01 — Rust workspace skeleton, Makefile and CI script

## Goal
`v3/trd` builds an empty `trd` binary on every platform, with every module file, dependency and
`make` target in place, so later tickets only fill in their own modules.

## Context
§4 (layout, modules, dependencies), §5 (CLI shape), overview §9 (Makefile contract). The
`v3-gate.yml` `trd` job runs `scripts/ci.sh` once `v3/trd/Cargo.toml` exists.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §4, §5)
- `v3/trd/Cargo.toml` (new)
- `v3/trd/Cargo.lock` (new)
- `v3/trd/rust-toolchain.toml` (new)
- `v3/trd/Makefile` (new)
- `v3/trd/scripts/ci.sh` (new)
- `v3/trd/scripts/coverage.sh` (new)
- `v3/trd/scripts/coverage.py` (new)
- `v3/trd/scripts/test_coverage.py` (new)
- `v3/trd/scripts/fixtures/llvm-cov.json` (new)
- `v3/trd/scripts/check-system-tests.sh` (new)
- `v3/trd/scripts/test-check-system-tests.sh` (new)
- `v3/trd/scripts/fixtures/system-tests/` (new; the four fixture trees of step 8)
- `v3/trd/coverage-floors.toml` (new)
- `v3/trd/crates/trd-core/Cargo.toml` (new)
- `v3/trd/crates/trd-core/src/lib.rs` (new)
- `v3/trd/crates/trd/Cargo.toml` (new)
- `v3/trd/crates/trd/src/main.rs` (new)
- `v3/trd/crates/trd/src/cli.rs` (new)
- `v3/trd/crates/trd/src/bin/trdw.rs` (new)

## Approach
1. Workspace with members `crates/trd-proto`, `crates/trd-core`, `crates/trd`,
   `crates/trd-testtools`; `[workspace.package] version` = `v3/VERSION`; every §4 dependency in
   `[workspace.dependencies]`; release profile from §4. `trd-proto` and `trd-testtools` are empty
   library crates here (filled by TRD-02 and TRD-05).
2. Create every module file listed in §4 with a one-line doc comment and declare them all in
   `lib.rs` / `main.rs`.
3. `cli.rs`: the §5 subcommand tree with clap; every subcommand except `version` prints
   "not implemented yet" and exits 1. `trd version` prints `trd <version> (<platform>)`.
4. `trdw.rs`: `#![cfg_attr(windows, windows_subsystem = "windows")]`, calls the same entry with
   `daemon` as the default subcommand.
5. Makefile targets `run`, `develop`, `develop-daemon`, `release`, `clean`, `test`, `check`, `ci`
   (until later tickets: `run` = `cargo run -- daemon`, `develop`/`develop-daemon` print "sandbox
   arrives with TRD-28" and exit 1, `release` = `cargo build --release`).
6. `scripts/ci.sh`: §9.1 `trd` row, bash 3.2, `set -euo pipefail`, works under Git Bash on Windows.
7. Coverage: `scripts/coverage.sh` first runs `python3 -I -m unittest scripts/test_coverage.py`,
   adds the `llvm-tools-preview` component, runs
   `cargo llvm-cov --workspace --locked --json --output-path target/llvm-cov.json` (fake tools, as
   `cargo test`), then `python3 -I scripts/coverage.py target/llvm-cov.json coverage-floors.toml`,
   which sums line coverage per crate from the files under `crates/<crate>/src/` (generated code
   and `trd-testtools` excluded), prints a table and exits 1 when a crate is below its floor.
   `coverage-floors.toml` starts at `trd-core = 0` and `trd = 0`; TRD-GATE raises them to
   `trd-core = 80` and `trd = 60` (overview §15.3).
   `make coverage` runs it.
8. `scripts/check-system-tests.sh` (bash 3.2; honours `TRD_ROOT`, default the `v3/trd` directory
   above the script) enforces overview §10 rules 1–2 and §15.2 for the daemon's tests by reading
   the files under `crates/*/tests/` and `crates/trd-testtools/src/`:
   - R1: every `crates/*/tests/*_live.rs` file (a system test target) declares its tests only
     with `system_test!` (TRD-05): it contains at least one `system_test!(` and no line matching
     `^[[:space:]]*#\[(tokio::)?test`;
   - R2: no other file there contains `SysinfoProcessTable`, `MdnsSdRegistrar`, `ServiceDaemon`
     or `SystemRunner` (the real process table, mDNS registration and service-manager runner of
     TRD-08, TRD-19 and TRD-21);
   - R3: every other file there that contains `"daemon"` (the argument that starts the `trd`
     daemon) also contains `"--no-mdns"` and at least one of `"--sandbox"` and `"--no-adopt"`.

   It prints `check-system-tests: <file>: R<n> <explanation>` per violation and exits 1, or
   prints `SYSTEM-TESTS-OK` and exits 0 (also while there are no test files). Unit tests inside
   `src/` are covered differently: the real implementations panic when used inside a unit test
   (`assert!(!cfg!(test), …)`, TRD-08, TRD-19, TRD-21). `scripts/test-check-system-tests.sh` copies the fixture trees under
   `scripts/fixtures/system-tests/` (`good`, `r1`, `r2`, `r3`, each holding small `.rs` files
   under `crates/x/tests/` that are never compiled) into a temporary directory, runs the checker
   on each with `TRD_ROOT`, expects `SYSTEM-TESTS-OK` for `good` and exit 1 naming the matching
   rule for the others, and prints `PASS <name>` / `FAIL <name>`. `ci.sh` runs both scripts
   before `cargo test`, and `make check` runs them too.

## Acceptance criteria
- A1: `cargo build --workspace --locked` and `cargo test --workspace --locked` succeed — check: `make -C v3/trd test`.
- A2: `trd version` prints `trd 3.0.0 (<platform>)` — check: `cargo run -q -p trd -- version`.
- A3: the gate's `trd` and `trd-coverage` jobs are green — check: `v3-gate.yml`.
- A4: `coverage.py` reports per-crate line coverage from a recorded `llvm-cov` JSON fixture and fails when a floor is above it — check: `python3 -I -m unittest v3/trd/scripts/test_coverage.py`.
- A5: `check-makefiles.sh` passes for `v3/trd` — check: `bash v3/scripts/check-makefiles.sh`.
- A6: `check-system-tests.sh` passes on the skeleton and names the broken rule for each bad fixture tree — check: `bash v3/trd/scripts/test-check-system-tests.sh` and `bash v3/trd/scripts/check-system-tests.sh`.

## Tests
`trd/tests/cli.rs`: `version` output; an unimplemented subcommand exits 1.
`v3/trd/scripts/test_coverage.py` (new): the coverage summary and floor check against a fixture.
`v3/trd/scripts/test-check-system-tests.sh` (new): the three rules against the fixture trees.

## CI
`v3-gate.yml` jobs `t0` and `trd`.

## Out of scope
Any behaviour beyond `version`.

Depends on: none

---

### TRD-02 — The trd.v1 API proto and generated Rust code

## Goal
`v3/proto/trd/v1/trd.proto` holds the §7 API and `trd-proto` compiles it to Rust client and
server code without a system `protoc`.

## Context
§7 is the exact file content. Clients in specs 2–4 generate their own code from the same file.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §7)
- `v3/proto/trd/v1/trd.proto` (new)
- `v3/proto/README.md` (new)
- `v3/trd/crates/trd-proto/Cargo.toml` (new)
- `v3/trd/crates/trd-proto/build.rs` (new)
- `v3/trd/crates/trd-proto/src/lib.rs` (new)

## Approach
1. Copy §7's proto block verbatim into `trd.proto`.
2. `build.rs`: set `PROTOC` from `protoc_bin_vendored::protoc_bin_path()`, compile with
   `tonic_prost_build` (client and server), rerun-if-changed on the proto.
3. `lib.rs`: `pub mod trd { pub mod v1 { tonic::include_proto!("trd.v1"); } }`.
4. `README.md`: the API version rule (§7) and how each client generates code.

## Acceptance criteria
- A1: the proto in the repository is byte-identical to §7's block — check: a test that extracts the block from `v3/1-trd-core-daemon.md` and compares.
- A2: `cargo build -p trd-proto` succeeds with no `protoc` on `PATH` — check: `PATH=/usr/bin:/bin cargo build -p trd-proto`.

## Tests
`trd-proto/tests/proto_matches_spec.rs` (A1); a round trip of a `Snapshot` through prost.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Server and client implementations.

Depends on: TRD-01

---

### TRD-03 — Config: config.json and video_codecs.json model, validation, reload

## Goal
`trd-core` loads, validates, edits and writes `config.json` and `video_codecs.json` exactly as
§6.1–§6.2 say, compatible with the Perl and Swift daemons' files.

## Context
§6.1, §6.2; Swift `Config.swift`, `PathExpander.swift`, `RenderDaemon.swift`
(`reloadConfigIfChanged`, `updateConfig`); real files `config.json`, `gui_config.json`,
`video_codecs.json` (note `gui_config.json` has a trailing comma and must be reported as invalid
JSON). Swift tests to port: `testConfigDecodesPerlFormat`, `testConfigDefaultsForRenderHistory`,
`testYearExpansionPreservesNonPlaceholderPaths`, `testYearExpansionExpandsExistingDirectoriesOnly`,
`testCodecsPathAlongsideConfig`, `testDefaultCodecTableIncludesH264`,
`testCodecTableDecodesProfileAndExtraArgs`, `testVariantKeyStable`, `testWidthOnlyRenderType`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.1, §6.2)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/Config.swift`
- `v3/trd/crates/trd-core/src/config.rs`
- `v3/trd/crates/trd-core/src/codecs.rs`
- `v3/trd/crates/trd-core/src/paths.rs`
- `v3/trd/crates/trd-core/tests/config.rs` (new)

## Approach
1. `Config` struct with every §6.1 key and default; parse from `serde_json::Value` keeping unknown
   keys in a `BTreeMap`; serialize sorted and pretty with every known key plus the unknown ones.
2. `ConfigStore`: path, last mtime and size, last good config, revision (SHA-256 of bytes);
   `reload_if_changed()`, `apply(edit, expected_revision)` with the atomic write, `issues()`.
3. Validation per §6.1 returning `ConfigIssue { severity, field (JSON pointer), message }`.
4. `$YEAR` expansion with an injectable clock and a 60 s cache.
5. Codec table: parse, built-in defaults (verbatim from `Config.swift`), resolution order, atomic
   write, mtime reload, the in-use rule for remove and rename.
6. `variant_key`, `resolution_label` and example file name helpers for `RenderVariant`.

## Acceptance criteria
- A1: `config.json` and `video_codecs.json` from the repository load with no ERROR issues and round-trip without losing keys — check: `cargo test -p trd-core config`.
- A2: `gui_config.json` reports `PROBLEM_CONFIG_INVALID` and keeps the previously loaded config — check: test `invalid_json_keeps_last_good`.
- A3: an unknown top-level key survives `apply` — check: test `unknown_keys_survive`.
- A4: an `UpdateConfig`-style edit with a stale revision is refused — check: test `stale_revision_refused`.
- A5: every ported Swift test passes — check: `cargo test -p trd-core`.

## Tests
`tests/config.rs` with the cases above plus every validation rule (one test per rule), `$YEAR`
with a temp tree and a fixed clock, and the codec in-use rule.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Scanning; the gRPC mapping (TRD-17).

Depends on: TRD-01

---

### TRD-04 — Directories, logging and the SQLite store

## Goal
`trd-core` knows where its files live on each OS, logs to rotating files and to the `log` table,
and has the §6.13 database with migrations.

## Context
§6.13, §6.14, overview §4 (per-OS paths).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.13, §6.14)
- `v3/trd/crates/trd-core/src/dirs.rs`
- `v3/trd/crates/trd-core/src/logging.rs`
- `v3/trd/crates/trd-core/src/store/mod.rs`
- `v3/trd/crates/trd-core/src/store/schema.rs`
- `v3/trd/crates/trd-core/tests/store.rs` (new)

## Approach
1. `Dirs { config_default, data, logs }` per OS from a home directory and environment passed in
   (never read the real home in tests), with the `--data-dir`/`--log-dir` overrides.
2. `logging::init(dirs, level, foreground)`: `tracing-appender` daily rotation keeping 14 files;
   stderr layer when stderr is a terminal; the stderr redirect for daemon mode.
3. `Store` over rusqlite: WAL, migration 1 creating every §6.13 table and index; typed accessors
   for each table; retention (`prune(now, history_days)`).
4. `log_event(level, subject, message, event fields)` writing `tracing` and a `log` row, returning
   the row; a broadcast channel for live rows.
5. `settings` accessors including `server_id` (generated once).

## Acceptance criteria
- A1: a fresh database migrates and every accessor round-trips — check: `cargo test -p trd-core store`.
- A2: `log` sequence numbers keep increasing across a close and reopen — check: test `log_seq_survives_reopen`.
- A3: retention deletes old history rows and log rows beyond the limits — check: test `retention`.
- A4: paths per OS match overview §4 for a given home — check: test `dirs_per_os`.
- A5: `log_event` writes one `tracing` record (captured by a test subscriber) and one `log` row with the same fields, and publishes the row to live subscribers — check: test `log_event_writes_both`.
- A6: `--data-dir` and `--log-dir` override the defaults — check: test `dir_overrides`.

## Tests
`tests/store.rs` (A1–A3, A5), unit tests in `dirs.rs` (A4, A6).

## CI
`v3-gate.yml` `trd`.

## Out of scope
What writes to which table (later tickets).

Depends on: TRD-01

---

### TRD-05 — Tool resolution and the fake test tools

## Goal
`trd-core` finds ffmpeg, ffprobe and exiftool as §6.15 says and reports them; the workspace has
fake tools that let every later test run without real media tools.

## Context
§6.15. The fakes replace real tools in tests through `TRD_FFMPEG_DIR` and `TRD_EXIFTOOL_DIR`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.15)
- `v3/trd/crates/trd-core/src/tools.rs`
- `v3/trd/crates/trd-testtools/Cargo.toml` (new)
- `v3/trd/crates/trd-testtools/src/lib.rs` (new)
- `v3/trd/crates/trd-testtools/src/bin/fake-ffmpeg.rs` (new)
- `v3/trd/crates/trd-testtools/src/bin/fake-ffprobe.rs` (new)
- `v3/trd/crates/trd-testtools/src/bin/fake-exiftool.rs` (new)
- `v3/trd/crates/trd-testtools/src/system_test.rs` (new)
- `v3/trd/crates/trd-testtools/tests/fakes.rs` (new)
- `v3/trd/crates/trd-testtools/tests/system_test_macro.rs` (new)
- `v3/trd/scripts/fake-tools-dir.sh` (new)

## Approach
1. `Tools::resolve(env, exe_path)` implementing the four-step order, the Perl wrapper for the Unix
   exiftool, version probing, and `ToolInfo`/problem output.
2. `fake-ffmpeg`: reads a JSON script from `TRD_FAKE_FFMPEG` (`frames`, `exit_code`, `stderr_lines`,
   `write_output`, `wait_for_file`, `progress_end`); answers `-version`; finds `-progress <file>`
   and the last argument; writes one progress block per frame (blocking on `wait_for_file` before
   the last frame when set, so tests can suspend deterministically); writes a small output file
   unless told not to; exits with `exit_code`. Thumbnail invocations write a tiny JPEG. An output
   argument containing a `%0Nd` pattern (an image sequence, as the sandbox generates) writes
   `-frames:v` numbered files (default 1). The script may hold `rules`, a list of
   `{"match": "<substring of the joined arguments>", …behaviour fields}`; the first rule whose
   `match` occurs in the arguments supplies the behaviour, so one test can make a single
   sequence fail (for example `{"match": "sandbox-c", "exit_code": 1, "stderr_lines": ["Invalid data found when processing input"]}`).
3. `fake-ffprobe`: prints `width=`/`height=` from `TRD_FAKE_FFPROBE` (`{"width":..,"height":..}`).
4. `fake-exiftool`: `-ver`; `-csv -@ argfile` prints CSV rows from `TRD_FAKE_EXIFTOOL`
   (`{"rows": {"<file name>": {"Tag": "value"}}}`); write mode returns the exit code and output
   configured there (to simulate `Truncated mdat atom`).
5. `lib.rs`: helpers that build a temp dir with the fakes named `ffmpeg`, `ffprobe`, `exiftool`
   (copies of the built binaries; on macOS ad-hoc signed with `/usr/bin/codesign -s -`, mason's
   rule for copied binaries).
6. `scripts/fake-tools-dir.sh <dest>`: builds `trd-testtools` and makes the same directory at
   `<dest>` (`.exe` names on Windows) for scripts outside cargo tests (the client contract tests,
   the sandbox tests).
7. `system_test.rs`, the guard for system tests (overview §15.2), re-exported from `lib.rs`:
   - `system_test_allowed(get: impl Fn(&str) -> Option<String>, switch: &str) -> bool`: true only
     when `get("CI")` is `Some("true")` and `get(switch)` is `Some("1")`;
   - `require_system_test(switch: &str)`: panics with `refusing to run a system test outside CI:
     set CI=true and <switch>=1` unless `system_test_allowed(|k| std::env::var(k).ok(), switch)`;
   - the macro `system_test!(SWITCH, async fn name() { … })`, which expands to an `async fn name()`
     carrying `#[tokio::test(flavor = "multi_thread")]` and
     `#[ignore = "system test: run by v3-system-tests.yml"]` whose body calls
     `require_system_test("SWITCH")` before anything else. Ordinary runs therefore skip system
     tests, and `-- --ignored` without both variables fails instead of passing silently. The
     calling crate needs `tokio` (features `macros`, `rt-multi-thread`) as a dev-dependency.

## Acceptance criteria
- A1: resolution follows the order, including the development-tree step — check: `cargo test -p trd-core tools`.
- A2: a missing tool yields `PROBLEM_TOOL_MISSING` listing the paths tried, and a tool whose version command fails yields `PROBLEM_TOOL_BROKEN` — check: tests `missing_tool_problem`, `broken_tool_problem`.
- A3: the fakes behave as scripted on macOS, Linux and Windows, including image-sequence outputs, the thumbnail JPEG, `wait_for_file`, and the corrupt-write output of fake-exiftool — check: `cargo test -p trd-testtools`.
- A4: `fake-tools-dir.sh` produces a directory that `Tools::resolve` accepts through `TRD_FFMPEG_DIR` and `TRD_EXIFTOOL_DIR` — check: test `fake_tools_dir_resolves` (runs the script into a temp dir).
- A5: `system_test_allowed` is true only for `CI=true` together with the switch set to `1`: unset, empty, `false`, `1` for `CI`, or `true`, `0` for the switch all give false — check: test `system_test_guard_matrix`.
- A6: `require_system_test` panics with the refusal when the switch is unset — check: test `require_system_test_refuses` (`#[should_panic]`, with a switch name that is never set).
- A7: a `system_test!` test is ignored by an ordinary run: `tests/system_test_macro.rs` declares `system_test!(TRD_TESTTOOLS_NEVER_SET, async fn sample_system_test() {})`, which would fail the run through its guard if it were not ignored — check: test `sample_system_test` is listed as ignored by `cargo test -p trd-testtools`, which passes.

## Tests
`tools.rs` unit tests with temp trees; `trd-testtools/tests/fakes.rs` (A3, A4); `system_test.rs`
unit tests (A5, A6); `trd-testtools/tests/system_test_macro.rs` (A7).

## CI
`v3-gate.yml` `trd`.

## Out of scope
Running renders.

Depends on: TRD-01

---

### TRD-06 — Sequence scanner

## Goal
`trd-core` discovers raw sequences, backup sidecars and export sequences with counts, rates and
sizes exactly as §6.3 says, without ever blocking on a slow path.

## Context
§6.3; Swift `Scanner.swift`; Perl `search_and_discover*`, `count_raw_image_sequences`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.3)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/Scanner.swift`
- `v3/trd/crates/trd-core/src/scan.rs`
- `v3/trd/crates/trd-core/tests/scan.rs` (new)

## Approach
1. `Scanner` holding the raw-count cache (by directory mtime), backup entries, export counts (by
   directory mtime), rate samples and sizes, with an injected clock and a filesystem trait whose
   real implementation lists in `spawn_blocking` with the 10 s timeout and the one-at-a-time rule.
2. `scan(config) -> ScanResult { raw, exports: Vec<ExportSequence>, unreachable: Vec<PathProblem> }`.
3. Longest-match rule, ignore regexes, zero-frame exclusion, hidden-file and regular-file rules.
4. Backup loading in the background at start and every 10 minutes; source entries override.
5. Rate and ETA from samples; background size measurement (2 at a time, 60 s per sequence).

## Acceptance criteria
- A1: the longest raw name wins when two match — check: test `longest_match_wins`.
- A2: a path whose listing hangs is reported unreachable while the others are scanned — check: test `slow_path_does_not_block` (fake filesystem that blocks until released).
- A3: a backup sidecar with no count is skipped and a source entry overrides a backup one — check: tests `backup_zero_skipped`, `source_overrides_backup`.
- A4: rate and ETA come out of injected samples — check: test `export_rate`.
- A5: `ignore_sequence_regexes`, hidden files, non-regular files, extension case and zero-frame directories follow §6.3 — check: tests `ignore_regexes`, `hidden_and_irregular_files`, `extension_case`, `zero_frames_ignored`.
- A6: a sequence directory whose mtime is unchanged is not listed again until 60 s pass; a raw directory not until 10 minutes pass — check: test `mtime_cache` (counting filesystem, injected clock).
- A7: `$YEAR` source and export paths are expanded and scanned — check: test `year_paths_scanned`.
- A8: the export size is measured in the background at most every 60 s, two at a time — check: test `export_size_measurement`.

## Tests
`tests/scan.rs` over temp trees and a fake filesystem; no sleeps (injected clock).

## CI
`v3-gate.yml` `trd`.

## Out of scope
What to render (TRD-12).

Depends on: TRD-03, TRD-04

---

### TRD-07 — Render planning: names, frame patterns, aspect, ffmpeg argv

## Goal
For a complete sequence and a variant, `trd-core` produces the output name, the already-rendered
answer and the exact ffmpeg argument vector of §6.5.

## Context
§6.5; Swift `ImageSequencePattern.swift`, `FFmpegRunner.swift` (`Plan.arguments`, `AspectRatio`),
`RenderDaemon.swift` (`outputVideoFilename`, `shouldRender`, `queueRender`). Swift tests to port:
`testAspectRatio`, `testFFmpegArgsUseCodecProfileAndExtraArgs`, `testFFmpegArgsProResUnchanged`,
`testFFmpegArgsOmitProfileWhenNil`, `testDetectPattern*` (8 tests), `testPlanUsesTheDetectedDigitWidthAndPrefix`,
`testPlanOmitsStartNumberForOrdinarySequences`, `testPlanPassesStartNumberBeforeTheInputArgument`,
`testInputPatternMatchesTheArgumentPassedToFFmpeg`, `testDecompose*` (6 tests).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.5)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/ImageSequencePattern.swift`
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/FFmpegRunner.swift`
- `v3/trd/crates/trd-core/src/plan.rs`
- `v3/trd/crates/trd-core/src/pattern.rs`
- `v3/trd/crates/trd-core/src/aspect.rs`
- `v3/trd/crates/trd-core/tests/plan.rs` (new)

## Approach
1. `pattern.rs`: `decompose` and `detect` ported exactly.
2. `aspect.rs`: `AspectRatio.string` ported exactly.
3. `plan.rs`: `output_filename`, `already_rendered(config, filename, in_flight)`, `probe_size`
   (ffprobe through `Tools`), `final_size`, and `Plan { … } -> argv` in §6.5's order, with the
   partial path, the progress path and the sorted metadata.

## Acceptance criteria
- A1: the argv for each built-in codec matches a golden list, including `-progress`, the partial path and the aspect — check: `cargo test -p trd-core plan`.
- A2: an original-size render of a 6000×4000 sequence gets `-aspect 3/2` — check: test `original_size_aspect_from_source`.
- A3: `[720]` on a 6000×4000 source gives 720×480 and the name suffix `_720w_` — check: test `width_only`.
- A4: every ported Swift test passes — check: `cargo test -p trd-core`.
- A5: output names strip both prefixes repeatedly and match the names the Perl and Swift daemons produced for the same configs (`_720p_`, `_2160p_`, `_OriRes_`, `_720w_` examples from the repository's `config.json` and `gui_config.json`) — check: test `names_match_older_daemons`.
- A6: `already_rendered` finds the output in any expanded `master_videos` or `export_paths` directory and in queued, running or suspended jobs, and not elsewhere — check: test `already_rendered_locations`.
- A7: a failing ffprobe gives `FAILURE_NOT_QUEUEABLE` with "could not read the frame size of <file>"; no numbered frames give `FAILURE_NOT_QUEUEABLE` with the input analysis — check: tests `probe_failure`, `no_pattern`.

## Tests
`tests/plan.rs`; fake-ffprobe for the size probe.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Starting ffmpeg (TRD-08), failure text (TRD-11).

Depends on: TRD-03, TRD-05

---

### TRD-08 — Process runner: detached spawn, progress, suspend, resume, terminate

## Goal
`trd-core` starts ffmpeg detached with its output in a log file, follows its `-progress` file,
and can suspend, resume and end it on every platform.

## Context
§6.6 steps 2–4, §6.8 (signals); Swift `ProcessRunner.swift` (its notes on stdin and fast exits).
Swift tests to port: `testProcessRunnerReportsFastExit`, `testProcessRunnerReportsNonZeroExit`,
`testProcessRunnerThrowsLaunchFailedForMissingBinary`, `testFFmpegProgressParsing*` (3 tests,
adapted to the `-progress` key=value format).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.6, §6.8)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/ProcessRunner.swift`
- `v3/trd/crates/trd-core/src/process.rs`
- `v3/trd/crates/trd-core/src/progress.rs`
- `v3/trd/crates/trd-core/tests/process.rs` (new)

## Approach
1. `spawn_detached(argv, log_path) -> RunningProcess { pid, start_time }` with the unix and Windows
   flags of §6.6; launch failure is a typed error.
2. `wait(&mut RunningProcess) -> ExitInfo { code, by_signal }` for children; `is_alive(pid, start_time)`
   for adopted ones, querying only that pid through `sysinfo` (it never lists other processes).
3. `suspend`, `resume`, `terminate(grace = 10 s)` per platform (`nix` on unix; `NtSuspendProcess`,
   `NtResumeProcess`, `TerminateProcess` on Windows).
4. `progress.rs`: incremental reader of a `-progress` file returning the latest complete block.
5. A `ProcessTable` trait (list processes with name, args, start time) with the `sysinfo`
   implementation `SysinfoProcessTable` and `FakeProcessTable` for tests.
   `SysinfoProcessTable::new` begins with
   `assert!(!cfg!(test), "SysinfoProcessTable lists the owner's real processes; tests use FakeProcessTable")`,
   so no unit test can list real processes; integration tests may not name it either
   (`check-system-tests.sh` R2, TRD-01).

## Acceptance criteria
- A1: a fake-ffmpeg run's progress is read block by block and its exit code reported — check: `cargo test -p trd-core process`.
- A2: suspend then resume of a fake-ffmpeg blocked on `wait_for_file` works on all three OSes — check: test `suspend_resume` (asserts process state, no timing).
- A3: terminate ends a suspended process — check: test `terminate_suspended`.
- A4: the child survives the parent's process group being signalled (unix) — check: test `own_process_group`.
- A5: `SysinfoProcessTable::new` panics inside a unit test — check: test `sysinfo_table_refuses_in_unit_tests` (`#[should_panic]`, in `process.rs`).
- A6: `FakeProcessTable` returns exactly the processes a test gives it — check: test `fake_table_lists_given_processes`.

## Tests
`tests/process.rs` with fake-ffmpeg; deadlines only as hang guards. `process.rs` unit tests (A5,
A6).

## CI
`v3-gate.yml` `trd` (all three OSes).

## Out of scope
The queue (TRD-12).

Depends on: TRD-04, TRD-05

---

### TRD-09 — EXIF collection and sidecars

## Goal
`trd-core` builds the EXIF histogram of a raw sequence with batched exiftool runs, writes the
sidecar (or caches it), and gives ffmpeg its metadata, as §6.4 says.

## Context
§6.4; Swift `ExiftoolRunner.swift` (`ExifAccumulator`), `RenderDaemon.swift`
(`maybeStartExifJob`, `collectExifTotals`, `readExifMetadata`); Perl `read_exif_json`,
`ExiftoolRunner.pm`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.4)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/ExiftoolRunner.swift`
- `v3/trd/crates/trd-core/src/exif.rs`
- `v3/trd/crates/trd-core/tests/exif.rs` (new)

## Approach
1. `collect(raw_dir, raw_types, max_processes, tools) -> Histogram` with 64-file batches, argfiles,
   the `csv` crate, the skip rules and empty-cell rule.
2. `write_sidecar` (atomic, sorted, pretty) with the `exif_cache` fallback; `read_histogram`
   (sidecar or cache).
3. `metadata(histogram)` = single-valued tags, sorted.

## Acceptance criteria
- A1: 130 fake raw files run as 3 batches with at most `max_processes.exiftool` at once — check: test `batches_and_concurrency`.
- A2: binary tags and empty cells are skipped; `SourceFile` is kept — check: test `skip_rules`.
- A3: an unwritable raw parent stores the histogram in `exif_cache` and the sequence still gets metadata — check: test `cache_fallback`.
- A4: a sidecar written by the Swift daemon (fixture) reads back to the same metadata — check: test `reads_swift_sidecar`.

## Tests
`tests/exif.rs` with fake-exiftool and a Swift-format sidecar fixture in the test's temp dir.

## CI
`v3-gate.yml` `trd`.

## Out of scope
XMP (TRD-10).

Depends on: TRD-03, TRD-04, TRD-05

---

### TRD-10 — XMP embedding and the corrupt-render check

## Goal
`trd-core` embeds the canonical XMP-EXIF fields into a rendered file with exiftool and tells a
corrupt container from other exiftool failures, as the Swift `XMPEmbedder` does.

## Context
§6.6 step 5; Swift `XMPEmbedder.swift` (`XMPFieldMapper`, `detectStructuralCorruption`). Swift
tests to port: `testDetectStructuralCorruption*` (3 tests), `testXMPEmbedRoundTripWithFixture`
(runs only when a real exiftool is available, under `TRD_E2E=1`).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.6)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/XMPEmbedder.swift`
- `v3/trd/crates/trd-core/src/xmp.rs`
- `v3/trd/crates/trd-core/tests/xmp.rs` (new)

## Approach
1. Port `XMPFieldMapper.buildExiftoolAssignments` and its helpers exactly (mode with descending-key
   ties, earliest date, focal-length suffix, the three enum mappings, ISO 8601 conversion).
2. `embed(video, histogram, tools) -> XmpResult { Succeeded, Skipped(reason), StructuralCorruption(reason, output), ToolError(output, code) }`
   running `exiftool -overwrite_original -q <assignments> <video>`.

## Acceptance criteria
- A1: assignments for a sample histogram equal the Swift output — check: test `assignments_match_swift`.
- A2: `Truncated mdat atom` output maps to `StructuralCorruption` and an unrelated error to `ToolError` — check: the ported tests.
- A3: with real tools, a rendered MOV gets readable XMP — check: `TRD_E2E=1 cargo test -p trd-core xmp`.

## Tests
`tests/xmp.rs`.

## CI
`v3-gate.yml` `trd`; the real-tool test runs in `trd-media` after TRD-GATE.

## Out of scope
Deciding what to do with the result (TRD-12).

Depends on: TRD-09

---

### TRD-11 — Failure diagnosis

## Goal
`trd-core` explains a failed render in the words the Swift daemon used, from ffmpeg's log and the
input directory.

## Context
§6.9; Swift `RenderFailure.swift`, `RenderFailureDiagnosis.swift`. Swift tests to port:
`testSignificantLinesStripsFFmpegBanner`, `testDecomposeImageFilename`, `testAnalyzeInputSequence*`
(5 tests), `testDiagnose*` (6 tests), `testRenderFailureMergingCountsOccurrences`,
`testRenderFailureConsoleReportIncludesTheEssentials`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.9)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/RenderFailureDiagnosis.swift`
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/RenderFailure.swift`
- `v3/trd/crates/trd-core/src/diagnosis.rs`
- `v3/trd/crates/trd-core/tests/diagnosis.rs` (new)

## Approach
1. `FailureKind` with the Swift labels; `RenderFailure` with merging and the console report.
2. Port every diagnosis function and message text; `diagnose` takes the plan from TRD-07.

## Acceptance criteria
- A1: every ported Swift test passes with identical message text — check: `cargo test -p trd-core diagnosis`.
- A2: the "no frames matched" diagnosis names the prefix and digit-width mismatch for a sample directory — check: test `explains_no_frames`.

## Tests
`tests/diagnosis.rs`.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Storing failures (TRD-12).

Depends on: TRD-07

---

### TRD-12 — Render queue, completion pipeline and controls

## Goal
`trd-core` queues, promotes, runs, completes and controls renders exactly as §6.6–§6.9 say.

## Context
§6.6, §6.7, §6.8, §6.9, overview §6.9; Swift `RenderDaemon.swift` (`processDiscovered`,
`queueRender`, `promoteQueuedRenders`, `launchRender`, `markRenderFinished`, the pause, resume and
stop functions).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.6–§6.9)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/RenderDaemon.swift`
- `v3/trd/crates/trd-core/src/queue.rs`
- `v3/trd/crates/trd-core/src/pipeline.rs`
- `v3/trd/crates/trd-core/src/controls.rs`
- `v3/trd/crates/trd-core/tests/queue.rs` (new)

## Approach
1. `queue.rs`: jobs in the store; `plan_jobs(scan, config, histograms)` creates queued jobs per
   §6.7; `promote()` with both caps and the global-pause rule; removal of unwanted jobs.
2. `pipeline.rs`: per running job, follow progress; on exit run §6.6 steps 4–9 (diagnosis, XMP,
   strikes and auto-pause, rename, history, thumbnail request, events, delete-after-render, log
   retention) and the stale-partial sweep.
3. `controls.rs`: every overview §6.9 action with the §6.8 rules, persisting pauses through the
   config store.
4. Emit `EVENT_SEQUENCE_READY`, `EVENT_RENDER_STARTED`, `EVENT_RENDER_FINISHED`,
   `EVENT_SEQUENCE_FINISHED`, `EVENT_RENDER_FAILED`, `EVENT_SEQUENCE_DELETED`.

## Acceptance criteria
- A1: with `max_processes.ffmpeg = 2` and `max_cuncurrent_renders = 1`, two sequences of three variants run one variant each at a time — check: test `caps`.
- A2: a successful fake render ends with the final file, a history row and `EVENT_RENDER_FINISHED`; the last one also emits `EVENT_SEQUENCE_FINISHED` with the right count and bytes — check: test `completion`.
- A3: three consecutive corrupt renders auto-pause the variant in `config.json` — check: test `auto_pause_after_three`.
- A4: stop deletes the partial file and pauses the variant; retry clears both and re-queues — check: tests `stop`, `retry`.
- A5: global pause suspends running jobs and resume continues only those — check: test `global_pause`.
- A6: delete-after-render waits for every variant — check: test `delete_waits_for_all_variants`.
- A7: a failed variant is not re-queued by the next scan — check: test `failed_not_requeued`.
- A8: queued jobs start in order of their sequence's first sighting, then the variant's position in `rendering` — check: test `promotion_order`.
- A9: a queued job is removed when its variant leaves the config, its output appears, or its sequence is paused or disappears — check: test `unwanted_jobs_removed`.
- A10: on success the partial file is renamed to the final name; if the final name already exists the partial is deleted and the variant counts as already rendered — check: tests `rename_on_success`, `final_name_taken`.
- A11: an orphaned `.*.partial.mov` is deleted at start-up and by the 10-minute sweep, but never one a live job owns — check: test `stale_partial_sweep`.
- A12: only the newest 200 render logs are kept; progress files are deleted when a job ends — check: test `render_log_retention`.
- A13: a non-zero ffmpeg exit stores a diagnosed failure with the log tail, and a repeat of the same failure increments `occurrences` instead of adding a card — check: test `failure_recorded_and_merged`.
- A14: every control of overview §6.9 writes one INFO log entry naming the action and target, and a control that does not apply returns `FAILED_PRECONDITION` — check: test `controls_logged_and_validated`.

## Tests
`tests/queue.rs` with fake tools and temp trees, driven tick by tick (no sleeps).

## CI
`v3-gate.yml` `trd`.

## Out of scope
Re-adoption (TRD-13); snapshots (TRD-16).

Depends on: TRD-06, TRD-07, TRD-08, TRD-09, TRD-10, TRD-11

---

### TRD-13 — Re-adoption of trd's own renders and foreign renders

## Goal
After a restart `trd` picks up its own running renders with full progress and watches foreign
ffmpeg renders, never signalling anything it was not asked to.

## Context
§6.11; Swift `OrphanScanner.swift` (tests `testOrphanScannerParsesFFmpegLine`,
`testOrphanScannerRejectsNonFFmpeg` to port).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.11)
- `rewrite/TimelapseRenderCore/Sources/TimelapseRenderCore/OrphanScanner.swift`
- `v3/trd/crates/trd-core/src/adopt.rs`
- `v3/trd/crates/trd-core/tests/adopt.rs` (new)

## Approach
1. `readopt(store, process_table)`: the three outcomes of §6.11 step 1.
2. `adopt_foreign(process_table)`: parse arguments, skip `.partial.mov` outputs and outputs already
   owned, create adopted jobs; completion by output existence.
3. Everything takes the `ProcessTable` trait; `--no-adopt` skips both, and `--no-adopt-foreign`
   (implied by `--sandbox`) skips `adopt_foreign` only.

## Acceptance criteria
- A1: a job row whose process is alive with a matching start time is adopted with its progress — check: test `readopt_alive`.
- A2: a dead process whose progress ends with `progress=end` and a non-empty partial is finished as a success — check: test `readopt_finished_while_down`.
- A3: a pid reused by another process (start time differs) is not adopted — check: test `pid_reuse`.
- A4: with `--no-adopt-foreign`, a job of its own is re-adopted while a foreign ffmpeg in the same table is neither adopted nor signalled — check: test `no_adopt_foreign_keeps_own`.
- A5: the tests use only `FakeProcessTable` — check: `bash v3/trd/scripts/check-system-tests.sh` (R2: no test file names `SysinfoProcessTable`) and test `sysinfo_table_refuses_in_unit_tests` (TRD-08).

## Tests
`tests/adopt.rs` with `FakeProcessTable`.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Snapshot fields (TRD-16).

Depends on: TRD-12

---

### TRD-14 — Thumbnails and the Swift history import

## Goal
Every rendered video gets a poster frame, and the Swift daemon's recent history appears in
Recently rendered on first start.

## Context
§6.16, §6.17; Swift `DaemonState.swift` (`CompletedRenderResult`), `RenderDaemon.swift`
(`defaultHistoryURL`).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.16, §6.17)
- `v3/trd/crates/trd-core/src/thumbs.rs`
- `v3/trd/crates/trd-core/src/history_import.rs`
- `v3/trd/crates/trd-core/tests/thumbs.rs` (new)

## Approach
1. `Thumbnailer` with a 2-slot semaphore, the §6.16 command, on-demand generation for rows
   without a thumbnail, deletion with history retention.
2. `import_swift_history(path, store, retention, now)` parsing the Swift JSON (ISO 8601 dates),
   deriving variant fields from the file name when it matches the §6.5 pattern, once per database.

## Acceptance criteria
- A1: a finished render's thumbnail is written and `has_thumbnail` set — check: test `thumbnail_written` (fake ffmpeg).
- A2: a Swift `render_history.json` fixture imports once, skipping entries older than the window — check: test `import_once`.
- A3: at most two thumbnails are made at once; a failing ffmpeg leaves `has_thumbnail = false` and logs a warning — check: tests `thumbnail_concurrency`, `thumbnail_failure`.
- A4: an imported row without a thumbnail gets one on demand when its file exists, and none when it does not — check: test `thumbnail_on_demand`.
- A5: history retention deletes the thumbnails of the rows it removes — check: test `thumbnail_retention`.
- A6: imported rows get variant fields parsed from file names that match §6.5 and keep empty fields otherwise — check: test `import_parses_names`.

## Tests
`tests/thumbs.rs`.

## CI
`v3-gate.yml` `trd`.

## Out of scope
Serving thumbnails (TRD-17).

Depends on: TRD-04, TRD-05

---

### TRD-15 — macOS Full Disk Access guard

## Goal
On macOS `trd` touches no protected path until it has Full Disk Access, and says so.

## Context
§6.12, overview D2.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.12)
- `v3/trd/crates/trd-core/src/fda.rs`

## Approach
1. `FdaGuard::check(home) -> FdaState { Granted, Missing }` opening `TCC.db` read-only; `ENOENT`
   counts as granted; a no-op returning `Granted` on other platforms or with `--no-fda-check`.
2. The problem text of §6.12 as constants used by TRD-16.

## Acceptance criteria
- A1: `EPERM` and `EACCES` map to `Missing`, `ENOENT` and success to `Granted` — check: unit tests with an injected opener.
- A2: the problem title, detail and fix equal §6.12 — check: unit test.

## Tests
Unit tests in `fda.rs` (no real `TCC.db` access in tests).

## CI
`v3-gate.yml` `trd`.

## Out of scope
Gating the scan (TRD-16).

Depends on: TRD-01

---

### TRD-16 — Engine loop, snapshots, problems and events

## Goal
One engine ties scanning, EXIF, the queue, re-adoption, thumbnails and the guard together, and
publishes the §6.10 `Snapshot` on every change.

## Context
§6.1 (reload), §6.3 (tick), §6.10, §6.12, §6.14, §6.15; Swift `RenderDaemon.run`,
`publishSnapshot`, `DaemonState.swift`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.10)
- `v3/trd/crates/trd-core/src/engine.rs`
- `v3/trd/crates/trd-core/src/snapshot.rs`
- `v3/trd/crates/trd-core/src/problems.rs`
- `v3/trd/crates/trd-core/src/events.rs`
- `v3/trd/crates/trd-core/tests/engine.rs` (new)

## Approach
1. `Engine` actor (tokio task) owning the state; commands over an mpsc channel (every control,
   config edit, rescan); state published through a `watch` channel of `Arc<SnapshotModel>`.
2. Tick: guard check → config reload → scan → histograms/EXIF jobs → queue → promotion → snapshot.
   Start-up: tools, re-adoption, history import, backup loading, stale partial sweep.
3. `snapshot.rs`: build `SnapshotModel` (plain Rust types mirroring §7) with stages, variant
   statuses, the ordering table, counts, phase, recently rendered and `file_exists` caching.
4. `problems.rs`: the problem set with stable ids and `since_ms`; `events.rs`: the event helpers.

## Acceptance criteria
- A1: a sequence moves from EXPORTING through COLLECTING_METADATA, QUEUED and RENDERING, then leaves `in_progress` while its videos head `recently_rendered` in the same snapshot — check: test `lifecycle` (fake tools, ticks driven by the test).
- A2: the ordering table holds for a mixed set of sequences — check: test `ordering`.
- A3: with Full Disk Access missing no path is listed and the phase is WAITING_FOR_DISK_ACCESS — check: test `fda_blocks_scanning` (fake guard and a filesystem that fails the test if touched).
- A4: a config edit triggers a rescan and an `EVENT_CONFIG_RELOADED` — check: test `config_reload_event`.
- A5: every phase of §6.10 is reached in a scripted run (starting, no config, waiting for disk access, idle, exporting, rendering, paused) — check: test `phases`.
- A6: each problem kind appears with its stable id and keeps its `since_ms` while it persists, then disappears when resolved — check: test `problems_lifecycle`.
- A7: `counts`, `recently_rendered_total` and `revision` (strictly increasing, one step per change) are correct — check: test `snapshot_counters`.
- A8: a count mismatch shows `STAGE_COUNT_MISMATCH` with the §6.10 note and never renders — check: test `count_mismatch`.
- A9: `file_exists` of a recently rendered row is rechecked at most every 60 s — check: test `file_exists_cache` (injected clock).
- A10: an adopted foreign render is left alone (§6.11 step 3): a fake-ffmpeg that the test itself started, listed in `FakeProcessTable` as a foreign render, is still running and not suspended after a global pause, a config reload that no longer covers its folder, and engine shutdown; only an explicit `StopRender` on its job ends it — check: test `foreign_render_left_alone`.

## Tests
`tests/engine.rs`.

## CI
`v3-gate.yml` `trd`.

## Out of scope
gRPC (TRD-17).

Depends on: TRD-12, TRD-13, TRD-14, TRD-15

---

### TRD-17 — gRPC server on loopback

## Goal
`trd daemon` runs the engine and serves every §7 RPC on `127.0.0.1:50071`.

## Context
§5 (flags, `TRD_LISTENING`), §7 (API and server rules), §6.13 (paths).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §5, §7)
- `v3/trd/crates/trd/src/main.rs`
- `v3/trd/crates/trd/src/server.rs`
- `v3/trd/crates/trd/tests/server.rs` (new)

## Approach
1. `trd daemon`: parse flags, resolve directories, init logging, open the store, start the engine,
   bind loopback (retry every 30 s on failure), print `TRD_LISTENING`, serve until SIGTERM/SIGINT
   (Windows: Ctrl-C/console close), then stop accepting, end streams with `UNAVAILABLE`, and exit
   0 without touching running renders.
2. `server.rs`: map `SnapshotModel` and store rows to proto messages; implement every RPC with the
   §7 rules; `WatchSnapshots` sampling at `max_per_second`; `WatchLog` with backlog then live;
   `ListDirectory` with roots per OS; device RPCs return `PERMISSION_DENIED` unless the listener
   is loopback (a request extension set per listener).

## Acceptance criteria
- A1: every RPC is exercised by a test against a daemon started on port 0 with fake tools — check: `cargo test -p trd server`.
- A2: `WatchSnapshots` delivers the current snapshot at once and never more than `max_per_second` — check: test `watch_rate` (counts messages per revision, not time).
- A3: `UpdateConfig` with a stale revision returns `FAILED_PRECONDITION` — check: test.
- A4: SIGTERM leaves a running fake render alive — check: test `shutdown_keeps_renders` (unix).

## Tests
`tests/server.rs` using the tonic client from `trd-proto`, `--no-adopt --no-fda-check --lan-bind off --no-mdns`, temp dirs.

## CI
`v3-gate.yml` `trd`.

## Out of scope
LAN, TLS, pairing (TRD-18).

Depends on: TRD-02, TRD-16

---

### TRD-18 — LAN listener: TLS identity, tokens, pairing

## Goal
Paired devices can use the full API over pinned TLS on the LAN; nothing else can.

## Context
§8.1–§8.3; ReelVault `identity.rs`, `pairing.rs`, `auth.rs`, `main.rs` (the interceptor).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §8)
- `v3/trd/crates/trd/src/tls.rs`
- `v3/trd/crates/trd/src/auth.rs`
- `v3/trd/crates/trd/src/pairing.rs`
- `v3/trd/crates/trd/tests/lan.rs` (new)

## Approach
1. `tls.rs`: create or load the identity (§8.1); rustls server config with the `ring` provider.
2. `auth.rs`: an interceptor that allows `GetServerInfo` and `Pair` and otherwise checks the bearer
   token against `devices`; marks the request as LAN.
3. `pairing.rs`: code issue, attempts limit, `Pair`, revoke (ending open streams of that device).
4. Bind `--lan-bind`, retry on failure with `PROBLEM_LAN_LISTENER_FAILED`.

## Acceptance criteria
- A1: a client pinned to the fingerprint pairs with a code from `StartPairing` and then streams snapshots — check: test `pair_then_stream` (LAN bound to `127.0.0.1:0`).
- A2: calls without a token fail `UNAUTHENTICATED`; `StartPairing` over LAN fails `PERMISSION_DENIED` — check: tests.
- A3: five wrong codes invalidate the code — check: test `attempt_limit`.
- A4: revoking a device ends its open stream — check: test `revoke_ends_stream`.
- A5: the fingerprint is unchanged across restarts — check: test `stable_identity`.

## Tests
`tests/lan.rs`: the daemon runs with `--no-adopt --no-fda-check --no-mdns`, temporary
directories and port 0, and never binds a non-loopback address.

## CI
`v3-gate.yml` `trd`.

## Out of scope
mDNS (TRD-19).

Depends on: TRD-17

---

### TRD-19 — mDNS advertisement

## Goal
`trd` advertises `_trd._tcp` with the §8.4 TXT records, following address changes.

## Context
§8.4; ReelVault `discovery.rs`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §8.4, §9.7)
- `v3/trd/crates/trd/src/mdns.rs`
- `v3/trd/crates/trd/tests/mdns_live.rs` (new)

## Approach
1. Build `ServiceInfo` (instance sanitizing, host, port, addresses, TXT) in a pure function.
2. Register with `mdns-sd`, re-register on address changes (checked every 30 s), unregister on
   shutdown, raise `PROBLEM_MDNS_FAILED` on errors. Off with `--no-mdns` or `--lan-bind off`.
   Registration goes through a `Registrar` trait: `MdnsSdRegistrar` is the only code that creates
   an `mdns_sd::ServiceDaemon`, and its `new` begins with
   `assert!(!cfg!(test), "MdnsSdRegistrar uses real multicast; tests use FakeRegistrar")`; unit
   tests use `FakeRegistrar`, which records registrations. The address source is a trait too, so
   the re-registration logic is testable with a fake.
3. `tests/mdns_live.rs` (system test, overview §15.2), declared with
   `system_test!(TRD_MDNS_TEST, …)` (TRD-05): starts the `trd` binary with
   `--sandbox --port 0 --lan-bind 127.0.0.1:0` (mDNS on), browses `_trd._tcp.local.` with an
   `mdns-sd` browser in the test, and checks the instance name and every TXT record against
   `GetServerInfo`; then stops `trd` and checks the service is removed within 30 s.

## Acceptance criteria
- A1: the pure builder produces the §8.4 instance name, port and TXT records — check: unit tests.
- A2: an address change re-registers once with the new addresses; an unchanged set does not — check: unit test with the fake address source.
- A3: real multicast happens only in the system test (it could raise a macOS prompt on the owner's Mac): `tests/mdns_live.rs` uses only `system_test!`, no other test names `MdnsSdRegistrar` or `ServiceDaemon`, and every other test that starts the daemon passes `--no-mdns` — check: `bash v3/trd/scripts/check-system-tests.sh` (R1–R3) and test `registrar_refuses_in_unit_tests` (`#[should_panic]`).
- A4: the live test passes on the Linux runner — check: the `trd-live` job of `v3-system-tests.yml` (after TRD-29).

## Tests
Unit tests of the builder, the re-registration logic and `registrar_refuses_in_unit_tests`;
`tests/mdns_live.rs` (system test).

## CI
`v3-gate.yml` `trd` (the live test is ignored there); `v3-system-tests.yml` `trd-live`.

## Out of scope
Client discovery.

Depends on: TRD-18

---

### TRD-20 — CLI client commands and `trd status`

## Goal
Every §5 client command works against a running `trd`, and `trd status` shows the Activity view
as text like the Perl console did, with new entries at the top.

## Context
§5; overview §6.1 (sections and order); Perl `UpdateableLog.pm` and `progress_bar` (bar style:
blue brackets, green `*` for done, yellow `-` for remaining; width `progress_bar_width`; colour
when `log_in_color` and stdout is a terminal, never with `--plain`).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §5)
- `v3/trd/crates/trd/src/cli.rs`
- `v3/trd/crates/trd/src/status_view.rs`
- `v3/trd/crates/trd/tests/cli_client.rs` (new)

## Approach
1. Implement every client subcommand over the loopback client; `trd pair` prints the code, expiry,
   server name, LAN addresses, port and the first 16 hex digits of the fingerprint, and with
   `--json` the §5 object; `trd scan --once` runs the scanner and planner in-process without a
   daemon.
2. `status_view.rs`: render a `Snapshot` to lines — header, problems, failures, in progress (card
   line with bar, frames and path; indented variant lines with bar, frames, fps and ETA), recently
   rendered (time, relative time, file name, variant label, frames, duration, size, export path;
   newest 10). `--watch` redraws on the alternate screen at most twice a second.
3. The tests run every client command through one helper, `client(address, args)`, which always
   passes `--address` with the test daemon's port: a client command without it would reach
   `127.0.0.1:50071`, the owner's installed `trd` (overview §10 rule 3).

## Acceptance criteria
- A1: `trd status --plain` for a fixture snapshot equals a golden text — check: `cargo test -p trd cli_client`.
- A2: each control subcommand changes the daemon's state, and `trd pair --json` prints the §5 object — check: tests against a daemon on port 0.
- A3: `trd scan --once` lists complete sequences and the files it would render — check: test over a temp tree.
- A4: the tests' client helper never falls back to the default address: with nothing listening on the port it is given, a client command exits 2 with "trd is not running on 127.0.0.1:<that port>" — check: test `client_uses_test_address`.

## Tests
`tests/cli_client.rs`: the daemon runs with `--sandbox --no-mdns --lan-bind off`, temporary
directories and port 0.

## CI
`v3-gate.yml` `trd`.

## Out of scope
`trd service` (TRD-21).

Depends on: TRD-18

---

### TRD-21 — Service install, uninstall, status, restart

## Goal
`trd service …` sets up and controls the per-user autostart on each OS exactly as §10.4 says.

## Context
§10.1 (LaunchAgent content), §10.2 (unit), §10.3 (task XML), §10.4; overview D2.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §10)
- `v3/trd/crates/trd/src/service/mod.rs`
- `v3/trd/crates/trd/src/service/macos.rs`
- `v3/trd/crates/trd/src/service/linux.rs`
- `v3/trd/crates/trd/src/service/windows.rs`
- `v3/trd/packaging/macos/com.timelapserender.trd.plist` (new)
- `v3/trd/packaging/linux/trd.service` (new)
- `v3/trd/packaging/windows/task.xml.in` (new)
- `v3/trd/crates/trd/tests/service_live.rs` (new)

## Approach
1. The three templates as files (also used by TRD-24–26), rendered with the binary path and
   optional `--config`.
2. `Service` trait with install, uninstall, status, restart; one implementation per OS taking a
   home directory and a `CommandRunner`: `SystemRunner` runs the commands, and its `run` begins
   with `assert!(!cfg!(test), "SystemRunner runs real service-manager commands; tests use RecordingRunner")`;
   `RecordingRunner` records them and returns scripted results. `--dry-run` prints the files and
   commands through the same code path with a `RecordingRunner`.
3. `trd service` subcommands wired in `cli.rs`.
4. `tests/service_live.rs` (system test, overview §15.2), declared with
   `system_test!(TRD_SERVICE_TEST, …)` (TRD-05). On the runner it installs the
   service for the runner's user with the test's `trd` binary and a temporary `--config`, waits
   up to 60 s for `trd service status` to report running and for a loopback `GetServerInfo` to
   answer, runs `trd service restart` and waits again (a new pid), uninstalls, and checks that
   the plist, unit or task is gone and nothing answers on the loopback port.

## Acceptance criteria
- A1: install on each OS writes the expected file into a temporary home and issues the expected commands — check: `cargo test -p trd service` (recording runner).
- A2: the rendered plist, unit and task XML equal golden files; the task XML is UTF-16 LE with a BOM — check: tests.
- A3: `launchctl`, `systemctl` and `schtasks` run for real only in the system test: `tests/service_live.rs` uses only `system_test!` and no other test names `SystemRunner` — check: `bash v3/trd/scripts/check-system-tests.sh` (R1, R2) and test `system_runner_refuses_in_unit_tests` (`#[should_panic]`).
- A4: `--dry-run` prints the files and commands and changes nothing — check: test `dry_run_changes_nothing` (a unit test whose `RecordingRunner` and temporary home show that no command ran and no file was written).
- A5: the live test passes on the macOS, Linux and Windows runners — check: the `trd-live` job of `v3-system-tests.yml` (after TRD-29).

## Tests
Unit tests per OS module (all compiled on every OS; run behind `cfg` only where they need the OS),
including `system_runner_refuses_in_unit_tests`; `tests/service_live.rs` (system test).

## CI
`v3-gate.yml` `trd` (the live test is ignored there); `v3-system-tests.yml` `trd-live`.

## Out of scope
Installers (TRD-24–26).

Depends on: TRD-17

---

### TRD-22 — Port the ffmpeg build recipe and its workflow

## Goal
`v3/ffmpeg` builds the ffmpeg and ffprobe `trd` ships for every target, and `v3-ffmpeg.yml`
builds and smoke-tests them in CI.

## Context
Overview D6; §9.2; star's `ffmpeg/README.md`, `build.sh`, `lib/common.sh`, `lib/deps.sh`,
`lib/targets.sh`, `versions.env`, `smoke_test.sh` and `.github/workflows/ffmpeg-gpu.yml` at commit
`ae7ae6f2c2687c3c892832ce0c241a4e41d8f0d1` (§2).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §9.2)
- `v3/ffmpeg/build.sh` (new)
- `v3/ffmpeg/lib/common.sh` (new)
- `v3/ffmpeg/lib/deps.sh` (new)
- `v3/ffmpeg/lib/targets.sh` (new)
- `v3/ffmpeg/versions.env` (new)
- `v3/ffmpeg/smoke_test.sh` (new)
- `v3/ffmpeg/README.md` (new)
- `v3/ffmpeg/Makefile` (new)
- `.github/workflows/v3-ffmpeg.yml` (new)

## Approach
1. Copy the star files at that commit. Remove the `tables` target, the `--lgpl` option, and
   everything that refers to `check_extra_encoders.sh`, `muxer_codecs.c` or StarCore; change
   `--extra-version=star` to `--extra-version=trd`, the BUILDINFO recipe URL to this repository,
   and `--install` to copy into `v3/ffmpeg/dist/<target>` only. Record the source commit in the
   README.
2. Rewrite `smoke_test.sh`'s functional section for trd's needs, keeping its pass/fail/skip
   format and linkage checks: make 48 frames `LRT_%05d.tif` (rgb48le, 600×400) with
   `testsrc2`; render with the exact §6.5 argument vectors for `ProResUHQ` original size,
   `ProResMQ` `[1080,720]`, `h264` `[720]`, plus `libx265`; check with ffprobe that each output
   has 48 frames, the expected size, SAR 1:1 and DAR 3:2 (original size), `yuv422p10le` /
   `yuv444p10le` / `yuv420p`, `color_range=pc`, `color_primaries=bt709`, a `tmcd` stream, and that
   the `-progress` file ends with `progress=end`; decode each with `-f null -`; extract a JPEG
   thumbnail with §6.16's command; hardware encoders reported, not failed, without `--hw`; and a
   `buildinfo` case: `BUILDINFO.txt` beside the binaries names FFmpeg 9.0.2 (the version in
   `versions.env`) and this repository's recipe URL, and `ffmpeg -version` reports the
   `--extra-version=trd` suffix.
3. `Makefile`: `release` = `./build.sh` (native target), `clean` = remove `.work` and `dist`,
   `run` and `develop` = build the native target then smoke-test it, `test` = smoke test of the
   native build (building it first when missing), `check` = `bash -n` on every script, `ci` =
   print "built by its own workflow" and exit 0.
4. `v3-ffmpeg.yml` per §9.2.

## Acceptance criteria
- A1: `./v3/ffmpeg/build.sh` builds the native target on the owner's Mac and the smoke test passes — check: `ffmpeg-smoke` suite.
- A2: `v3-ffmpeg.yml` is green for macos-universal, linux-x64, linux-arm64 and windows-x64 — check: a `workflow_dispatch` run.
- A3: BUILDINFO.txt names FFmpeg 9.0.2 and this repository's recipe, and the binaries carry the `trd` version suffix — check: the smoke test's `buildinfo` case, in the `ffmpeg-smoke` suite and in every `v3-ffmpeg.yml` target.

## Tests
The smoke test.

## CI
`v3-ffmpeg.yml` (new).

## Out of scope
Packaging into installers (TRD-24–26).

Depends on: none

---

### TRD-23 — Bundle exiftool and its workflow

## Goal
`v3/exiftool` fetches a pinned ExifTool for Unix and Windows, verifies it, smoke-tests it, and CI
publishes both as artifacts.

## Context
Overview D5; §6.15 (how trd runs it); §9.3.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §6.15, §9.3)
- `v3/exiftool/versions.env` (new)
- `v3/exiftool/fetch.sh` (new)
- `v3/exiftool/smoke_test.sh` (new)
- `v3/exiftool/fixtures/tiny.tif` (new)
- `v3/exiftool/fixtures/tiny.mov` (new)
- `v3/exiftool/README.md` (new)
- `v3/exiftool/Makefile` (new)
- `.github/workflows/v3-exiftool.yml` (new)

## Approach
1. `versions.env`: `EXIFTOOL_VERSION` = the newest production release on the day of the ticket;
   `EXIFTOOL_UNIX_URL` = `https://github.com/exiftool/exiftool/archive/refs/tags/<version>.tar.gz`
   with its SHA-256; `EXIFTOOL_WINDOWS_URL` =
   `https://sourceforge.net/projects/exiftool/files/exiftool-<version>_64.zip/download` with its
   SHA-256 and `EXIFTOOL_WINDOWS_MIRROR` =
   `https://downloads.sourceforge.net/project/exiftool/exiftool-<version>_64.zip`. (exiftool.org
   serves only the current development build, so a production release's zip is not there.)
2. `fetch.sh unix|windows <dest>`: download into `v3/exiftool/.work/` (mirror on failure), verify
   SHA-256 (a mismatch fails), extract; Unix: `<dest>/exiftool` (the script) and `<dest>/lib/`;
   Windows: `<dest>/exiftool.exe` (renamed from `exiftool(-k).exe`) and `<dest>/exiftool_files/`;
   copy the licence text to `<dest>/LICENSE-exiftool.txt`.
3. Fixtures made once with ffmpeg (a 16×16 TIFF and a 1-second 16×16 ProRes MOV) and committed.
4. `smoke_test.sh <dir>`: `-ver`; write EXIF tags into a copy of `tiny.tif` and read them back with
   `-csv`; write the TRD-10 XMP assignments into a copy of `tiny.mov` and read them back with
   `-XMP:all`; print `PASS`/`FAIL` lines; on Unix run through `/usr/bin/perl` or `perl`.
5. `Makefile`: `run` and `develop` = fetch the bundle for this host (`unix`, or `windows` under
   Git Bash) and smoke-test it; `release` = fetch both bundles into `dist/`; `clean` = remove
   `.work` and `dist`; `test` = fetch (when missing) and smoke-test the host's bundle; `check` =
   `bash -n` on every script; `ci` = print "built by its own workflow" and exit 0.
6. `v3-exiftool.yml` per §9.3.

## Acceptance criteria
- A1: `fetch.sh` refuses a tarball whose checksum differs — check: smoke test case `checksum_mismatch` (a tampered copy).
- A2: the smoke test passes on Linux, macOS and Windows in CI — check: `v3-exiftool.yml`.
- A3: the `exiftool-smoke` suite passes on the owner's Mac — check: `bash v3/scripts/suite.sh exiftool-smoke`.

## Tests
The smoke test.

## CI
`v3-exiftool.yml` (new).

## Out of scope
Installers.

Depends on: none

---

### TRD-24 — Icons and the macOS app bundle and .pkg

## Goal
`build-pkg.sh` produces the signed, notarized `.pkg` of §10.1 that installs the app and its
LaunchAgent and starts `trd`; the shared icon exists for every client.

## Context
§10.1, §9.4 (how CI calls it), overview D2 and §8 (Full Disk Access); ReelVault
`release-core.sh` and `.github/workflows/release.yml` (keychain, notarization) and its fix
"make the macOS .pkg non-relocatable".

## Files
- `v3/1-trd-core-daemon.md` (this spec; §10.1)
- `v3/assets/icon/make_icon.swift` (new)
- `v3/assets/icon/make_ico.py` (new)
- `v3/assets/icon/trd-1024.png` (new)
- `v3/assets/icon/trd.icns` (new)
- `v3/assets/icon/monitor-1024.png` (new)
- `v3/assets/icon/monitor-512.png` (new)
- `v3/assets/icon/monitor.icns` (new)
- `v3/assets/icon/monitor.ico` (new)
- `v3/trd/packaging/macos/Info.plist.in` (new)
- `v3/trd/packaging/macos/preinstall` (new)
- `v3/trd/packaging/macos/postinstall` (new)
- `v3/trd/packaging/macos/build-pkg.sh` (new)
- `v3/trd/packaging/macos/test-scripts.sh` (new)

## Approach
1. Icons: `make_icon.swift --variant trd|monitor` draws, offscreen with CoreGraphics (nothing on
   screen), a 1024×1024 rounded square in `#1E2A44` with a white film strip whose frames step from
   dark to light and a thin amber (`#F2A33A`) progress arc around it; the `monitor` variant adds a
   white eye glyph in the lower right. It writes `<variant>-1024.png` and the iconset sizes;
   `iconutil` makes `trd.icns` and `monitor.icns`; `sips` makes `monitor-512.png`; `make_ico.py`
   (standard library only) writes a PNG-compressed `monitor.ico` (16, 32, 48, 256). The outputs
   are committed. The daemon app uses `trd.icns`; the desktop, iOS and Android clients use the
   `monitor` files.
2. `build-pkg.sh` flags: `--version`, `--trd-bin` (universal, else it builds and lipos),
   `--ffmpeg-dir`, `--exiftool-dir`, `--out`, `--sign-app`, `--sign-installer`, `--notarize`
   (needs `APPLE_API_KEY_PATH`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`). Steps: assemble the
   bundle, sign inside out (or ad hoc), build the payload root with the LaunchAgent from TRD-21,
   `pkgbuild --analyze` then set `BundleIsRelocatable` false, `pkgbuild` with scripts and
   identifier, sign, notarize, staple, `pkgutil --check-signature`.
3. Wire `make release` in `v3/trd/Makefile` to it on macOS (unsigned unless the signing variables
   are set).

## Acceptance criteria
- A1: an unsigned local build produces a `.pkg` whose payload has the app with `trd`, `ffmpeg`, `ffprobe`, exiftool, licences and the LaunchAgent — check: `pkgutil --payload-files`.
- A2: `BundleIsRelocatable` is false in the component plist — check: script output.
- A3: in CI with secrets, the notarization result is `Accepted` and the pkg is stapled — check: TRD-27's dispatch run.
- A4: the `preinstall` and `postinstall` scripts do nothing and exit 0 when there is no console user, and issue exactly the §10.1 `launchctl` commands when there is one — check: `bash v3/trd/packaging/macos/test-scripts.sh` (runs both scripts with a stub `launchctl`, `stat` and `id` on `PATH`).
- A5: the package installs on a runner, the LaunchAgent runs `trd`, `trd status` answers and the installed app verifies with `codesign` — check: the `trd-install-macos` job of `v3-system-tests.yml` (after TRD-29).

## Tests
`build-pkg.sh --self-check` validates the assembled bundle (paths, `lipo -archs` of every Mach-O,
plist keys) and runs in `ci.sh` on macOS; `v3/trd/packaging/macos/test-scripts.sh` (new) tests the
install scripts with stubs; the `trd-install-macos` system test installs the package for real.

## CI
Used by `v3-trd-build.yml` (TRD-27).

## Out of scope
Installing on the owner's machine (the owner does that).

Depends on: TRD-21, TRD-22, TRD-23

---

### TRD-25 — Linux .deb and .tar.gz

## Goal
`build-deb.sh` produces the §10.2 packages for amd64 and arm64.

## Context
§10.2; ReelVault `release-core.sh` (dpkg-deb usage).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §10.2)
- `v3/trd/packaging/linux/build-deb.sh` (new)
- `v3/trd/packaging/linux/test-deb-in-container.sh` (new)
- `v3/trd/packaging/linux/postinst` (new)
- `v3/trd/packaging/linux/prerm` (new)

## Approach
1. Stage `/usr/lib/trd`, the symlink, docs and the unit from TRD-21; `DEBIAN/control` with the
   §10.2 fields; `postinst` and `prerm`; `dpkg-deb --build --root-owner-group`.
2. The `.tar.gz` with `INSTALL.txt`.
3. `make release` on Linux calls it.
4. `build-deb.sh --self-check <deb>`: compares `dpkg-deb --contents` with the §10.2 path list and
   `dpkg-deb --field <deb> Depends` with the §10.2 dependencies, printing each difference and
   exiting 1 on any.
5. `test-deb-in-container.sh <deb>`: `docker run --rm` of a clean `ubuntu:24.04` with the deb
   mounted read-only, which runs `apt-get update`, `apt-get install -y <deb>`,
   `/usr/bin/trd version`, then `trd daemon --sandbox --no-mdns --lan-bind off` in the
   background with a temporary `--config`, `--data-dir` and `--log-dir`, and waits up to 60 s
   for `trd status --plain` to answer; exits non-zero when any step fails. Both run in
   `v3-trd-build.yml`'s Linux jobs (§9.4); the container needs Docker, which the owner's Mac
   need not have.

## Acceptance criteria
- A1: `dpkg-deb --contents` lists every §10.2 path and `dpkg-deb --field` shows the dependencies — check: `build-deb.sh --self-check`, run by `v3-trd-build.yml` (TRD-27) on amd64 and arm64.
- A2: installing the deb in a clean `ubuntu:24.04` container and running `/usr/bin/trd version` works, and `trd daemon --sandbox --no-mdns --lan-bind off` started in the container answers `trd status --plain` — check: `test-deb-in-container.sh`, run by `v3-trd-build.yml` (TRD-27) on amd64 and arm64.
- A3: on a runner the installed unit is enabled for user sessions, `systemctl --user start trd` starts it, `trd status` answers, and removing the package removes `/usr/lib/trd` and disables the unit — check: the `trd-install-linux` job of `v3-system-tests.yml` on amd64 and arm64 (after TRD-29).

## Tests
`build-deb.sh --self-check`; `test-deb-in-container.sh`; the `trd-install-linux` system test.

## CI
Used by `v3-trd-build.yml`.

## Out of scope
Other distributions' formats.

Depends on: TRD-21, TRD-22, TRD-23

---

### TRD-26 — Windows installer and .zip

## Goal
`build-installer.sh` produces the per-user setup `.exe` and `.zip` of §10.3.

## Context
§10.3; ReelVault `releases/reelvault_core_installer.nsi` (NSIS reference).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §10.3)
- `v3/trd/packaging/windows/build-installer.sh` (new)
- `v3/trd/packaging/windows/test-installer.sh` (new)
- `v3/trd/packaging/windows/trd.nsi` (new)

## Approach
1. `trd.nsi`: `RequestExecutionLevel user`, install dir, files, HKCU uninstall entry, runs
   `trd.exe service install` and `schtasks /Run`; the uninstaller runs `service uninstall` and
   removes files.
2. `build-installer.sh` stages files, runs `makensis` with version defines, and zips.
3. `test-installer.sh <setup.exe>` (Git Bash, CI only: it refuses to run unless `CI=true`): runs
   the installer with `/S` and `LOCALAPPDATA` set to a temporary directory, runs the installed
   `trd.exe version`, runs the uninstaller with `/S`, and checks that the install directory and
   the `TimelapseRenderDaemon` task are gone; exits non-zero when any step fails. It runs in
   `v3-trd-build.yml`'s Windows job (§9.4).

## Acceptance criteria
- A1: the installer installs silently (`/S`) into a temporary `LOCALAPPDATA` on the CI runner, `trd.exe version` runs, and the uninstaller removes it — check: `test-installer.sh`, run by `v3-trd-build.yml` (TRD-27).
- A2: after a silent install the `TimelapseRenderDaemon` task exists and has started `trdw.exe`, `trd.exe status --plain` answers, and the silent uninstall removes the task and the directory — check: the `trd-install-windows` job of `v3-system-tests.yml` (after TRD-29).

## Tests
`test-installer.sh`; the `trd-install-windows` system test.

## CI
Used by `v3-trd-build.yml`.

## Out of scope
Code signing on Windows.

Depends on: TRD-21, TRD-22, TRD-23

---

### TRD-27 — trd build and release workflows

## Goal
A `v3.*` tag builds every `trd` package in CI and creates a draft release; a manual dispatch
builds the same without publishing.

## Context
§9.4, §9.5; ReelVault `.github/workflows/release.yml`.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §9.4, §9.5)
- `.github/workflows/v3-trd-build.yml` (new)
- `.github/workflows/v3-release.yml` (new)
- `v3/scripts/release-version.sh` (new)
- `v3/scripts/test-release-version.sh` (new)

## Approach
1. `release-version.sh <github-ref> <VERSION file>` prints `version=…`, `is_release=…` and
   `prerelease=…` lines for `$GITHUB_OUTPUT` and exits 1 when a `refs/tags/v3.*` ref does not
   match the file; the `version` job runs it.
2. Write both workflows as §9.4 and §9.5 say, including the signing requirement on releases, the
   container install check of the deb (TRD-25 A2) and the silent Windows install check
   (TRD-26 A1).
3. Run `workflow_dispatch` of `v3-release.yml` and fix until green.

## Acceptance criteria
- A1: a dispatch run is green and produces every §9.4 asset as `release-asset-trd-*` artifacts — check: the run.
- A2: on a dispatch run the publish job is skipped — check: the run.
- A3: a tag that differs from `v3/VERSION` is rejected, a matching tag is a release, and a `-` suffix marks a pre-release — check: `bash v3/scripts/test-release-version.sh`.

## Tests
`v3/scripts/test-release-version.sh` covers the version logic.

## CI
`v3-trd-build.yml`, `v3-release.yml` (new).

## Out of scope
Client release jobs (specs 2–4).

Depends on: TRD-24, TRD-25, TRD-26

---

### TRD-28 — Sandbox and `make develop`

## Goal
`make develop` runs a complete, isolated `trd` with generated sequences in every state, for the
owner and for the clients' `develop` targets.

## Context
§12; overview D16 and §9.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §12)
- `v3/trd/sandbox/sandbox.py` (new)
- `v3/trd/sandbox/README.md` (new)
- `v3/trd/crates/trd/tests/sandbox.rs` (new)
- `v3/trd/Makefile`

## Approach
1. `sandbox.py` (Python 3 standard library, run with `python3 -I`): `init [--dir PATH] [--reset]
   [--check]`, `drip [--dir PATH] [--fps N]`, `path [--dir PATH]`, building the §12 tree with the
   resolved tools (§6.15 order, so `TRD_FFMPEG_DIR` and `TRD_EXIFTOOL_DIR` are honoured).
2. Makefile `develop`, `develop-daemon` and `run` (the real setup: `cargo run -- daemon` with no
   overrides, refusing to start when something already listens on 50071).

## Acceptance criteria
- A1: `sandbox.py init` creates every §12 sequence and config — check: `python3 -I v3/trd/sandbox/sandbox.py init --check` (validates the tree).
- A2: with the fake tools, `init --dir <tmp>` builds a tree that passes `--check`, `drip --dir <tmp> --fps 1000` completes sequence A's 60 frames, `path --dir <tmp>` prints `<tmp>`, and `init --reset` recreates the tree — check: `cargo test -p trd --test sandbox`.
- A3: a `trd` started on that tree with `--sandbox`, the fake tools, port 0, `--lan-bind 127.0.0.1:0` and `--no-mdns` renders sequence B's three variants, fails sequence C, never shows D or E, and deletes G after rendering — check: `cargo test -p trd --test sandbox` (deadline 120 s as a hang guard, polling the snapshot).
- A4: the `develop`, `develop-daemon` and `run` targets exist and dry-run cleanly — check: `bash v3/scripts/check-makefiles.sh`.

## Tests
`v3/trd/crates/trd/tests/sandbox.rs` (new) for A2 and A3; `init --check`. Running `make develop`
interactively is the owner's manual check M-TRD-9 (§16).

## CI
None beyond the gate.

## Out of scope
Client sandboxes.

Depends on: TRD-20, TRD-21

---

### TRD-29 — System tests workflow for the daemon

## Goal
Every part of `trd` that only a real machine can exercise (the three installers, the per-user
service managers and mDNS) is tested on GitHub-hosted runners on every push to `develop`, nightly,
and inside every release run before it publishes.

## Context
§9.7, §9.5 (the `system-tests` job), §11.3 (`[watch.ci]` files a ticket when a run on `develop`
goes red), overview §15. The live tests come from TRD-19 and TRD-21; the packages from TRD-24–27.

## Files
- `v3/1-trd-core-daemon.md` (this spec; §9.5, §9.7)
- `.github/workflows/v3-system-tests.yml` (new)
- `.github/workflows/v3-release.yml`
- `v3/trd/scripts/wait-for-trd.sh` (new)
- `v3/trd/scripts/test-wait-for-trd.sh` (new)

## Approach
1. `v3-system-tests.yml` with the triggers, the `version` job, the build jobs and the four daemon
   test jobs of §9.7.
2. `scripts/wait-for-trd.sh [--trd PATH] [--address A] [--timeout S]` polls `trd status --plain`
   until it answers, exiting non-zero after the timeout (every install job uses it; bash 3.2;
   works in Git Bash).
3. Add the `system-tests` job to `v3-release.yml` and `system-tests` to `publish.needs` (§9.5).
4. Dispatch `v3-system-tests.yml` and fix until green; dispatch `v3-release.yml` and check that its
   `system-tests` job tests that run's own packages.

## Acceptance criteria
- A1: a dispatch of `v3-system-tests.yml` is green with all four daemon jobs on every runner of their matrices — check: the run.
- A2: in a dispatch of `v3-release.yml`, the called system tests skip their build jobs and test the packages that run built — check: the run's job graph.
- A3: `wait-for-trd.sh` exits 0 once a stub `trd` answers and non-zero after the timeout when it never does — check: `bash v3/trd/scripts/test-wait-for-trd.sh` (stub `trd` first on `PATH`).

## Tests
The workflow run; `v3/trd/scripts/test-wait-for-trd.sh`.

## CI
`v3-system-tests.yml` (new), `v3-release.yml`.

## Out of scope
Client system tests (specs 2–4 add their jobs).

Depends on: TRD-19, TRD-21, TRD-27

---

### TRD-GATE — End-to-end acceptance, media CI job, coverage and API freeze

## Goal
`trd` is proven end to end with real ffmpeg and exiftool, the media job guards it in CI, the
coverage and suite floors are raised, the system tests are green, and API version 1 is frozen for
the clients.

## Context
Every section of this spec; §9.6; §12 (the sandbox tree, reused on port 0); §16 (test plan);
overview §15.3 (coverage floors).

## Files
- `v3/1-trd-core-daemon.md` (this spec; §9.6, §12, §16)
- `v3/trd/scripts/ci-media.sh` (new)
- `v3/trd/crates/trd/tests/e2e.rs` (new)
- `v3/trd/coverage-floors.toml`
- `.github/workflows/v3-gate.yml`
- `.mason/mason.toml`
- `v3/proto/README.md`

## Approach
1. `tests/e2e.rs` (runs only with `TRD_E2E=1`): build the sandbox tree in a temp dir
   (`python3 -I v3/trd/sandbox/sandbox.py init --dir <tmp>`), start the `trd` binary with
   `--sandbox --config <tmp>/config.json --data-dir <tmp>/data --log-dir <tmp>/logs --port 0 --lan-bind 127.0.0.1:0 --no-mdns`,
   read `TRD_LISTENING`, run `sandbox.py drip --dir <tmp> --fps 50` in the background, and check
   with the tonic client:
   - sequence B renders all three variants; the files exist with the §6.5 names; ffprobe reports
     the sizes 1080×720, 720×480 and 320×213, the original-size one with DAR 3:2; exiftool reads
     the XMP `Make`, `Model` and `ISO` from each; `recently_rendered` lists them newest first with
     thumbnails;
   - sequence C renders all 24 frames, shows no failure card, and the log tail of its render
     records the decode error (the owner accepted this real behaviour in place of a
     `FAILURE_SOURCE_FRAME_UNREADABLE` failure);
   - sequence D never appears; E never appears; F renders from its backup count; G is deleted
     after rendering;
   - after drip finishes, sequence A renders; during A's first render, `SuspendRender` then
     `ResumeRender` change its status; `StopRender` on another variant pauses it in `config.json`;
     `RetryVariant` renders it;
   - `UpdateConfig` adding a variant makes it render for already-rendered sequences;
   - pairing over the LAN listener with the pinned fingerprint, then `WatchSnapshots` with the token;
   - stopping `trd` mid-render and starting it again with the same flags and data directory
     re-adopts the render and finishes it (`--sandbox` implies only `--no-adopt-foreign`, so
     `trd` re-adopts renders from its own database and never looks at the owner's).
2. `ci-media.sh`: `cargo test -p trd --test e2e` with `TRD_E2E=1` and the real-tool tests of TRD-10.
3. Add the `trd-media` job to `v3-gate.yml` (§9.6).
4. Raise `min_tests` of `trd-test`, `trd-check` and `trd-e2e` in `.mason/mason.toml` to 90 % of
   the measured counts.
5. Raise `v3/trd/coverage-floors.toml` to `trd-core = 80` and `trd = 60` (overview §15.3), adding
   tests for whatever is uncovered until the gate's `trd-coverage` job passes.
6. Walk the §16 test plan and add any test it names that does not exist yet, until
   `python3 -I v3/scripts/check-test-names.py --spec 1` passes. From now on the gate's `t0` job
   runs it for spec 1 on every batch (`--completed` recognises `ci-media.sh`).
7. `v3/proto/README.md`: "API version 1 is frozen as of <date>; changes need `api_version` 2 and
   a new proto package `trd.v2`."

## Acceptance criteria
- A1: every check in step 1 passes on the owner's Mac and in `trd-media` on Linux and macOS — check: `bash v3/scripts/suite.sh trd-e2e` and the gate run.
- A2: `v3-release.yml` dispatch is green, including its `system-tests` job — check: the run.
- A3: the suite floors are raised — check: `.mason/mason.toml`.
- A4: the gate's `trd-coverage` job passes with floors of 80 % (`trd-core`) and 60 % (`trd`) — check: the gate run.
- A5: `v3-system-tests.yml` is green on `develop` — check: the latest run.
- A6: every test that spec 1's acceptance criteria and §16 table name exists under that name — check: `python3 -I v3/scripts/check-test-names.py --spec 1`.
- A7: the re-adoption check of step 1 re-adopts only the e2e daemon's own render: a fake-ffmpeg started by the test (not by `trd`) with foreign-render arguments is never listed in the snapshot — check: test `e2e_ignores_foreign_renders` in `tests/e2e.rs`.

## Tests
`tests/e2e.rs`; whatever step 5 adds.

## CI
`v3-gate.yml` (adds `trd-media`).

## Out of scope
Client code. The manual checks of §16.1 (the owner runs them before the first release).

Depends on: TRD-01, TRD-02, TRD-03, TRD-04, TRD-05, TRD-06, TRD-07, TRD-08, TRD-09, TRD-10, TRD-11, TRD-12, TRD-13, TRD-14, TRD-15, TRD-16, TRD-17, TRD-18, TRD-19, TRD-20, TRD-21, TRD-22, TRD-23, TRD-24, TRD-25, TRD-26, TRD-27, TRD-28, TRD-29

## 14. Dependency summary

| Ticket | Depends on |
|---|---|
| TRD-00 | — (manual, before filing) |
| TRD-01 | — |
| TRD-02 | 01 |
| TRD-03 | 01 |
| TRD-04 | 01 |
| TRD-05 | 01 |
| TRD-06 | 03, 04 |
| TRD-07 | 03, 05 |
| TRD-08 | 04, 05 |
| TRD-09 | 03, 04, 05 |
| TRD-10 | 09 |
| TRD-11 | 07 |
| TRD-12 | 06, 07, 08, 09, 10, 11 |
| TRD-13 | 12 |
| TRD-14 | 04, 05 |
| TRD-15 | 01 |
| TRD-16 | 12, 13, 14, 15 |
| TRD-17 | 02, 16 |
| TRD-18 | 17 |
| TRD-19 | 18 |
| TRD-20 | 18 |
| TRD-21 | 17 |
| TRD-22 | — |
| TRD-23 | — |
| TRD-24 | 21, 22, 23 |
| TRD-25 | 21, 22, 23 |
| TRD-26 | 21, 22, 23 |
| TRD-27 | 24, 25, 26 |
| TRD-28 | 20, 21 |
| TRD-29 | 19, 21, 27 |
| TRD-GATE | every ticket above except TRD-00 |

TRD-22 and TRD-23 can start at once, in parallel with the Rust work.

## 15. Definition of done

- Every ticket's acceptance criteria hold and `TRD-GATE` has landed on `develop`.
- `v3-gate.yml` is green on `develop`, including `trd-media`, `trd-coverage` (at the overview
  §15.3 floors) and `t0`.
- `v3-system-tests.yml` is green on `develop`.
- A dispatch of `v3-release.yml` is green; the owner can tag `v3.0.0-alpha.1` to get a draft
  release with the macOS, Linux and Windows `trd` packages.
- Before that release is published, the owner has run the §16.1 manual checks and recorded the
  results on the TRD-GATE issue (overview §15.4).

## 16. Test plan

Every area of `trd`, the tests that verify it, and where they run. "Same" means the row above:
the mason `trd-test` suite on the owner's Mac and the gate's `trd` job on macOS, Linux and Windows.

| Area | Automated tests | Kind | Ticket | Runs in |
|---|---|---|---|---|
| bootstrap scripts, Makefile contract, workflows, mason config, test-name checker | `test-scripts.sh`, `check-makefiles.sh`, `check-test-names.py`, actionlint, `mason definition check` | static | TRD-00 | mason `t0-*`; gate `t0` (all but `mason definition check`) |
| CLI skeleton, coverage tooling, system-test rules | `tests/cli.rs`, `test_coverage.py`, `test-check-system-tests.sh`, `check-system-tests.sh` | unit, static | TRD-01 | `trd-test`, gate `trd`; gate `trd-coverage` |
| API definition | `proto_matches_spec.rs`, prost round trip | unit | TRD-02 | same |
| config and codec table | `tests/config.rs`, including the repository's real configs | unit | TRD-03 | same |
| directories, logging, database | `tests/store.rs`, `dirs.rs` tests | unit | TRD-04 | same |
| tool resolution, fake tools, system-test guard | `tools.rs` tests, `tests/fakes.rs`, `system_test.rs` tests, `tests/system_test_macro.rs` | unit | TRD-05 | same |
| scanner | `tests/scan.rs` | unit | TRD-06 | same |
| render planning | `tests/plan.rs`: golden argument vectors, the Swift tests, names of the earlier daemons | unit | TRD-07 | same |
| process runner, process tables | `tests/process.rs`; `process.rs` tests | integration, unit | TRD-08 | same (three OSes) |
| EXIF collection | `tests/exif.rs` | unit | TRD-09 | same |
| XMP embedding | `tests/xmp.rs`; the real-exiftool test under `TRD_E2E` | unit, e2e | TRD-10 | same; mason `trd-e2e`, gate `trd-media` |
| failure diagnosis | `tests/diagnosis.rs` | unit | TRD-11 | same |
| queue, completion pipeline, controls | `tests/queue.rs` | integration | TRD-12 | same |
| re-adoption, `--no-adopt-foreign` | `tests/adopt.rs` with `FakeProcessTable`; `check-system-tests.sh` R2 | unit, static | TRD-13 | same |
| thumbnails, Swift history import | `tests/thumbs.rs` | unit | TRD-14 | same |
| Full Disk Access guard | `fda.rs` tests; `fda_blocks_scanning` (TRD-16) | unit, integration | TRD-15 | same; manual M-TRD-2, M-TRD-3 |
| engine, snapshots, problems | `tests/engine.rs` | integration | TRD-16 | same |
| gRPC server | `tests/server.rs`, every RPC | integration | TRD-17 | same |
| LAN listener, TLS, pairing | `tests/lan.rs` | integration | TRD-18 | same |
| mDNS | builder, re-registration and `registrar_refuses_in_unit_tests` tests; `check-system-tests.sh`; `tests/mdns_live.rs` | unit, static, system | TRD-19 | same; system `trd-live` (Linux); manual M-TRD-8 |
| CLI commands, `trd status` | `tests/cli_client.rs` | integration | TRD-20 | same |
| service install | per-OS tests with `RecordingRunner`, `system_runner_refuses_in_unit_tests`; `check-system-tests.sh`; `tests/service_live.rs` | unit, static, system | TRD-21 | same; system `trd-live` (three OSes); manual M-TRD-6 |
| ffmpeg build | `v3/ffmpeg/smoke_test.sh` | smoke | TRD-22 | mason `ffmpeg-smoke`; `v3-ffmpeg.yml` (four targets) |
| exiftool bundle | `v3/exiftool/smoke_test.sh` | smoke | TRD-23 | mason `exiftool-smoke`; `v3-exiftool.yml` (three OSes) |
| macOS package | `build-pkg.sh --self-check`, `packaging/macos/test-scripts.sh`; a real install | packaging, system | TRD-24 | `ci.sh` on macOS; system `trd-install-macos`; manual M-TRD-1, M-TRD-5 |
| Linux package | `build-deb.sh --self-check`, `test-deb-in-container.sh`; a real install with systemd | packaging, system | TRD-25 | `v3-trd-build.yml`; system `trd-install-linux` (amd64, arm64) |
| Windows installer | `test-installer.sh`; a real install with the task | packaging, system | TRD-26 | `v3-trd-build.yml`; system `trd-install-windows` |
| build and release workflows | dispatch runs; `test-release-version.sh` | workflow | TRD-27 | `v3-release.yml` dispatch |
| sandbox | `tests/sandbox.rs` with the fake tools | integration | TRD-28 | same; manual M-TRD-9 |
| system tests workflow | its dispatch run; `test-wait-for-trd.sh` | system | TRD-29 | `v3-system-tests.yml` |
| the whole daemon with real tools | `tests/e2e.rs` | e2e | TRD-GATE | mason `trd-e2e`; gate `trd-media` (Linux, macOS) |
| coverage | `coverage.sh` with floors 80 % (`trd-core`) and 60 % (`trd`) | coverage | TRD-01, TRD-GATE | gate `trd-coverage` |
| every named test exists | `check-test-names.py --spec 1` | static | TRD-GATE | gate `t0` on every batch once TRD-GATE has landed |

### 16.1 Manual checks (the owner, on the Mac)

| ID | Steps | Expected result |
|---|---|---|
| M-TRD-1 | Install the release `.pkg` from the draft release. | Gatekeeper opens it without a warning; the installer finishes; `trd status` answers within a minute; macOS asks at most once to allow incoming connections and local network access, while the installer is still open. |
| M-TRD-2 | Before granting Full Disk Access, run `trd status`. | Phase `WAITING_FOR_DISK_ACCESS`, the "Full Disk Access is needed" problem, no sequences listed, and no permission prompt. |
| M-TRD-3 | Grant Full Disk Access to `/Applications/Timelapse Render Daemon.app`, wait 30 s. | Scanning starts; sequences on `/Volumes/*` and `/mammoth` appear; no permission prompt appears then or later. |
| M-TRD-4 | Let a real LRTimelapse export finish with the real config. | The configured variants render with the same names the Swift and Perl daemons used (`_720w_`, `_OriRes_`, `_2160p_` …); `ffprobe` reports DAR 3:2 for the original-size render; `exiftool -XMP:all` shows the camera fields. |
| M-TRD-5 | Install a newer `.pkg` while a render is running. | The render continues and finishes; the log shows it was re-adopted; the video plays to the end. |
| M-TRD-6 | Log out and in (or restart the Mac). | `trd` runs again without any prompt; Full Disk Access is still granted. |
| M-TRD-7 | Start `trd` for the first time on the Mac where the Swift GUI has run. | Recently rendered lists the Swift daemon's renders of the last 30 days. |
| M-TRD-8 | Run `dns-sd -B _trd._tcp`, then `dns-sd -L <name> _trd._tcp`. | The service appears with the §8.4 TXT records and port 50072. |
| M-TRD-9 | Run `make -C v3/trd develop`. | The sandbox starts on 50081/50082; sequence B renders within a minute, C renders all 24 frames with no failure card, A fills as frames drip in and renders when complete; `trd status --address 127.0.0.1:50081 --watch` shows it all with new entries at the top. |
