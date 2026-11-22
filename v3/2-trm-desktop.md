<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Spec 2 — Timelapse Render Monitor for desktop (Kotlin Compose)

Read `v3/0-overview.md` first (especially §6, the information model every client follows) and
`v3/1-trd-core-daemon.md` §7 (the API). Every ticket here depends on `TRD-GATE`.

This spec also builds the Kotlin code shared with the Android client (`:proto` and `:core`);
spec 4 depends on DSK-01 to DSK-04 for it.

## 1. Outcome

- A Compose Desktop app, **Timelapse Render Monitor**, for macOS (arm64 and x86_64), Linux x86_64
  and Windows x86_64, that connects to the local `trd` automatically and to paired `trd`s on the
  LAN, and implements overview §6 completely: Activity, Details, Config editor, Log, Servers and
  pairing, Devices, notifications and tray.
- Shared Kotlin modules `:proto` (generated API code) and `:core` (connection, state, presenters,
  formatting, editor and browser models, discovery, pairing) used by this app and by Android.
- Signed and notarized macOS `.dmg`s, a Linux `.deb` and a Windows `.msi`, built by GitHub Actions
  on every `v3.*` tag and attached to the draft release.

## 2. What to read

- Overview §2, §4, §6, §9, §10; spec 1 §5 (`TRD_LISTENING`, ports), §7 (API), §8 (TLS, pairing,
  mDNS TXT records), §10.4 (`trd service` commands, used to start `trd`), §12 (the sandbox).
- ReelVault at `228e3c8b7937291b1d53314eb3df92132f6757d6` (URL form in spec 1 §2):
  `shared/src/main/kotlin/com/reelvault/data/remote/ChannelFactory.kt`, `PinnedTls.kt`,
  `ServerDiscovery.kt`, `PairingClient.kt`, `TokenStorage.kt`; `kotlin-desktop/build.gradle.kts`.
- star at `ae7ae6f2c2687c3c892832ce0c241a4e41d8f0d1`: `star-desktop/build.gradle.kts` (Compose
  Desktop on this machine, macOS signing with an identity and entitlements, `javaHome` override
  for jpackage) and `.github/workflows/kotlin-client.yml` job `macos-signed` (keychain, packaging,
  notarytool with an API key, stapling with retries).

## 3. Build layout

```
v3/clients/kotlin/
  settings.gradle.kts        includes :proto, :core, :desktop, and :android only when android/build.gradle.kts exists
  build.gradle.kts           every plugin declared once with `apply false`
  gradle/libs.versions.toml  every version for desktop and Android (below)
  gradle/wrapper/            Gradle wrapper
  gradle.properties          org.gradle.jvmargs=-Xmx4g, parallel, caching, android.useAndroidX=true
  Makefile                   the desktop component (overview §9)
  scripts/ci-desktop.sh      what v3-gate.yml's `kotlin` job runs
  scripts/mason-desktop.sh   the `kotlin-test` mason suite
  proto/                     :proto — kotlin("jvm") + com.google.protobuf; generates from ../../../proto
  core/                      :core — kotlin("jvm") + java-test-fixtures; JVM bytecode 17
  desktop/                   :desktop — kotlin("jvm") + org.jetbrains.compose + compose compiler plugin
  android/                   :android — spec 4
```

Why no Kotlin Multiplatform: both clients run on the JVM, so plain JVM modules are shared
directly (as ReelVault does); each client has its own Compose UI, and the presenters in `:core`
keep that UI thin.

Versions (`gradle/libs.versions.toml`), starting set:

| Entry | Version |
|---|---|
| Kotlin (jvm, android, compose compiler plugin) | 2.1.21 |
| Compose Multiplatform (`org.jetbrains.compose`) | 1.8.2 |
| Android Gradle Plugin | 8.10.1 |
| Gradle wrapper | 8.11.1 |
| `com.google.protobuf` Gradle plugin | 0.9.4 |
| protobuf (`protoc`, `protobuf-javalite`, `protobuf-kotlin-lite`) | 3.25.5 |
| grpc-java (`grpc-okhttp`, `grpc-protobuf-lite`, `grpc-stub`, `protoc-gen-grpc-java`; tests: `grpc-inprocess`, `grpc-testing`) | 1.68.1 |
| grpc-kotlin (`grpc-kotlin-stub`, `protoc-gen-grpc-kotlin` `jdk8@jar`) | 1.4.1 |
| kotlinx-coroutines (`core`, `swing`, `android`, `test`) | 1.10.2 |
| jmdns | 3.5.9 |
| JUnit | 4.13.2 |
| AndroidX libraries (spec 4 §3) | newest stable on the day of DSK-01 |

If this set fails to build `:desktop` or (later) `:android`, raise Kotlin, Compose Multiplatform,
AGP and Gradle together to the lowest mutually compatible newer versions and record the reason as
a comment in the catalog. JDK 21 is the toolchain; `:proto` and `:core` compile to JVM 17
bytecode so Android can use them. Compiler warnings are errors (`allWarningsAsErrors`).

The UI is in English only.

## 4. `:proto`

Generates Java lite messages, Kotlin lite extensions, grpc-java lite stubs and grpc-kotlin stubs
from `v3/proto/trd/v1/trd.proto` (the `proto` source set points at `../../../proto`), using the
`protoc` artifact (no system protoc). Package `com.timelapserender.trd.v1`.

## 5. `:core`

All behaviour that does not draw pixels, shared by desktop and Android.

### 5.1 Connection

- `ServerEndpoint(host, port, tls: TlsMode)` where `TlsMode` is `Plaintext` (loopback) or
  `Pinned(fingerprintSha256Hex)`.
- `ChannelFactory` builds a grpc-okhttp `ManagedChannel` for both platforms: plaintext for
  loopback; for LAN an `SSLContext` whose trust manager accepts exactly the leaf certificate whose
  SHA-256 over its DER equals the pin (anything else is rejected), with a hostname verifier that
  accepts any name (the pin is the identity). Keep-alive 30 s.
- `TokenInterceptor` adds `authorization: Bearer <token>` when the server has one.
- `TrdClient(endpoint, token)`: `serverInfo()`, `watchSnapshots(rate)`, every unary RPC of
  spec 1 §7 as a `suspend` function, `watchLog(afterSeq)`. On connect it calls `GetServerInfo`;
  `api_version != 1` → state `Incompatible(serverVersion)` with the message "This trd uses API
  version N; this app supports version 1."
- `ConnectionState`: `Connecting`, `Connected(info)`, `Reconnecting(attempt, lastUpdateMs)`,
  `NeedsPairing` (`UNAUTHENTICATED`: the token is missing, wrong or revoked), `Incompatible`,
  `Offline(lastUpdateMs)` (after the fourth failed attempt; keeps retrying). Back-off between
  attempts: 1, 2, 5, then 10 s.

### 5.2 Servers

- `ServerRecord(serverId, name, host, port, fingerprint, token, platform, lastSeenMs)` and
  `ServerStore` (interface: `list`, `save`, `remove`). `JsonFileServerStore` writes
  `servers.json` with owner-only permissions under the client's data directory (macOS
  `~/Library/Application Support/TimelapseRenderMonitor/`, Linux `$XDG_CONFIG_HOME/trm/`, Windows
  `%APPDATA%\TimelapseRenderMonitor\`); Android supplies its own (spec 4).
- `ServerRegistry` merges the local server (desktop only: `127.0.0.1:50071` or the `--server`
  override), stored records and discovered services, and holds the active server id.

### 5.3 Discovery and pairing

- `ServerDiscovery` interface with `servers: StateFlow<List<DiscoveredServer>>`, `start()`,
  `stop()`; `DiscoveredServer(serverId, name, host, port, fingerprint, version, apiVersion)` from
  the spec 1 §8.4 TXT records (`id`, `name`, `fp`, `port`, `ver`, `api`).
- `JmdnsDiscovery` (used by desktop) browses `_trd._tcp.local.` on every non-loopback interface.
- `PairingClient.pair(endpoint, code, deviceName, platform)`: connects with the pin and no token,
  calls `Pair`, then `GetServerInfo` with the new token to confirm `caller_is_paired`, and returns
  the `ServerRecord` to store. Wrong code → "That code is not valid. Ask for a new one on the trd
  machine."; `RESOURCE_EXHAUSTED` → "Too many wrong codes. Issue a new code on the trd machine."

### 5.4 State and presenters

- `SnapshotRepository(client)`: a `StateFlow<RepoState>` of the latest `Snapshot`, the
  `ConnectionState` and `lastUpdateMs`.
- `ActivityPresenter(repo, filter)` → `StateFlow<ActivityUi>`: header (server name, connection
  text, phase chip text and colour role, counts line, global pause, sandbox flag), problem rows,
  failure rows, sequence cards with variant rows, recently-rendered rows (from the snapshot, then
  older pages from `ListRenderedVideos` on demand), each with its overview §6.2 key and every
  display string preformatted (§5.5). Filter: search text (sequence name and output file name,
  case-insensitive) and an optional export path.
- `ScrollKeeper` (pure): given the previous keys, the new keys, the first visible key and whether
  the list is at the top, returns `StayAtTop`, or `KeepAnchor(key)` with the number of rows
  inserted above it since the user left the top. It implements overview §6.2 S1–S7 and is used by
  both UIs.
- `Commands(client)`: one `suspend` function per control of overview §6.9, returning
  `Result<Unit>` whose failure message is the gRPC status description.
- `ThumbnailCache`: JPEG bytes by rendered-video id; memory LRU of 200 entries plus a disk cache of
  at most 200 MB per server under the client's cache directory; fetches with `GetThumbnail`; a
  `NOT_FOUND` is remembered for 10 minutes.
- `DetailsModel`: loads `GetRenderDetails` for a render, rendered video or failure.
- `LogModel`: `ListLog` for the first page, `WatchLog(after_seq)` live, older pages on demand,
  level filter and search; newest first.
- `ConfigEditorModel`: loads `GetConfig`; holds an editable copy of `Config` and the codec list;
  calls `ValidateConfig` 400 ms after the last edit; exposes issues by JSON pointer; `save()` sends
  `UpdateConfig` (and `UpdateCodecs` when codecs changed) with the loaded revisions; on
  `FAILED_PRECONDITION` sets `conflict = true` so the UI can offer a reload; tracks dirty state.
- `DirectoryBrowserModel`: `ListDirectory` navigation with roots, parent, and a selection.
- `DevicesModel` (loopback only): `StartPairing` with a countdown, `ListPairedDevices`,
  `RevokeDevice`.
- `EventWatcher(client, lastSeqStore)`: follows `WatchLog` and emits `SEQUENCE_FINISHED` and
  `RENDER_FAILED` events for notifications; it stores the last seen `seq` per server so nothing is
  notified twice, and on the first connection to a server starts from the newest `seq` (no flood of
  old events).

### 5.5 Formatting (identical on every Kotlin client; spec 3 copies the rules)

| Value | Rule | Examples |
|---|---|---|
| bytes | binary units with labels B, KB, MB, GB, TB; one decimal below 100, none from 100 | `512 B`, `12.4 GB`, `231 MB` |
| frame counts | thousands separator `,` | `1,234 / 1,500 frames` |
| duration of a video | below 60 s one decimal `50.0 s`; else `m:ss`; from an hour `h:mm:ss` | `50.0 s`, `3:20`, `1:02:05` |
| ETA | `~<n> s` below a minute, `~<n> min` below an hour, `~<h> h <m> min` | `~40 s`, `~6 min`, `~1 h 12 min` |
| export rate | `<n> frames/min` | `41 frames/min` |
| relative time | `just now` (< 60 s), `<n> min ago`, `<n> h ago` (< 24 h), `yesterday`, else the local date `Nov 20` (`Nov 20, 2025` in another year) | |
| absolute time | local `HH:mm` today, else `MMM d HH:mm` | `14:02`, `Nov 20 14:02` |
| variant label | `<codec> · <resolution_label> · <frame_rate> fps` | `ProResMQ · 2160p · 30 fps` |

### 5.6 Test fixtures

`core/src/testFixtures`: `FakeTrdService` (a grpc-java in-process implementation of every RPC,
driven by scripted snapshots, log entries and configs, recording every call) and snapshot
builders. Desktop and Android tests use them; no test talks to a real `trd`.

## 6. The desktop app

### 6.1 Start and connection

- Command-line options: `--server host:port` (plaintext loopback override, used by `make develop`
  with `127.0.0.1:50081`).
- On start: if a local server answers on `127.0.0.1:50071` (or `--server`), connect to it.
  Otherwise show the **trd is not running** screen: "trd is not running on this computer.",
  buttons **Start trd** (runs the spec 1 §10.4 restart command for the OS:
  `launchctl kickstart -k gui/<uid>/com.timelapserender.trd`, `systemctl --user start trd`,
  `schtasks /Run /TN TimelapseRenderDaemon`; on failure shows the command's output and the
  release page link), **Get trd** (opens
  `https://github.com/brianm998/timelapse_render_daemon/releases`) and **Connect to another
  computer…** (the server list). It keeps probing every 2 s.
- The last active server is remembered and reconnected on start.

### 6.2 Window

- Title: `Timelapse Render Monitor — <server name>`; minimum size 900 × 600; size and position
  remembered.
- Top bar: server switcher (local server first, then paired servers, then a "Discovered" group with
  **Pair…**), connection text, phase chip, `SANDBOX` badge when `ServerInfo.sandbox`, counts line,
  the "Pause all rendering" switch, the search field (Cmd/Ctrl+F), the export-path filter.
- Body: a split pane. Left: the Activity list (overview §6.1). Right: the Details pane (overview
  §6.3) for the selected row, collapsible (remembered); with nothing selected it shows the last
  scan time and scan duration.
- Menus (the Compose `MenuBar`, the macOS menu bar on macOS): **File** (Connect to another
  computer…, Pair a phone… [loopback only], Close window), **View** (Config… Cmd/Ctrl+E, Log…
  Cmd/Ctrl+L, Details pane Cmd/Ctrl+D, Preferences… Cmd/Ctrl+,), **trd** (Pause all rendering /
  Resume rendering, Rescan now Cmd/Ctrl+R, Start trd, Show trd's log folder [loopback only]),
  **Help** (About, Releases page).
- Config and Log open in their own windows, one per server.

### 6.3 Activity list

- A `LazyColumn` with the overview §6.1 sections and §6.2 rules: keys from the presenter; before
  each update it records whether the list is at the top (first visible index 0 and offset 0);
  after the update it calls `requestScrollToItem(0)` when it was, otherwise lets `LazyColumn` keep
  the anchor and shows the `↑ N new` pill from `ScrollKeeper`.
- Rows follow overview §6.1 exactly; posters come from `ThumbnailCache` (Skia decode), 16:9 box,
  letterboxed, 96 px wide; a grey placeholder while loading or missing.
- Selection: single click selects (Details pane); double click on a rendered video opens it
  (loopback and `file_exists` only).

### 6.4 Controls and Details

- Each sequence card has a menu: Pause / Resume sequence, Details. Each variant row has icon
  buttons by status: Suspend (RUNNING), Resume (SUSPENDED), Stop (QUEUED, RUNNING, SUSPENDED),
  Pause / Resume variant (otherwise), Retry (FAILED or PAUSED). Failure cards: Retry, Dismiss,
  Details. Rendered rows: Open and Reveal (loopback and `file_exists` only), Copy path, Details.
- Stop asks for confirmation ("Stop this render? ffmpeg is stopped, the partial file deleted and
  the variant paused.") with a "Don't ask again" checkbox stored in preferences; Pause all
  rendering asks the first time only, with the same checkbox.
- Open / Reveal: macOS `open <path>` / `open -R <path>`; Linux `xdg-open <path>` /
  `xdg-open <directory>`; Windows `explorer.exe <path>` / `explorer.exe /select,<path>`.
- A failed command shows a snackbar with the message.
- Details pane content: overview §6.3, with Copy buttons for the command line and the report.

### 6.5 Config window

Overview §6.4 in a window with a sidebar of the groups and a form for the selected group. Path
lists open a folder browser dialog (`DirectoryBrowserModel`) starting at the current value or the
roots. The rendering table edits variants inline. Save (Cmd/Ctrl+S) is disabled while errors exist;
issues show under their fields and as a count in the sidebar. Closing with unsaved changes asks to
save, discard or cancel. A conflict shows "The config changed on the server" with **Reload**.

### 6.6 Log window

Overview §6.5: a `LazyColumn` (newest first, the same scroll rules), a level picker (Debug, Info,
Warning, Error; default Info), search, Copy selected lines.

### 6.7 Devices and first-run setup (loopback only)

- **Pair a phone…**: the overview §6.7 dialog: the code in 48 pt digits grouped `123 456`, the
  countdown, server name, LAN addresses and port, the first 16 hex digits of the fingerprint
  (grouped in fours), **New code**; below, the paired devices with **Revoke** (confirmation).
- **First-run setup** appears when connected over loopback and the phase is `PHASE_NO_CONFIG`:
  step 1 "Choose a config": **Use an existing config.json…** (file chooser; calls `SetConfigPath`)
  or **Create a new config** (`SetConfigPath` with `create_if_missing` at `trd`'s default path),
  then opens the Config window; step 2 (macOS, when `PROBLEM_FULL_DISK_ACCESS_MISSING` is present)
  "Allow Full Disk Access": text from spec 1 §6.12, **Open Privacy settings** (opens
  `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`) and **Show trd in
  Finder** (`open -R "/Applications/Timelapse Render Daemon.app"`); the step completes by itself
  when the problem disappears.
- The `PROBLEM_FULL_DISK_ACCESS_MISSING` banner's **How to fix** opens step 2.

### 6.8 Tray, notifications and preferences

- Tray icon (Compose `Tray`): the monitor icon with a status dot — grey idle, green rendering, red
  when failures exist; tooltip = the counts line; menu: Open window, Pause all rendering / Resume
  rendering, Quit.
- Closing the window keeps the app in the tray when "Keep running in the tray" is on (default on);
  Quit ends it.
- Notifications through `TrayState.sendNotification`: overview §6.8 texts, from `EventWatcher`
  for every paired or local server that the app is connected to.
- Preferences window: notifications for finished sequences (on), for failed renders (on), keep
  running in the tray (on), snapshot rate (1–10 per second, default 4), and the "Don't ask again"
  choices (reset button). Stored with `java.util.prefs.Preferences` under
  `com/timelapserender/monitor`.

## 7. Makefile (`v3/clients/kotlin/Makefile`, component `desktop`)

| Target | Command |
|---|---|
| `run` | `./gradlew :desktop:run` (connects to the installed `trd` on 50071) |
| `develop` | starts `make -C ../../trd develop-daemon` in the background, waits for its `TRD_LISTENING` line, runs `./gradlew :desktop:run --args="--server 127.0.0.1:50081"`, stops the daemon on exit |
| `release` | `./gradlew :desktop:packageReleaseDistributionForCurrentOS`, signed and notarized on macOS when `TRM_SIGN_IDENTITY`, `APPLE_API_KEY_PATH`, `APPLE_API_KEY_ID` and `APPLE_API_ISSUER_ID` are set; `TRM_JPACKAGE_JDK` points packaging at a JDK that has `jpackage` |
| `clean` | `./gradlew clean` |
| `test` | `./gradlew :proto:build :core:test :desktop:test` |
| `check` | `./gradlew :proto:compileKotlin :core:compileKotlin :desktop:compileKotlin` (warnings are errors) |
| `ci` | `bash scripts/ci-desktop.sh` |

`scripts/ci-desktop.sh`: `./gradlew --no-daemon --console=plain :proto:build :core:test :desktop:test`
with `-Djava.awt.headless=true` for tests. `scripts/mason-desktop.sh`: the same with `JAVA_HOME`
from the environment, then merges every `*/build/test-results/test/TEST-*.xml` into
`build/mason-desktop-results.xml` (one `<testsuites>` root), keeping Gradle's exit code.

## 8. GitHub Actions

- `v3-gate.yml` job `kotlin` (defined by TRD-00) starts running `scripts/ci-desktop.sh` as soon
  as DSK-01 lands; nothing here edits it.
- `v3-desktop-build.yml` (DSK-12), `workflow_call` with inputs `version` and `is_release`,
  secrets inherited. Matrix:

| Target | Runner | Gradle task | Asset |
|---|---|---|---|
| `macos-arm64` | `macos-15` | `:desktop:packageReleaseDmg` | `TimelapseRenderMonitor-<v>-macos-arm64.dmg` |
| `macos-x64` | `macos-15-intel` | `:desktop:packageReleaseDmg` | `TimelapseRenderMonitor-<v>-macos-x64.dmg` |
| `linux-x64` | `ubuntu-24.04` | `:desktop:packageReleaseDeb` | `timelapse-render-monitor_<v>_amd64.deb` |
| `windows-x64` | `windows-2022` | `:desktop:packageReleaseMsi` | `TimelapseRenderMonitor-<v>-windows-x64.msi` |

  Steps: checkout; `actions/setup-java@v5` (temurin 21, which includes `jpackage`);
  `gradle/actions/setup-gradle@v5`; Linux: `sudo apt-get install -y fakeroot binutils`; Windows:
  if `candle.exe` is missing, `choco install wixtoolset --version 3.14.1 -y` and add its `bin` to
  `PATH`; macOS: import the Developer ID certificate into a temporary keychain and set
  `TRM_SIGN_IDENTITY` from `security find-identity` (the "Developer ID Application: … (<team>)"
  line, as ReelVault does), stage the API key; run the Gradle task with
  `-Ptrm.version=<v>`; macOS with a certificate: `xcrun notarytool submit <dmg> --key … --key-id …
  --issuer … --wait` (print `notarytool log` and fail unless `Accepted`), then `xcrun stapler staple`
  with up to 8 attempts 15 s apart, then `xcrun stapler validate`; macOS release without a
  certificate fails ("macOS release builds must be signed"); rename to the asset name; upload
  artifact `release-asset-desktop-<target>`; always delete the keychain and key.
- `v3-release.yml` gains job `desktop` (`needs: [version]`, `uses: ./.github/workflows/v3-desktop-build.yml`,
  `with: version, is_release`, `secrets: inherit`), and `desktop` is appended to `publish.needs`.
- `macos-15-intel` is GitHub's last Intel macOS image (announced for retirement in 2027); when it
  is retired, the x86_64 `.dmg` stops being built and that job is removed.

## 9. Packaging details

- `desktop/build.gradle.kts` `compose.desktop.application`: `mainClass =
  "com.timelapserender.monitor.desktop.MainKt"`; `nativeDistributions`: `targetFormats(Dmg, Deb,
  Msi)`, `packageName = "Timelapse Render Monitor"`, `packageVersion` = the version without a
  pre-release suffix (from `-Ptrm.version`, else `v3/VERSION`), `vendor = "Timelapse Render"`,
  `description = "Watches and controls the Timelapse Render Daemon"`; macOS `bundleID =
  "com.timelapserender.monitor"`, icon `v3/assets/icon/monitor.icns`; Windows icon
  `v3/assets/icon/monitor.ico`, `menu = true`, `shortcut = true`, `upgradeUuid` a fixed UUID chosen
  in DSK-12 and never changed; Linux icon `v3/assets/icon/monitor-512.png`,
  `packageName = "timelapse-render-monitor"`.
- macOS signing when `TRM_SIGN_IDENTITY` is set: `signing { sign.set(true); identity.set(…) }`,
  `entitlementsFile` and `runtimeEntitlementsFile` = `desktop/packaging/macos-entitlements.plist`
  with `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`
  and `com.apple.security.cs.disable-library-validation` set to true.
- `javaHome` for packaging = `TRM_JPACKAGE_JDK` when set (the Android Studio JBR used for builds
  has no `jpackage`).

## 10. Tickets

Each ticket's last line names the IDs it depends on; when filing, replace it with
`Depends on #<n>, #<m>` (overview §12).

### DSK-01 — Kotlin build skeleton for desktop and shared modules

## Goal
`v3/clients/kotlin` is a Gradle build with `:proto`, `:core` and `:desktop` that compiles and
tests on macOS, Linux and Windows, with every version and plugin for desktop and Android declared,
so later tickets (including Android's) never touch the build skeleton.

## Context
§3 (layout and versions), §7 (Makefile and scripts), overview §9. The `v3-gate.yml` `kotlin` job
and the `kotlin-test` mason suite start running once `settings.gradle.kts` and
`scripts/mason-desktop.sh` exist.

## Files
- `v3/2-trm-desktop.md` (this spec; §3, §7)
- `v3/clients/kotlin/settings.gradle.kts` (new)
- `v3/clients/kotlin/build.gradle.kts` (new)
- `v3/clients/kotlin/gradle/libs.versions.toml` (new)
- `v3/clients/kotlin/gradle.properties` (new)
- `v3/clients/kotlin/gradlew` (new)
- `v3/clients/kotlin/Makefile` (new)
- `v3/clients/kotlin/scripts/ci-desktop.sh` (new)
- `v3/clients/kotlin/scripts/mason-desktop.sh` (new)
- `v3/clients/kotlin/proto/build.gradle.kts` (new)
- `v3/clients/kotlin/core/build.gradle.kts` (new)
- `v3/clients/kotlin/desktop/build.gradle.kts` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/Main.kt` (new)

## Approach
1. Gradle wrapper 8.11.1 (`gradle wrapper`), the §3 catalog including the AndroidX entries named in
   spec 4 §3, root `plugins {}` with every plugin `apply false`, `settings.gradle.kts` with the
   conditional `:android` include.
2. `:proto` per §4 (it generates and compiles here); `:core` with `java-test-fixtures`, coroutines,
   grpc-okhttp and jmdns; `:desktop` as a Compose Desktop application whose `Main.kt` opens a window
   titled "Timelapse Render Monitor" saying "Not connected".
3. Makefile and both scripts per §7.

## Acceptance criteria
- A1: `make -C v3/clients/kotlin test` succeeds and the generated `TrdServiceGrpcKt` class exists — check: the command and a test in `:core` that references `TrdServiceGrpcKt.TrdServiceCoroutineStub`.
- A2: the `kotlin` gate job is green on all three runners — check: `v3-gate.yml`.
- A3: `bash v3/scripts/suite.sh kotlin-test` writes `build/mason-desktop-results.xml` with at least one passing case — check: the command.

## Tests
One `:core` test referencing the generated stub; one `:desktop` test that composes the start screen offscreen.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Any real behaviour.

Depends on: TRD-GATE

---

### DSK-02 — Shared connection layer: channels, pinned TLS, tokens, servers

## Goal
`:core` connects to `trd` over loopback or pinned TLS with a token, reports connection state, and
stores servers, as §5.1–§5.2 say.

## Context
§5.1, §5.2; spec 1 §8.2 (auth); ReelVault `ChannelFactory.kt`, `PinnedTls.kt`, `TokenStorage.kt`.

## Files
- `v3/2-trm-desktop.md` (this spec; §5.1, §5.2)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/net/ChannelFactory.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/net/PinnedTls.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/net/TrdClient.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/net/ConnectionState.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/servers/ServerStore.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/servers/ServerRegistry.kt` (new)
- `v3/clients/kotlin/core/src/testFixtures/kotlin/com/timelapserender/monitor/core/fake/FakeTrdService.kt` (new)

## Approach
1. The pieces of §5.1 and §5.2; `JsonFileServerStore` with owner-only permissions (POSIX
   permissions where supported, ACL-free default elsewhere).
2. `FakeTrdService` (§5.6) with in-process and TLS (self-signed test certificate generated in the
   test) variants.

## Acceptance criteria
- A1: a pinned channel to a fake TLS server with the right fingerprint connects; a wrong fingerprint fails before any RPC — check: `:core:test` `PinnedTlsTest`.
- A2: `UNAUTHENTICATED` from the server yields `NeedsPairing`; API version 2 yields `Incompatible` — check: `TrdClientTest`.
- A3: the back-off sequence is 1, 2, 5, 10 s (injected scheduler) — check: `TrdClientTest`.
- A4: `servers.json` round-trips and is owner-only on POSIX — check: `ServerStoreTest`.

## Tests
`PinnedTlsTest`, `TrdClientTest`, `ServerStoreTest`.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Presenters (DSK-03).

Depends on: TRD-GATE, DSK-01

---

### DSK-03 — Shared state: repository, presenters, formatting, models

## Goal
`:core` turns snapshots, logs and configs into ready-to-draw UI state for any Kotlin client, with
the §5.5 formatting and the §6.2 scroll rules.

## Context
§5.4, §5.5; overview §6.1–§6.5, §6.9.

## Files
- `v3/2-trm-desktop.md` (this spec; §5.4, §5.5)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/SnapshotRepository.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/ActivityPresenter.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/ScrollKeeper.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/Commands.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/ThumbnailCache.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/DetailsModel.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/LogModel.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/ConfigEditorModel.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/DirectoryBrowserModel.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/DevicesModel.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/state/EventWatcher.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/format/Format.kt` (new)

## Approach
Implement every class of §5.4 over `TrdClient`, with an injectable clock and dispatcher, and the
§5.5 table in `Format.kt`.

## Acceptance criteria
- A1: `ScrollKeeper` implements S1–S7 — check: `ScrollKeeperTest` (a case per rule).
- A2: the presenter's sections, order, keys and strings for a scripted snapshot equal a golden list — check: `ActivityPresenterTest`.
- A3: every row of the §5.5 table formats as shown — check: `FormatTest`.
- A4: the config editor validates after 400 ms of quiet, saves with the revision, and reports a conflict — check: `ConfigEditorModelTest` (virtual time).
- A5: `EventWatcher` does not repeat events across reconnects and starts at the newest seq on first contact — check: `EventWatcherTest`.

## Tests
One test class per model, all against `FakeTrdService`.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Composables.

Depends on: TRD-GATE, DSK-02

---

### DSK-04 — Shared discovery and pairing

## Goal
`:core` finds `trd`s on the LAN and pairs with one, as §5.3 says.

## Context
§5.3; spec 1 §8.3, §8.4; ReelVault `ServerDiscovery.kt`, `PairingClient.kt`.

## Files
- `v3/2-trm-desktop.md` (this spec; §5.3)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/discovery/ServerDiscovery.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/discovery/JmdnsDiscovery.kt` (new)
- `v3/clients/kotlin/core/src/main/kotlin/com/timelapserender/monitor/core/pairing/PairingClient.kt` (new)

## Approach
1. The interface, a pure `parseTxt(map) -> DiscoveredServer?` (rejects missing `id`, `fp` or
   `port`, and `api` other than `1`), and `JmdnsDiscovery`.
2. `PairingClient` with the messages of §5.3.

## Acceptance criteria
- A1: TXT parsing accepts a valid record and rejects each missing field — check: `DiscoveryTest`.
- A2: pairing against `FakeTrdService` over TLS stores a working record; a wrong code and an exhausted code give the §5.3 messages — check: `PairingClientTest`.
- A3: no test opens a multicast socket — check: review (jmdns is not started in tests).

## Tests
`DiscoveryTest`, `PairingClientTest`.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
The pairing UI.

Depends on: TRD-GATE, DSK-02

---

### DSK-05 — Desktop app shell: windows, menus, connection and server switching

## Goal
The desktop app starts, finds the local `trd` or explains how to start it, connects to local or
paired servers, and has the §6.2 window, top bar and menus.

## Context
§6.1, §6.2; spec 1 §10.4 (start commands).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.1, §6.2)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/Main.kt`
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/AppState.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/MainWindow.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/TopBar.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/NotRunningScreen.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/platform/TrdService.kt` (new)

## Approach
1. `AppState` wires `ServerRegistry`, `JmdnsDiscovery`, the active `TrdClient` and the presenters;
   remembers the last server and window geometry.
2. `TrdService` runs the start command through an injectable runner.
3. The window, top bar (with the server switcher and Pair… entries), menus and the not-running
   screen; the Activity area is a placeholder until DSK-06.

## Acceptance criteria
- A1: with no server, the not-running screen shows and Start trd calls the OS command — check: `NotRunningScreenTest` (recording runner, offscreen compose test).
- A2: with a fake local server the top bar shows its name, phase and counts — check: `TopBarTest`.
- A3: switching servers reconnects and resets the list to the top — check: `AppStateTest`.

## Tests
Offscreen Compose tests (`createComposeRule`, headless) and `AppStateTest`.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Activity rows (DSK-06).

Depends on: TRD-GATE, DSK-03, DSK-04

---

### DSK-06 — Desktop Activity view

## Goal
The Activity list shows problems, failures, in-progress sequences and recently rendered videos as
overview §6.1 describes, and never scrolls away from new entries.

## Context
§6.3; overview §6.1, §6.2; `ActivityPresenter`, `ScrollKeeper`, `ThumbnailCache` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.3)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/ActivityList.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/rows/ProblemBanner.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/rows/FailureCard.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/rows/SequenceCard.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/rows/RenderedRow.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/NewItemsPill.kt` (new)

## Approach
Composables for every row type and the list with the §6.3 scroll handling, the search field and
export-path filter from the top bar, and "Show older" paging.

## Acceptance criteria
- A1: a scripted snapshot renders every section in order with the expected texts — check: `ActivityListTest` (semantics assertions).
- A2: at the top, a new in-progress card is visible after the update; scrolled down, the anchor row stays and the pill shows the count — check: `ActivityScrollTest`.
- A3: a finished sequence's card disappears and its videos head Recently rendered in the same frame — check: `ActivityListTest`.

## Tests
Offscreen Compose tests; PNG renders of each state written to `desktop/build/screenshots/` for review (not compared).

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Row actions (DSK-07).

Depends on: TRD-GATE, DSK-05

---

### DSK-07 — Desktop controls and Details pane

## Goal
Every control of overview §6.9 is reachable from the desktop rows, and the Details pane shows
everything overview §6.3 lists.

## Context
§6.4; overview §6.3, §6.9; `Commands`, `DetailsModel` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.4)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/DetailsPane.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/RowActions.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/ConfirmDialog.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/platform/FileOpener.kt` (new)

## Approach
Menus and icon buttons per status, confirmations with "Don't ask again", snackbars for errors,
`FileOpener` with an injectable runner, the Details pane per item type.

## Acceptance criteria
- A1: each status shows exactly the §6.4 buttons and each sends the right RPC — check: `RowActionsTest` (FakeTrdService records calls).
- A2: Open and Reveal appear only for loopback servers and existing files and run the OS command — check: `FileOpenerTest`.
- A3: the Details pane shows the command line and log tail from `GetRenderDetails` — check: `DetailsPaneTest`.

## Tests
Offscreen Compose tests with a recording runner.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Config editing (DSK-08).

Depends on: TRD-GATE, DSK-06

---

### DSK-08 — Desktop Config window

## Goal
The whole config and codec table can be edited from the desktop with validation and a server-side
folder browser, as overview §6.4 says.

## Context
§6.5; overview §6.4; spec 1 §6.1–§6.2 (validation rules); `ConfigEditorModel`,
`DirectoryBrowserModel` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.5)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/config/ConfigWindow.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/config/PathListEditor.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/config/RenderingTable.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/config/CodecEditor.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/config/FolderBrowserDialog.kt` (new)

## Approach
One composable per overview §6.4 group, the sidebar with issue counts, Save, the unsaved-changes
prompt and the conflict reload.

## Acceptance criteria
- A1: editing a regex to an invalid one shows the server's issue under the field and disables Save — check: `ConfigWindowTest`.
- A2: adding a path through the folder browser and saving sends `UpdateConfig` with the loaded revision — check: `ConfigWindowTest`.
- A3: removing a codec used by a variant is refused with the server's message — check: `CodecEditorTest`.

## Tests
Offscreen Compose tests against `FakeTrdService`.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
First-run setup (DSK-10).

Depends on: TRD-GATE, DSK-05

---

### DSK-09 — Desktop Log window

## Goal
The daemon's log is readable live from the desktop, newest first, filtered and searchable.

## Context
§6.6; overview §6.5; `LogModel` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.6)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/LogWindow.kt` (new)

## Approach
The window over `LogModel` with the scroll rules, the level picker, search and copying.

## Acceptance criteria
- A1: live entries appear at the top while at the top, and the pill counts them when scrolled — check: `LogWindowTest`.
- A2: the level picker hides Info lines when set to Warning — check: `LogWindowTest`.

## Tests
Offscreen Compose tests.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Notifications (DSK-11).

Depends on: TRD-GATE, DSK-05

---

### DSK-10 — Desktop Devices dialog and first-run setup

## Goal
From the desktop on `trd`'s machine the owner can pair phones, revoke devices, choose or create a
config, and grant Full Disk Access with guidance.

## Context
§6.7; overview §6.6, §6.7; spec 1 §6.12, §8.3; `DevicesModel` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.7)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/DevicesDialog.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/FirstRunSetup.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/PairServerDialog.kt` (new)

## Approach
1. The Devices dialog and its countdown.
2. `PairServerDialog`: pairing this desktop with a discovered or hand-entered LAN server
   (platform `DEVICE_PLATFORM_DESKTOP`, device name = host name + " (desktop)").
3. First-run setup with its two steps; the Full Disk Access step's buttons use the injectable
   runner.

## Acceptance criteria
- A1: Pair a phone shows a 6-digit code and counts down; New code replaces it — check: `DevicesDialogTest` (virtual time).
- A2: the dialog and menu items are absent when connected over LAN — check: `DevicesDialogTest`.
- A3: with `PHASE_NO_CONFIG`, choosing a file calls `SetConfigPath` with it — check: `FirstRunSetupTest`.
- A4: the Full Disk Access step completes when the problem disappears from the snapshot — check: `FirstRunSetupTest`.

## Tests
Offscreen Compose tests.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Phone-side pairing (specs 3 and 4).

Depends on: TRD-GATE, DSK-05, DSK-08

---

### DSK-11 — Desktop tray, notifications and preferences

## Goal
The desktop app lives in the tray, notifies when sequences finish and renders fail, and has
preferences.

## Context
§6.8; overview §6.8; `EventWatcher` (DSK-03).

## Files
- `v3/2-trm-desktop.md` (this spec; §6.8)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/TrayIcon.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/ui/PreferencesWindow.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/Preferences.kt` (new)
- `v3/clients/kotlin/desktop/src/main/kotlin/com/timelapserender/monitor/desktop/Notifier.kt` (new)

## Approach
1. `Tray` with the three status dots drawn at runtime over the monitor icon, the tooltip and menu.
2. `Notifier` maps `EventWatcher` events to `TrayState.sendNotification` per preference.
3. `Preferences` over `java.util.prefs` with an in-memory implementation for tests.

## Acceptance criteria
- A1: a scripted `EVENT_SEQUENCE_FINISHED` produces one notification with the §6.8 text, and none after a reconnect — check: `NotifierTest` (fake tray state).
- A2: turning off failure notifications suppresses them — check: `NotifierTest`.
- A3: closing the window with "keep running in the tray" on hides it instead of quitting — check: `AppStateTest`.

## Tests
Unit tests with a fake tray; no real system tray in tests.

## CI
`v3-gate.yml` `kotlin`.

## Out of scope
Packaging (DSK-12).

Depends on: TRD-GATE, DSK-05

---

### DSK-12 — Desktop packaging, signing, release workflow and acceptance

## Goal
Tags produce a signed, notarized macOS `.dmg` for each architecture, a Linux `.deb` and a Windows
`.msi`, attached to the draft release; the desktop suite floors are raised.

## Context
§8, §9; spec 1 §9.5 (release orchestrator); star `star-desktop/build.gradle.kts` and
`kotlin-client.yml` `macos-signed` (§2).

## Files
- `v3/2-trm-desktop.md` (this spec; §8, §9)
- `v3/clients/kotlin/desktop/build.gradle.kts`
- `v3/clients/kotlin/desktop/packaging/macos-entitlements.plist` (new)
- `.github/workflows/v3-desktop-build.yml` (new)
- `.github/workflows/v3-release.yml`
- `.mason/mason.toml`

## Approach
1. `nativeDistributions` and signing per §9; `-Ptrm.version`.
2. `v3-desktop-build.yml` per §8; add the `desktop` job and `publish.needs` entry to
   `v3-release.yml`.
3. Run a `workflow_dispatch` of `v3-release.yml`; fix until green.
4. Raise `kotlin-test`'s `min_tests` in `.mason/mason.toml` to 90 % of the measured count.
5. Acceptance by the owner on the Mac: install the `.dmg` from the dispatch artifacts, run it
   against the installed `trd` and against `make develop`; record the result in the PR
   description.

## Acceptance criteria
- A1: the dispatch run produces the four §8 assets as `release-asset-desktop-*` artifacts — check: the run.
- A2: with signing secrets, both `.dmg`s are notarized and stapled (`xcrun stapler validate` passes) — check: the run log.
- A3: the `.msi` installs silently on the Windows runner and its launcher exists — check: a CI step `msiexec /i … /qn` followed by a file existence check.
- A4: `kotlin-test`'s floor is raised — check: `.mason/mason.toml`.

## Tests
The CI steps above.

## CI
`v3-desktop-build.yml` (new), `v3-release.yml`.

## Out of scope
Android packaging (spec 4).

Depends on: TRD-GATE, DSK-06, DSK-07, DSK-08, DSK-09, DSK-10, DSK-11

## 11. Dependency summary

| Ticket | Depends on |
|---|---|
| DSK-01 | TRD-GATE |
| DSK-02 | TRD-GATE, DSK-01 |
| DSK-03 | TRD-GATE, DSK-02 |
| DSK-04 | TRD-GATE, DSK-02 |
| DSK-05 | TRD-GATE, DSK-03, DSK-04 |
| DSK-06 | TRD-GATE, DSK-05 |
| DSK-07 | TRD-GATE, DSK-06 |
| DSK-08 | TRD-GATE, DSK-05 |
| DSK-09 | TRD-GATE, DSK-05 |
| DSK-10 | TRD-GATE, DSK-05, DSK-08 |
| DSK-11 | TRD-GATE, DSK-05 |
| DSK-12 | TRD-GATE, DSK-06, DSK-07, DSK-08, DSK-09, DSK-10, DSK-11 |

Android (spec 4) depends on DSK-01, DSK-02, DSK-03 and DSK-04.

## 12. Definition of done

- Every ticket's acceptance criteria hold.
- `v3-gate.yml` `kotlin` is green on all three runners; a `v3-release.yml` dispatch produces the
  desktop assets.
- On the owner's Mac, the app installed from the `.dmg` shows the installed `trd`'s activity, can
  pair a phone, and edits the config.
