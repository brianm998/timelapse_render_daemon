# Timelapse Render Daemon — Swift rewrite

A Swift rewrite of the Perl `timelapse_render_daemon` from the parent
directory.  Same `config.json` format; faster startup; non-fatal handling of
unreachable mounts; pause / resume controls exposed to a SwiftUI GUI.

## Layout

```
rewrite/
├── TimelapseRenderCore/   # cross-platform business logic library
├── cli/                   # command-line app (macOS, Linux, Windows)
└── gui/                   # SwiftUI app (macOS only)
```

Each is its own SwiftPM package.  `cli` and `gui` depend on
`TimelapseRenderCore` via a relative path.

## What's improved over the Perl version

- **Faster startup.**  The Perl daemon synchronously walked every backup path
  before doing anything.  The Swift daemon kicks that work off in a background
  task and starts scanning sources / exports immediately; backup data is folded
  in as it becomes available.
- **Faster change detection.**  Default scan interval is 2 seconds (was 12),
  driven concurrently across export paths.  The whole scan loop is async, so a
  slow path doesn't gate the others.
- **Survives unreachable mounts.**  Every filesystem operation is in a
  per-path concurrent task with a timeout; failures are logged as warnings
  and never propagate as fatal errors.
- **Edge case the Perl script missed.**  If the output video for a sequence
  already exists in `master_videos`, an `export_paths` directory, or the same
  export dir, the corresponding ffmpeg job is skipped (the original handles
  this for some paths, but the Swift version emits a single time-stamped
  "already rendered" log line per sequence and never re-attempts).
- **Pause controls.**  An entire sequence or one render variant can be paused
  at runtime from the GUI; pauses can also be persisted to `config.json` via
  the `pause` / `resume` CLI subcommands so they survive restarts.

## Building

Requires Swift 5.9 or newer.

Top-level `Makefile` delegates to each sub-package:

```bash
make build      # debug build of all three packages
make test       # unit tests
make release    # full multi-platform release (calls ./release.sh)
make clean      # wipe all .build directories
```

Or per-package:

```bash
cd TimelapseRenderCore && make {build,test,release,clean}
cd cli                && make {build,release,install,clean}
cd gui                && make {build,release,clean}
```

The CLI requires `ffmpeg` and `exiftool` on `PATH`.

## Running the GUI

```bash
gui/.build/debug/TimelapseRenderGUI                            # opens with no config loaded
gui/.build/debug/TimelapseRenderGUI -c /path/to/config.json    # auto-load + auto-start
gui/.build/debug/TimelapseRenderGUI --config ../../config.json # same, long form
```

The config path **must** be passed as a `-c` / `--config` flag, not as a
bare positional argument.  AppKit intercepts positional argv entries on
launch as "documents to open"; the resulting `application(_:openFiles:)`
dispatch causes SwiftUI's WindowGroup to suppress its primary window and
the GUI never appears.  Flag-shaped arguments are routed through
NSUserDefaults' command-line override mechanism and don't hit that path.

## Releasing

The release scripts mirror the layout used by
`nighttime_timelapse_airplane_remover`.

```
rewrite/
├── VERSION                       # plain-text project version
├── release.sh                    # multi-platform orchestrator
├── release_darwin.sh             # builds CLI + GUI on macOS
├── release_linux.sh              # builds CLI .deb on Linux
├── release_windows.sh            # builds CLI .zip on Windows
├── release_config.json.example   # template for remote build hosts
├── cli/release.sh                # CLI .pkg on macOS (signed/notarized)
└── gui/release.sh                # GUI .app + .pkg on macOS (signed/notarized)
```

`./release.sh` runs the local platform's release script, optionally SSHes
into the hosts in `release_config.json` to run their platform's script in
parallel, and collects all artifacts under `releases/<version>/`.  The git
tag is created only after every build passes.

### Signing / notarization (macOS)

Per-package release scripts code-sign and notarize the `.pkg` (and `.app`).
They expect a `notarytool` keychain profile named `timelapse-render` and a
Developer ID certificate.  Override or skip with environment variables:

```bash
SKIP_SIGN=1 ./cli/release.sh                          # produce unsigned .pkg
NOTARY_PROFILE=other ./gui/release.sh                 # different keychain profile
SIGN_IDENTITY="Developer ID Installer: Foo (TEAMID)" ./cli/release.sh
```

To create the keychain profile:
```bash
xcrun notarytool store-credentials timelapse-render \
      --apple-id you@example.com --team-id TEAMID
```

## CLI usage

```bash
timelapse-render run -c /path/to/config.json
timelapse-render scan -c /path/to/config.json    # one-shot, prints discovered sequences
timelapse-render pause my-sequence-name          # pause a whole sequence
timelapse-render pause my-sequence-name --variant ProResMQ_3840x2160_30
timelapse-render resume my-sequence-name
timelapse-render show-config                     # dump parsed config
```

Use `--plain-log` if redirecting to a file or piping to another process; the
default in-place updateable log uses ANSI cursor movement.

## Config

The same `config.json` used by the Perl daemon works unchanged.  The Swift
version recognises a few extra optional keys:

- `scan_interval_seconds` *(default: 2)* — how often to walk source / export
  paths.  The Perl daemon's `sleep_time_seconds` is still read but only used
  when there is no work to do.
- `image_extensions` *(default: ["tif", "tiff"])* — explicit list of output
  image extensions, replacing the hardcoded `tif`/`tiff` regex.
- `paused_sequence_names` and `paused_variants` — persisted pause set,
  managed by the `pause` / `resume` CLI subcommands.

### `$YEAR` path placeholder

Any path entry in `source_paths`, `backup_paths`, `export_paths`, or
`master_videos` can contain the literal token `$YEAR`.  At scan time the
daemon expands it into one path per year from the current year back to 1970,
keeps only the expansions that resolve to a real directory, and uses those.

So instead of:

```json
"backup_paths": [
  "/mammoth/Photos/timelapses/2026",
  "/mammoth/Photos/timelapses/2025",
  "/mammoth/Photos/timelapses/2024"
]
```

you can write:

```json
"backup_paths": ["/mammoth/Photos/timelapses/$YEAR"]
```

and the daemon discovers which years actually exist on disk.  This avoids
overlapping/duplicate entries and removes the need to edit the config every
January.

## Architecture

`RenderDaemon` (an actor) owns all state and exposes:

- `run()` — the main loop; runs until cancelled.
- `currentConfig()`, `updateConfig(_:persist:)` — read / replace the config.
- `pauseSequence`, `resumeSequence`, `pauseVariant`, `resumeVariant`.
- `stateStream: AsyncStream<DaemonStateSnapshot>` — every state change is
  re-published here.  Both the CLI updateable log and the SwiftUI app
  consume this stream.

Snapshots (`DaemonStateSnapshot`) are immutable, `Sendable`, and contain
everything a UI needs to render a status view: sequence progress, render
progress, paused entries, current phase.

The CLI either consumes the stream silently while the in-process
`UpdateableLog` writes to stdout, or — with `--plain-log` — renders snapshots
as plain text.

The GUI's `AppViewModel` (an `@Observable` class) owns a `RenderDaemon` and
republishes its `stateStream` for SwiftUI views.  Modifying the config in the
editor and pressing `Save & Apply` writes to disk and pushes the new config
into the live daemon.
