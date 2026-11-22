<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Timelapse Render Daemon v3 — overview, shared rules and ticket filing

This document is shared by the four v3 specifications. Read it first, then the spec you are
working from:

| File | Component | Ticket prefix |
|---|---|---|
| `v3/0-overview.md` | shared decisions, information model, rules, ticket format (this file) | — |
| `v3/1-trd-core-daemon.md` | `trd`, the Rust core daemon, its bundled ffmpeg and exiftool, the mason bootstrap | `TRD-` |
| `v3/2-trm-desktop.md` | Timelapse Render Monitor for desktop (Kotlin Compose), plus the Kotlin code shared with Android | `DSK-` |
| `v3/3-trm-ios.md` | Timelapse Render Monitor for iOS/iPadOS (SwiftUI) | `IOS-` |
| `v3/4-trm-android.md` | Timelapse Render Monitor for Android (Kotlin Compose) | `AND-` |

Everything in these five files is decided. If a ticket's text and this file disagree, this
file wins; if two specs disagree, the lower-numbered spec wins. An implementer who finds a gap
picks the option closest to the Swift rewrite's behaviour (`rewrite/`), records the choice in
the commit message, and does not stop to ask.

## 1. What v3 is

v3 is the third implementation of the timelapse render daemon (after the Perl script
`timelapse_render_daemon` and the Swift rewrite in `rewrite/`). It lives in `v3/` beside the
other two. It has the same job: watch the folders LRTimelapse exports image sequences into,
work out when an exported sequence has as many frames as its raw sequence, collect EXIF
metadata from the raw frames, render every configured video variant with ffmpeg, embed the
metadata into each video, and show the user what is happening. The differences are:

- the daemon (`trd`) runs on its own, started automatically at login, with no client needed;
- clients (desktop, iOS, Android) are separate programs that watch and control it over gRPC;
- phones reach `trd` directly over the LAN after a one-time pairing issued from a desktop client;
- ffmpeg and exiftool are bundled, built and fetched by GitHub Actions;
- the UI is redesigned: in-progress work on top, recently rendered videos directly below it,
  in one list that never scrolls away from new entries.

## 2. Decisions

These were decided by the owner on 2026-11-22 and are not open for re-discussion in tickets.

| # | Topic | Decision |
|---|---|---|
| D1 | Mobile access | Phones discover `trd` over mDNS and talk to it directly over TLS, pinned to `trd`'s self-signed certificate. A phone is authorized once with a 6-digit pairing code that only a client on `trd`'s own machine (loopback) or the `trd pair` command can issue. After pairing, phones work with no desktop client running. |
| D2 | Autostart | `trd` starts at login and runs as the logged-in user: a LaunchAgent on macOS, a systemd **user** unit on Linux, a Task Scheduler logon task on Windows. It is never a system service. On macOS it is a Developer ID–signed, notarized app bundle with a stable identity, so the owner grants Full Disk Access once and is never prompted for disk access later (in particular not in the middle of the night). Without Full Disk Access, `trd` on macOS scans nothing and reports the problem (spec 1 §6.12). |
| D3 | Platforms | `trd`: macOS universal (arm64 + x86_64), Linux x86_64 and arm64, Windows x86_64. Desktop client: macOS (arm64 and x86_64 builds), Linux x86_64, Windows x86_64. iOS/iPadOS 18+. Android 8.0+ (API 26). |
| D4 | Config | `trd` reads and writes the existing `config.json` format (same keys as the Perl and Swift daemons, including the historical `max_cuncurrent_renders` spelling, `$YEAR` placeholders and `paused_*` keys) and `video_codecs.json`. Unknown keys are preserved on write. |
| D5 | EXIF/XMP | exiftool is bundled (pinned version, SHA-256 verified, fetched by CI). The sidecar JSON and XMP output match the Swift rewrite. |
| D6 | ffmpeg | The ffmpeg build recipe from the star repository (`ffmpeg/build.sh`, `lib/`, `versions.env`, `smoke_test.sh`) is ported into `v3/ffmpeg/`: FFmpeg 9.0.2, static, GPL, x264, x265, SVT-AV1, dav1d, libvpx, VideoToolbox on macOS, NVENC/AMF/Vulkan on Windows and Linux. |
| D7 | Mobile controls | Full parity with the desktop: view everything, all render controls, and edit the whole config (paths chosen with a folder browser served by `trd`). The only desktop-only functions are pairing-code issue and device revocation, which require loopback (D1), and opening a video file, which needs a local path. |
| D8 | Previews | `trd` makes a JPEG poster frame for every video it renders; every client shows it in Recently rendered. No in-app playback. |
| D9 | Store delivery | Version tags upload iOS to TestFlight and Android to the Google Play internal testing track, using the same secret names as ReelVault. |
| D10 | Mason setup | Spec 1 begins with a bootstrap ticket (`TRD-00`) that is done by hand, not by mason; it adds `.mason/`, the issue template, the CI gate and the repo `CLAUDE.md`. The owner installs the registration file. |
| D11 | Identifiers | Prefix `com.timelapserender`: daemon `com.timelapserender.trd`, clients `com.timelapserender.monitor`, mDNS `_trd._tcp`, client display name "Timelapse Render Monitor". |
| D12 | Licence | GPL-3.0-or-later for everything in `v3/`, with an SPDX header in every source file. |
| D13 | `[720]` | A one-number `resolution` means **width** (the Swift behaviour): height follows the source aspect ratio, and the file name uses `720w`. `[W,H]` is an exact size (file name `<H>p`); no resolution is original size (`OriRes`). |
| D14 | Recently rendered | One row per finished video, newest first, added the moment that video completes. |
| D15 | Notifications | The desktop client posts OS notifications (sequence finished, render failed) and has a tray/menu-bar icon. Phones show the same events when opened; there is no push. |
| D16 | Makefiles | Every component Makefile has `run` (debug build against the real setup), `develop` (debug build in an isolated sandbox that never touches the installed `trd`), `release` (the same packages CI builds) and `clean`. |

Two behaviours of the Swift rewrite are deliberately **not** carried over, because they are
bugs:

- Original-resolution renders were tagged `-aspect 16/9` whatever the source shape, so 3:2
  sources play stretched (verified: a 6000×4000 Swift render reports SAR 32:27, DAR 16:9; the
  Perl render of 7008×4672 reports 1:1, 3:2). `trd` computes the aspect ratio from the real
  source dimensions, as the Perl daemon did.
- A render that died mid-way left a partial file under the final name, which then counted as
  "already rendered". `trd` renders to a hidden partial name and renames on success (spec 1 §6.6).

## 3. Architecture

```
 ┌───────────────────────────── trd's machine ─────────────────────────────┐
 │                                                                          │
 │  LaunchAgent / systemd user unit / logon task  ──starts──▶  trd daemon   │
 │                                                              │  │  │     │
 │   config.json, video_codecs.json ◀──reads/writes────────────┘  │  │     │
 │   SQLite (history, failures, jobs, devices, log) ◀─────────────┘  │     │
 │   ffmpeg / ffprobe / exiftool (bundled) ◀──spawns──────────────────┘     │
 │                                                                          │
 │   127.0.0.1:50071  gRPC, plaintext, no auth  ◀── desktop client, trd CLI │
 │   0.0.0.0:50072    gRPC over TLS (self-signed, pinned), bearer token     │
 │   mDNS  _trd._tcp  (TXT: id, name, fp, port, ver, api)                   │
 └──────────────────────────────────────┬───────────────────────────────────┘
                                        │ LAN
            ┌───────────────────────────┼───────────────────────────┐
            ▼                           ▼                           ▼
   desktop client on another    iOS / iPadOS client          Android client
   machine (paired like a phone)
```

- `trd` owns all state. Clients hold no state of their own beyond preferences and pairing
  tokens; everything they show comes from `trd`'s gRPC API (`v3/proto/trd/v1/trd.proto`,
  defined in spec 1 §7).
- The loopback listener is unauthenticated, as in ReelVault: any process of the logged-in user
  may control `trd`. The LAN listener requires a bearer token for every call except
  `GetServerInfo` and `Pair`.
- Clients get live state from one server-streaming call, `WatchSnapshots`, which sends a full
  `Snapshot` immediately and then on every change, at most a few times a second.

## 4. Names, identifiers, ports and paths

| Item | Value |
|---|---|
| Daemon executable | `trd` (`trd.exe` console and `trdw.exe` windowless on Windows) |
| macOS daemon app | `/Applications/Timelapse Render Daemon.app`, `CFBundleIdentifier` `com.timelapserender.trd`, `LSUIElement` true |
| LaunchAgent label | `com.timelapserender.trd` |
| Linux package / unit | package `trd`, files in `/usr/lib/trd/`, `/usr/bin/trd` symlink, unit `/usr/lib/systemd/user/trd.service` |
| Windows install / task | `%LOCALAPPDATA%\Programs\TimelapseRenderDaemon\`, scheduled task `TimelapseRenderDaemon` |
| Client app id | `com.timelapserender.monitor` (macOS bundle id of the desktop app, iOS bundle id, Android `applicationId`; Android debug builds add `.debug`) |
| Client display name | Timelapse Render Monitor |
| mDNS service type | `_trd._tcp.local.` |
| Loopback gRPC | `127.0.0.1:50071`, plaintext |
| LAN gRPC | `0.0.0.0:50072`, TLS |
| Sandbox ports (`make develop`) | `127.0.0.1:50081` and `0.0.0.0:50082` |
| Proto | `v3/proto/trd/v1/trd.proto`, package `trd.v1`, service `TrdService`, API version `1` |
| Java/Kotlin proto package | `com.timelapserender.trd.v1` |
| GitHub repository | `brianm998/timelapse_render_daemon`, integration branch `develop` |

Per-user locations of `trd` (spec 1 §6.13 is normative):

| | macOS | Linux | Windows |
|---|---|---|---|
| default config | `~/Library/Application Support/TimelapseRenderDaemon/config.json` | `$XDG_CONFIG_HOME/trd/config.json` | `%APPDATA%\TimelapseRenderDaemon\config.json` |
| data (DB, identity, thumbnails, render logs) | `~/Library/Application Support/TimelapseRenderDaemon/trd/` | `$XDG_DATA_HOME/trd/` | `%LOCALAPPDATA%\TimelapseRenderDaemon\data\` |
| logs | `~/Library/Logs/TimelapseRenderDaemon/trd.log` | `$XDG_STATE_HOME/trd/trd.log` | `%LOCALAPPDATA%\TimelapseRenderDaemon\logs\trd.log` |

## 5. Repository layout

```
.mason/mason.toml, .mason/landing.toml        mason pipeline (TRD-00)
.github/ISSUE_TEMPLATE/mason-ticket.md        ticket template (TRD-00)
.github/actionlint.yaml                       runner labels for the workflow linter (TRD-00)
.github/workflows/v3-gate.yml                 CI gate, dispatched by mason (TRD-00)
.github/workflows/v3-ffmpeg.yml               ffmpeg build (TRD-22)
.github/workflows/v3-exiftool.yml             exiftool fetch (TRD-23)
.github/workflows/v3-trd-build.yml            trd packages (TRD-27)
.github/workflows/v3-release.yml              release orchestrator (TRD-27)
.github/workflows/v3-system-tests.yml         installers, services, mDNS, packaged apps, simulator and emulator tests (TRD-29)
.github/workflows/v3-desktop-build.yml        desktop packages (DSK-13)
.github/workflows/v3-ios-build.yml            iOS archive + TestFlight (IOS-09)
.github/workflows/v3-android-build.yml        Android bundle + Play (AND-06)
CLAUDE.md                                     rules for agents in this repository (TRD-00)
v3/
  0-overview.md … 4-trm-android.md            these specifications
  instructions                                the owner's original brief
  README.md, VERSION, LICENSE, Makefile        (TRD-00)
  scripts/                                    suite runner, SPDX and workflow checks, version bump (TRD-00)
  proto/trd/v1/trd.proto                      the API (TRD-02)
  trd/                                        Rust workspace: crates/, packaging/, sandbox/, scripts/ (spec 1)
  ffmpeg/                                     ffmpeg recipe (TRD-22)
  exiftool/                                   exiftool fetch + smoke test (TRD-23)
  assets/icon/                                the daemon and monitor icons (TRD-24)
  clients/kotlin/                             Gradle build: proto/, core/, desktop/ (spec 2), android/ (spec 4)
  ios/                                        XcodeGen project, TRMKit package, app (spec 3)
```

Nothing in `v3/` changes the Perl daemon or `rewrite/`, and no v3 ticket edits them.

## 6. Shared information model and UI

All three clients show the same information from the same API, laid out for their platform.
This section is normative for all of them; the client specs say how each platform realises it.

### 6.1 The Activity view

The main screen of every client is one vertically scrolling list, the **Activity** view,
with a fixed header above it:

1. **Header** (does not scroll): server name; connection state (`Connected`,
   `Reconnecting…`, or `Offline — last update 14:02`); a phase chip from `Snapshot.phase`;
   the global pause switch (label "Pause all rendering"); and the counts line, e.g.
   `2 rendering · 1 queued · 3 exporting · 1 failed`.
2. **Problems** — only when `Snapshot.problems` is non-empty. One banner per problem, red for
   `ERROR` and amber for `WARNING`, showing `title`, `detail` and `fix`. The
   `FULL_DISK_ACCESS_MISSING` banner on a desktop client connected over loopback has a
   "How to fix" button (spec 2).
3. **Needs attention** — only when `Snapshot.failures` is non-empty. One card per failure,
   newest first: a red icon, the `kind_label` chip, the output file name (truncated in the
   middle), `summary` (wrapped), `×N` when `occurrences > 1`, "variant paused after repeated
   corrupt renders" when `variant_auto_paused`, and actions **Retry**, **Dismiss**, **Details**.
4. **In progress** — `Snapshot.in_progress` in the order the server sends it (spec 1 §6.10).
   One card per sequence:
   - line 1: sequence name (monospaced), stage chip, `paused` badge, actions menu;
   - line 2: progress bar of `exported_frames / expected_frames`, the text
     `1,234 / 1,500 frames`, and while exporting the rate and estimate
     (`41 frames/min · ~6 min left`), plus the export size when known;
   - line 3 (secondary text): the export path; `note` when set (e.g. a frame-count mismatch);
   - variant rows, shown once the sequence is complete: status icon, label
     (`ProResMQ · 2160p · 30 fps`), a progress bar (`frame / total_frames`, indeterminate when
     `progress_known` is false), `843 / 1,500 · 4.2 fps · ~3 min`, and controls. A finished
     variant shows a check mark and its size; `ALREADY_RENDERED` shows `exists`; `PAUSED_BY_USER`
     shows `paused` and Resume; `UNKNOWN_CODEC` shows an error chip; `FAILED` shows its message.
5. **Recently rendered** — `Snapshot.recently_rendered`, newest first, then a "Show older"
   row that pages through `ListRenderedVideos`. One row per video: poster thumbnail (16:9 box,
   letterboxed), output file name (truncated in the middle), variant label, size, frame count,
   duration (`frame_count / frame_rate`), rendered time as relative text ("12 min ago", with the
   absolute local time on hover or long-press), and the export path. When `file_exists` is
   false the row says `moved or deleted` in secondary text. Actions: **Open** and **Reveal**
   (desktop connected over loopback only), **Copy path**, **Details**.

Empty states: "Nothing in progress" and "Nothing rendered in the last N days" (N from the
config's `recent_render_history_days`).

A search field above the list filters every section by case-insensitive substring of the
sequence name and output file name. The desktop client also has an export-path filter. Filters
are client-side and persist per client.

### 6.2 Scrolling (normative — this fixes the Swift GUI's "scrolled too far" problem)

- **S1** On first display, on reconnect and on server switch, the list is at the top.
- **S2** While the first row is visible, the list stays at the top as rows are inserted, moved
  or removed; new entries appear in place and are seen immediately.
- **S3** When the user has scrolled down, the first visible row stays where it is (anchored by
  its key), and a floating pill `↑ N new` counts rows inserted above the viewport since the
  user scrolled away. Tapping it scrolls to the top; it disappears at the top.
- **S4** Row keys are stable: `Problem.id`, `FailureCard.id`, `SequenceCard.id`,
  `SequenceCard.id + "/" + VariantRow.variant_key`, `RenderedVideo.id`.
- **S5** A progress update never changes a key and never scrolls the list.
- **S6** Nothing ever scrolls the list to the bottom.
- **S7** When a sequence finishes, its card leaves In progress and its videos appear at the top
  of Recently rendered in the same snapshot, directly below the remaining in-progress cards.

### 6.3 Details

Selecting a card or row opens Details (a side pane on desktop and iPad/tablet, a pushed screen
on phones):

- sequence: every `SequenceCard` field, its variants, the raw sequence location and the controls;
- variant or render: everything `GetRenderDetails` returns (input pattern, start number, frame
  count, output and partial paths, the ffmpeg command line with a Copy button, the last 200
  lines of the render log, timings, pid, adopted flag);
- rendered video: every `RenderedVideo` field and a large poster;
- failure: the full report (`FailureCard` fields, tool output, command line) and **Copy report**.

### 6.4 Config editor (every client — D7)

One screen with these groups, editing `ConfigState.config` and `ConfigState.codecs`:

- **Locations**: `source_paths`, `backup_paths`, `export_paths`, `master_videos`. Each is a
  list with add (the server folder browser, `ListDirectory`), remove and reorder. `$YEAR` is
  allowed and explained in helper text.
- **Matching**: `image_name_prefix`, `image_sequence_dirname_prefix`, `raw_types`,
  `image_extensions`, `ignore_sequence_regexes`.
- **Rendering**: the variants table — codec (picker of codec names), frame rate, resolution
  mode (`Original`, `Width`, `Width × Height`), width, height; add, remove, reorder. The
  computed `variant_key` and an example output file name are shown per row.
- **Limits**: `max_processes.ffmpeg`, `max_processes.exiftool`, `max_concurrent_renders`.
- **Behaviour**: `scan_interval_seconds`, `delete_sequence_after_render_regexes`,
  `recent_render_history_days`.
- **Pauses**: `paused_sequence_names` and `paused_variants`, each removable.
- **Codecs**: the `video_codecs.json` table — each codec's ffmpeg fields and file-name fields;
  add, remove, rename (rename and remove are refused while a variant uses the codec).
- **Advanced**: the config file path (`SetConfigPath`), the raw JSON (read-only), and the
  legacy keys `log_in_color`, `progress_bar_width` (used by `trd status`) and
  `sleep_time_seconds` (kept for the older daemons, ignored by `trd`).

Issues from `trd` are shown next to the field they name (`ConfigIssue.field` is a JSON pointer
such as `/rendering/2/codec`). Save is disabled while any `ERROR` issue exists. Save sends
`UpdateConfig` with the `revision` that was loaded; a `FAILED_PRECONDITION` answer shows
"The config changed on the server" and reloads.

### 6.5 Log

A list of `LogEntry` rows (time, level, subject, message), newest first, live from
`WatchLog`, with a level filter (default: Info and above) and a search field. It follows the
same scrolling rules as Activity (§6.2).

### 6.6 Servers and pairing

- Desktop: the local server (`127.0.0.1:50071`) is used automatically when it answers. Servers
  found over mDNS are listed with **Pair…**.
- iOS and Android: discovered servers are listed; a server can also be added by hand
  (host, port and fingerprint, all shown in the desktop pairing dialog) when mDNS is blocked.
- Pairing: choose the server, type the 6-digit code shown on `trd`'s machine, done. The client
  stores the server id, name, last address, certificate fingerprint and token.
- Several servers can be paired; one is active at a time; the header has a server switcher.
- Forget (client side) removes the stored token. Revocation on the server is desktop-only (§6.7).

### 6.7 Devices (desktop connected over loopback only)

**Pair a phone** calls `StartPairing` and shows the code in large digits, a 5-minute countdown,
the server name, its LAN addresses and port, and the first 16 hex digits of the certificate
fingerprint. Below it, the paired devices (name, platform, paired at, last seen) with **Revoke**.

### 6.8 Notifications (desktop only — D15)

From `WatchLog` events: `SEQUENCE_FINISHED` → "LRT_x rendered — 3 videos (12.4 GB)";
`RENDER_FAILED` → "Render failed: <file> — <kind label>". A tray / menu-bar icon shows idle
(grey), rendering (green) or a failure present (red), a tooltip with the counts line, and a
menu: Open window, Pause all rendering / Resume rendering, Quit. Each notification type can be
switched off in preferences (both default on).

### 6.9 Control semantics (implemented by trd, spec 1 §6.8)

| Action | Target | Effect | Persisted in |
|---|---|---|---|
| Pause sequence | sequence | its queued, running and suspended renders end, partial files are deleted, nothing new is queued, the card shows `PAUSED` | `config.json` `paused_sequence_names` |
| Resume sequence | sequence | removed from the paused list; the next scan queues what is missing | same |
| Pause variant | sequence + variant | as above, for one variant | `config.json` `paused_variants` |
| Resume variant | sequence + variant | as above | same |
| Suspend render | running render | the ffmpeg process is suspended (SIGSTOP; `NtSuspendProcess` on Windows); status `SUSPENDED`; it keeps its slot | job record in the DB |
| Resume render | suspended render | the process continues (SIGCONT; `NtResumeProcess`) | job record |
| Stop render | queued, running or suspended render | the process ends, the partial file is deleted, and the variant is paused | `paused_variants` |
| Retry | failed or paused variant | its failure is dismissed, the variant pause and corruption strikes are cleared, and a scan runs at once | `config.json` + DB |
| Pause all rendering | everything | no queued render starts; every running render is suspended; EXIF jobs and scanning continue | DB setting |
| Resume rendering | everything | renders suspended by Pause all continue (individually suspended ones stay suspended); the queue runs | DB setting |
| Dismiss failure | failure | the card goes away until the same render fails again | DB |
| Rescan now | — | a scan runs at once | — |

## 7. Versions, tags and releases (summary; spec 1 §9 is normative)

- One version for all v3 components, in `v3/VERSION`, starting at `3.0.0`.
  `v3/scripts/bump-version.sh X.Y.Z[-pre]` updates every place that embeds it.
- Pushing a tag `v3.X.Y` or `v3.X.Y-<pre>` (it must equal `v3/VERSION`) runs
  `v3-release.yml`, which builds every component, signs and notarizes the macOS packages,
  uploads iOS to TestFlight and Android to the Play internal track, runs the system tests
  (§15) against the packages it just built, and only when they pass creates a **draft** GitHub
  Release (a pre-release when the version has a `-` suffix) for the owner to publish.
- `workflow_dispatch` of `v3-release.yml` runs the same builds without uploading or publishing.

## 8. Owner prerequisites (one-time, done by the owner, never by an agent)

1. Merge the specs and `TRD-00` to `develop`; install `~/.config/mason/projects/trd.toml`
   (content in spec 1 §11.2); restart `masond` (a new registration file needs a restart).
2. Add these repository secrets to `brianm998/timelapse_render_daemon` (same names and values
   as ReelVault where they exist):
   - macOS signing and notarization: `APPLE_CERTIFICATE_P12_BASE64` (Developer ID Application
     and Developer ID Installer identities with their keys), `APPLE_CERTIFICATE_PASSWORD`,
     `KEYCHAIN_PASSWORD`, `APPLE_TEAM_ID`, `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`,
     `APPLE_API_ISSUER_ID`;
   - iOS: `IOS_DIST_CERT_P12_BASE64`, `IOS_DIST_CERT_PASSWORD`, `IOS_PROVISION_PROFILE_BASE64`
     (an App Store provisioning profile for `com.timelapserender.monitor`);
   - Android: `KEYSTORE_BASE64`, `KEYSTORE_PASSWORD`, `KEY_ALIAS`, `KEY_PASSWORD`,
     `PLAY_SERVICE_ACCOUNT_JSON`.
3. Register the bundle id `com.timelapserender.monitor` and create the app record in App Store
   Connect; create the app `com.timelapserender.monitor` in the Play Console, grant the service
   account release permission, and upload the first AAB by hand (Play allows no API upload
   before that).
4. Install Android SDK Platform 36 and Build-Tools 36.0.0 with Android Studio's SDK Manager
   (this accepts their licences; agents must not accept licences).
5. After installing `trd` on the Mac for the first time, grant Full Disk Access to
   `/Applications/Timelapse Render Daemon.app` (System Settings → Privacy & Security → Full Disk
   Access). The desktop client's first-run screen walks through this.

## 9. Makefile contract (D16)

Every component directory has a `Makefile`. The components and their names are `v3/ffmpeg`
(`ffmpeg`), `v3/exiftool` (`exiftool`), `v3/trd` (`trd`), `v3/clients/kotlin` (`desktop`: the
desktop app and the shared Kotlin modules), `v3/clients/kotlin/android` (`android`) and `v3/ios`
(`ios`). Each Makefile has at least:

| Target | Meaning |
|---|---|
| `run` | debug build, run against the real setup (the installed `trd`, the user's real config, port 50071) |
| `develop` | debug build, run in the sandbox: a generated config, generated fixture folders, its own data directory and ports 50081/50082; never touches the installed `trd`, its config or its data |
| `release` | build the same packages CI builds for this host; signed and notarized only when the signing environment variables named in the spec are set |
| `clean` | delete every build output of the component |
| `test` | every automated test of the component (what mason runs) |
| `check` | formatting and lint checks |
| `ci` | exactly what the component's job in `v3-gate.yml` runs; implemented as a bash script under the component's `scripts/` so CI on Windows does not need `make`. For `ffmpeg` and `exiftool`, which have their own workflows instead of a gate job, it prints "built by its own workflow" and exits 0 |

`v3/Makefile` delegates `make release`, `make clean`, `make test`, `make check` and `make ci` to
every component directory that exists, in the order `ffmpeg exiftool trd desktop android ios`;
`make run` and `make develop` (which do not return) mean `make trd-run` and `make trd-develop`;
`make <component>-<target>` (for example `make desktop-develop`, `make android-run`) runs one
target of one component. It discovers components with `$(wildcard …)`, so adding a component
never requires editing it.

No Makefile runs a command at parse time (no `$(shell …)` that needs Xcode, adb, cargo or a
device), so `make -n <target>` works on every OS; `v3/scripts/check-makefiles.sh` relies on that
when it checks every Makefile on Linux in the gate.

## 10. Rules for every ticket and every agent

1. **Nothing appears on the owner's screen.** On the owner's Mac no XCUITest or UI-test bundle
   runs, no simulator or emulator is booted by a test or suite, no Compose window is opened by a
   test (UI tests render offscreen with `ImageComposeScene` / `runComposeUiTest` / Robolectric),
   no app is opened, no `caffeinate -u`. `make run` and `make develop` for the iOS and Android
   apps are for the owner to type; no ticket's tests call them. Tests that need a simulator, an
   emulator, a real service manager, real multicast or an installer run only on GitHub-hosted
   runners in `v3-system-tests.yml` (§15); every other test that starts `trd` passes
   `--no-mdns`.
2. **Real processes are off limits.** Nothing may signal, adopt or kill an ffmpeg process the
   test did not start; the owner often has production renders running. Every `trd` that a test
   or the sandbox starts, other than a system test's on a GitHub runner, runs with `--sandbox`
   (which implies `--no-adopt-foreign`: only renders recorded in its own database are
   re-adopted) or with `--no-adopt`, and in-process tests use `FakeProcessTable`. Spec 1's
   `check-system-tests.sh` enforces this for the daemon's tests, and the real process table
   panics when a unit test tries to use it.
3. **Real data is off limits.** No test reads or writes the owner's config, data directory,
   LaunchAgents, systemd units or scheduled tasks, or binds or connects to ports 50071/50072 (a
   `trd` client command in a test always passes `--address` with the test daemon's port). Tests
   use port 0 and temporary directories.
4. **Fixtures live in the system temporary directory**, never in the worktree: worktrees are on
   an external volume, and touching files there from a test process can raise a macOS privacy
   prompt.
5. **No timing assertions.** Use injected clocks and deterministic fakes; deadlines of 30–120 s
   that only guard against hangs are allowed.
6. **SPDX header** in every new source file (`.rs .kt .kts .swift .sh .py .proto .toml .yml
   .yaml`, Makefiles, `.plist`, `.xml` except generated files):
   `SPDX-License-Identifier: GPL-3.0-or-later` and
   `Copyright (C) 2026 Timelapse Render Daemon contributors`, in the file's comment syntax.
   Generated code (`*.pb.swift`, `*.grpc.swift`, anything under a `generated/` or `Generated/`
   directory) and JSON are exempt. `v3/scripts/check-spdx.sh` enforces this.
7. **Branches**: work starts from `origin/develop`; mason creates and lands branches. An
   interactive session puts worktrees in `.claude/worktrees/<branch-name>`.
8. **Tool paths on macOS**: scripts call `/usr/bin/lipo`, `/usr/bin/codesign`, `/usr/bin/xcrun`
   and `/usr/bin/plutil` by absolute path (MacPorts tools earlier on PATH behave differently).
9. **Shell scripts** are bash 3.2 compatible (no associative arrays, no `mapfile`). Anything
   that needs data structures is a Python 3 script run with `python3 -I`.
10. **Versions of third-party libraries**: where a spec names a version, use it. Where it says
    "newest stable", use the newest stable release available on the day of the ticket that is
    compatible with the other pinned versions, and pin it exactly (lockfile or version catalog).
11. **Every behaviour has a test.** Each ticket adds tests for every behaviour it implements, of
    the kind §15 assigns to that layer, besides the ones its `## Tests` section names. A
    behaviour without a test is not done, and the review step sends such work back. Every test
    that a ticket's acceptance criteria name exists under that name before the ticket lands:
    `python3 -I v3/scripts/check-test-names.py --ticket <ID>` passes. Tests never weaken an
    existing assertion or lower a coverage floor or a `min_tests` floor.

## 11. Ticket format

Every ticket in specs 1–4 is written in this structure, and the filed GitHub issue uses the
same structure:

```
---
explored: true
---
## Goal
One or two sentences: the outcome.

## Context
What exists, which spec sections to read, reference implementations.

## Files
- `v3/1-trd-core-daemon.md` (the spec; read §6.4 and §7)
- `v3/trd/crates/trd-core/src/scan.rs` (new)

## Approach
Numbered steps, specific enough to implement without exploring.

## Acceptance criteria
- A1: <observable outcome> — check: <test name or command>

## Tests
The tests the ticket adds.

## CI
The `v3-gate.yml` jobs and workflows that must be green, and any workflow the ticket changes.

## Out of scope
What not to touch.

Depends on #12, #15
```

Notes that make mason handle these tickets well:

- `explored: true` front matter (and the label `mason:explored`) skips mason's exploring step;
  the flag counts because the owner's account files the issues and the owner approves them.
  The Files, Approach and Acceptance criteria sections are the plan the develop step works from.
- In `## Files`, every file the ticket creates is on its own line ending in `(new)`; every other
  listed file exists on `develop` by the time the ticket starts (its dependencies created it).
- The `Depends on #…` line must be plain text (not in backticks or a code block), the last
  line of the body.
- Two tickets that name the same file are serialized by mason; the specs keep shared files to a
  minimum for that reason. The spec files, `rewrite/` (read-only reference) and `v3/assets/`
  (read-only for clients) are excluded from this by the registration's `hold_back_ignore`.
- Inside the spec files each ticket's sections use `##` headings so the text can be filed as the
  issue body unchanged.

## 12. Filing procedure (for the agent that files the tickets)

Preconditions: the five spec files are on `origin/develop`; `TRD-00` is merged to `develop`
(it is done by hand and is **not** filed); the owner has installed the mason registration
(§8). Then:

1. Run as the owner (the owner's `gh` login), from a checkout of `develop`.
2. Create any missing labels (`gh label create … --force` is idempotent):
   `mason` (`#5319e7`), `mason:explored` (`#c5def5`), `v3` (`#0e8a16`), `v3:trd`, `v3:desktop`,
   `v3:ios`, `v3:android` (`#bfdadc`).
3. File in this order: every ticket of spec 1 in the order listed (skipping `TRD-00`), then
   spec 2, then spec 3, then spec 4. Each spec lists its tickets in dependency order, so every
   dependency already has an issue when a ticket is filed.
4. Write the filing as one Python 3 script (bash 3.2 has no associative arrays). For each ticket:
   - a ticket is the text from its `### <ID> — <title>` heading to the next line that is exactly
     `---` (or the next `## <number>.` heading of the spec);
   - title: `[<ID>] <title>`, e.g. `[TRD-07] Render planning: names, frame patterns, aspect, ffmpeg argv`;
   - labels: `mason`, `mason:explored`, `v3`, and the component label (`v3:trd` for `TRD-`,
     `v3:desktop` for `DSK-`, `v3:ios` for `IOS-`, `v3:android` for `AND-`);
   - body: the `explored: true` front matter, then every line of the ticket after its heading
     unchanged, except the final `Depends on: <IDs>` line, which becomes
     `Depends on #<n>, #<m>` with the dependencies' issue numbers (and is dropped when it says
     `none`);
   - skip the ticket if an issue whose title starts with `[<ID>]` already exists in any state
     (the procedure is re-runnable);
   - resolve each dependency ID to its issue number with
     `gh issue list --repo brianm998/timelapse_render_daemon --state all --search "\"[<ID>]\" in:title" --json number,title`
     (take the issue whose title starts with `[<ID>]`); a missing dependency is an error that
     stops the script;
   - after creating the issue, also record each dependency as a native GitHub "blocked by"
     link: `gh api -X POST repos/brianm998/timelapse_render_daemon/issues/<new>/dependencies/blocked_by -F issue_id=<blocking issue's numeric id>`
     (the numeric `id` from `gh api repos/brianm998/timelapse_render_daemon/issues/<n>`); if the
     call fails, keep the body line and continue.
5. Print a table of ID → issue number at the end.
6. The owner then approves the imported tasks in mason (`mason task approve`); tasks whose
   blockers have not landed stay held until they do.

## 13. Cross-spec dependencies

"Every task in specs 2–4 depends on every task in spec 1" is implemented with one ticket:
`TRD-GATE` depends on every other `TRD-` ticket, and every ticket in specs 2–4 depends on
`TRD-GATE`. Android tickets additionally depend on the specific spec 2 tickets that build the
Kotlin code they reuse (listed per ticket in spec 4).

## 14. Glossary

| Term | Meaning |
|---|---|
| raw sequence | a directory of camera raw frames (`raw_types`, e.g. ARW, ORF, DNG) under a `source_paths` entry, or a `<name>.json` sidecar under a `backup_paths` entry that records its frame count |
| export sequence | a directory under an `export_paths` entry whose name contains `image_sequence_dirname_prefix` + the raw sequence's name, holding exported frames (`image_extensions`, e.g. tif) |
| complete | an export sequence whose frame count equals its raw sequence's frame count |
| variant | one entry of `rendering` (codec, frame rate, resolution); its `variant_key` is `<codec>_<res>_<fps>` with `<res>` = `WxH`, `Ww` or `ori` |
| render | one ffmpeg run producing one variant of one sequence |
| sidecar | `<raw parent>/<raw name>.json`: the EXIF histogram of a raw sequence, `{tag: {value: count}}` |
| adopted render | an ffmpeg process `trd` did not start in this session (its own from before a restart, or a foreign one from the Perl or Swift daemons) |
| problem | a condition the user must fix (missing tool, unreachable path, missing Full Disk Access …), shown as a banner |
| failure | a render that did not produce a usable video, with a diagnosis |
| contract test | a client library's tests run against a real `trd` built from the same commit, with fake media tools |
| system test | a test that needs a GitHub-hosted runner: installers, service managers, multicast, packaged apps, a simulator or an emulator |

## 15. Testing

Every component is verified by tests of at least one automated kind, and by a manual check
where only a person on the real machine or phone can tell. Each spec ends with a test plan that
maps every area of its component to the tests that verify it and to where they run.

### 15.1 Test levels

| Level | Proves | Tools | Runs in |
|---|---|---|---|
| static | formatting, lint, SPDX headers, workflow syntax, the Makefile contract, the bootstrap scripts; that every test the specs name exists; that system tests and real processes cannot be reached from ordinary tests | rustfmt, clippy, Kotlin `allWarningsAsErrors`, `swift build -warnings-as-errors`, `check-spdx.sh`, actionlint, `check-makefiles.sh`, `test-scripts.sh`, `check-test-names.py`, the two `check-system-tests.sh` scripts (spec 1 TRD-01, spec 2 DSK-04) | mason `t0-*` and `trd-check` suites; gate `t0`, `trd` and `kotlin`; `check-test-names.py --ticket` before each ticket lands (§10 rule 11) |
| unit | each module's logic, against fakes and injected clocks | `cargo test`, JUnit with kotlinx-coroutines-test, XCTest | mason component suites; gate component jobs (`trd` and `kotlin` on macOS, Linux and Windows) |
| integration | a component's parts wired together: the daemon over gRPC with fake tools; presenters and models over an in-process fake server | cargo integration tests, `FakeTrdService` (Kotlin and Swift) | same as unit |
| UI | every screen shows the right content and reacts to input, rendered offscreen | Compose Desktop offscreen tests, Robolectric Compose tests, SwiftUI `ImageRenderer` renders plus model-level assertions | same as unit |
| contract | each client library against a real `trd` from the same commit, with fake media tools: connection, snapshots, every control, config editing, pairing over pinned TLS, log events, thumbnails, the folder browser | Kotlin `:core` and Swift `TRMKit` contract suites (`TRM_E2E=1`) | gate `contract`; runnable locally with one script |
| end-to-end | the daemon with real ffmpeg and exiftool on generated sequences, including re-adoption across a restart | `crates/trd/tests/e2e.rs` (`TRD_E2E=1`) | mason `trd-e2e`; gate `trd-media` |
| smoke | the bundled ffmpeg and exiftool do everything `trd` asks of them | `v3/ffmpeg/smoke_test.sh`, `v3/exiftool/smoke_test.sh` | mason `ffmpeg-smoke`, `exiftool-smoke`; `v3-ffmpeg.yml`, `v3-exiftool.yml` |
| packaging | the packages contain what they must | `build-pkg.sh --self-check`, deb and installer self-checks | the build workflows |
| system | installers, real service managers, real mDNS, packaged desktop apps, app-hosted iOS tests on a simulator, Android instrumented tests on an emulator | `v3-system-tests.yml` | GitHub-hosted runners only: every push to `develop` touching v3, nightly, and inside every release run before publishing |
| coverage | how much code the unit, integration and UI tests execute | `cargo-llvm-cov`, Kover, `swift test --enable-code-coverage` | gate (`trd-coverage`, `kotlin`, `android`, `ios`), enforced floors |
| manual | what only the owner on the real Mac, phone or tablet can check | the checklists `M-TRD-*`, `M-DSK-*`, `M-IOS-*`, `M-AND-*` in each spec | the owner, before publishing a release |

### 15.2 Where tests run, and who sees a failure

- **mason** runs the suites of `.mason/mason.toml` (spec 1 §11.3) in a task's test step and in
  the landing batch; a red suite sends the task back to develop.
- **`v3-gate.yml`** is dispatched by mason for every landing batch; a batch lands only when it is
  green. It holds every test an agent can reproduce on the owner's Mac.
- **`v3-system-tests.yml`** holds the tests an agent cannot reproduce locally. It runs on every
  push to `develop` that touches v3 and nightly; mason watches it (`[watch.ci]`) and files a
  ticket when it goes red. `v3-release.yml` runs it against the packages it built and publishes
  only when it is green.
- System tests that would touch the machine they run on (installing services, multicast) stay
  out of ordinary test runs and need `CI=true` plus their own switch: `TRD_SERVICE_TEST=1`,
  `TRD_MDNS_TEST=1` or `TRM_MDNS_TEST=1`, so they cannot run on the owner's Mac by accident. The
  Rust ones are declared with `system_test!` (spec 1 TRD-05), which marks them ignored and makes
  them fail, not skip, when run without both variables; the Kotlin one skips itself without both
  (spec 2 DSK-04). Every part of this is tested: a unit test covers each guard's decision,
  `check-system-tests.sh` (spec 1 TRD-01, spec 2 DSK-04) proves that ordinary runs exclude or
  skip the system tests and that no other test reaches the real mDNS, service-manager or
  process-table code, and each system job fails unless its tests really ran. The simulator and
  emulator scripts refuse to run without `CI=true`, which the iOS and Android `ci` scripts check
  on the runner. `TRD_E2E=1` and `TRM_E2E=1` (real tools, contract tests) are safe locally: they
  use temporary directories, port 0, a LAN listener on `127.0.0.1`, `--sandbox` and no mDNS.

### 15.3 Coverage floors

Line coverage of each module's own sources by its automated tests, measured in the gate:

| Module | Floor | Set by | Raised to the floor by |
|---|---|---|---|
| `trd-core` | 80 % | TRD-01 (0 %) | TRD-GATE |
| `trd` (binary crate) | 60 % | TRD-01 (0 %) | TRD-GATE |
| `:core` (Kotlin) | 80 % | DSK-01 (0 %) | DSK-13 |
| `:desktop` | 50 % | DSK-01 (0 %) | DSK-13 |
| `TRMCore` | 80 % | IOS-01 (0 %) | IOS-09 |
| `TRMUI` | 40 % | IOS-01 (0 %) | IOS-09 |
| `:android` | 50 % | AND-01 (0 %) | AND-06 |

Generated code (proto stubs) and test helpers are excluded. A floor may be raised by any ticket
and lowered by none.

### 15.4 Manual checks

Each spec's test plan lists its manual checks with steps and expected results. They cover only
what automation cannot: macOS privacy grants on the owner's Mac, Gatekeeper on a fresh download,
real phones on the home Wi-Fi, store installs. Before publishing a release, the owner runs the
checklists of every component in it and records each result (pass, or fail with a note) as a
comment on that spec's final ticket (TRD-GATE, DSK-13, IOS-09, AND-06). They are not part of any
ticket's acceptance criteria, because an agent cannot perform them.
