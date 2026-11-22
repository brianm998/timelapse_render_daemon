<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Spec 4 — Timelapse Render Monitor for Android (Kotlin Compose)

Read `v3/0-overview.md` first (especially §6), `v3/1-trd-core-daemon.md` §7–§8, and
`v3/2-trm-desktop.md` §3 and §5: the Android app lives in the same Gradle build as the desktop app
and reuses its shared modules `:proto` and `:core`. Every ticket here depends on `TRD-GATE` and on
the spec 2 tickets that build what it reuses (named per ticket).

## 1. Outcome

- An Android phone and tablet app, **Timelapse Render Monitor** (`com.timelapserender.monitor`,
  Android 8.0 / API 26 or newer, target and compile SDK 36), that discovers `trd` with NSD, pairs
  with a code issued on `trd`'s machine, and implements overview §6 with full parity (D7):
  Activity, Details, every control, the Config editor with a server folder browser, the Log, and
  Servers. Devices (pairing-code issue and revocation) are not on Android (overview §6.7).
- Version tags upload a signed bundle to the Google Play internal testing track and attach a signed
  APK to the draft GitHub Release.

## 2. What to read

- Overview §2, §4, §6, §9, §10; spec 1 §7, §8, §12; spec 2 §3 (build and versions), §5 (`:core`:
  connection, servers, discovery interface, pairing, presenters, models, formatting, fakes), §6.4
  (the buttons per status and the Stop confirmation text).
- ReelVault at `228e3c8b7937291b1d53314eb3df92132f6757d6` (URL form in spec 1 §2):
  `android/build.gradle.kts`, `android/src/main/kotlin/com/reelvault/android/data/AndroidServerDiscovery.kt`,
  `AndroidTokenStorage.kt`, `OkHttpChannelFactory.kt`, `.github/workflows/android-release.yml`.

## 3. Module

`v3/clients/kotlin/android/` (`:android`, included automatically by `settings.gradle.kts` once
`android/build.gradle.kts` exists), plugins `com.android.application`,
`org.jetbrains.kotlin.android`, `org.jetbrains.kotlin.plugin.compose` (versions from the catalog
that DSK-01 created):

- `namespace = "com.timelapserender.monitor.android"`, `applicationId =
  "com.timelapserender.monitor"`, debug `applicationIdSuffix = ".debug"`; `minSdk = 26`,
  `targetSdk = 36`, `compileSdk = 36`; Java and Kotlin target 17; Compose enabled; `buildConfig`
  enabled.
- `versionName` = the version (`-Ptrm.version`, else `v3/VERSION`); `versionCode` =
  `MAJOR×1000000 + MINOR×10000 + PATCH×100 + PRE` where `PRE` is `N` for `-alpha.N`, `32+N` for
  `-beta.N`, `65+N` for `-rc.N` (N from 1 to 32) and `99` for a release without a suffix (so
  `3.0.0-alpha.1` → 3000001 and `3.0.0` → 3000099).
- Release signing from the environment (`KEYSTORE_FILE`, `KEYSTORE_PASSWORD`, `KEY_ALIAS`,
  `KEY_PASSWORD`), declared before `buildTypes` (ReelVault's comment explains why); without
  `KEYSTORE_FILE` the release build is unsigned. Release: `isMinifyEnabled = true` with
  `proguard-rules.pro` keeping `com.timelapserender.trd.v1.**`, `-keep class * extends
  com.google.protobuf.GeneratedMessageLite { *; }`, and `-dontwarn` entries for grpc-okhttp's
  optional dependencies reported by R8.
- Dependencies: `project(":core")`; AndroidX (catalog entries added by DSK-01, newest stable on
  that day): `activity-compose`, `core-ktx`, `lifecycle-runtime-compose`,
  `lifecycle-viewmodel-compose`, `navigation-compose`, the Compose BOM with `material3`,
  `material3-adaptive-navigation-suite`, `material3.adaptive:adaptive-layout`,
  `material-icons-extended`, `datastore-preferences`; `kotlinx-coroutines-android`. Tests:
  `testFixtures(project(":core"))`, JUnit 4, Robolectric, `compose-ui-test-junit4`,
  `compose-ui-test-manifest` (debug), `androidx.test:core`.
- Manifest permissions: `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`,
  `CHANGE_WIFI_MULTICAST_STATE`. If, for the target SDK in use on the day of AND-02, the platform
  requires a runtime permission for local-network access, AND-02 declares and requests it on the
  Servers screen before discovery starts, with the rationale "Timelapse Render Monitor finds the
  Timelapse Render Daemon on your network."
- Theme: Material 3; dynamic colour on Android 12+, otherwise a scheme with primary `#1E2A44` and
  secondary `#F2A33A` (the icon's colours); light and dark follow the system.
- Launcher icon: adaptive icon with background colour `#1E2A44` and a foreground made from
  `v3/assets/icon/monitor-1024.png` (the icon at 72 % of the canvas, padded with the background
  colour), plus legacy `ic_launcher.png` per density, all generated once with `/usr/bin/sips` by
  `android/scripts/make-icons.sh` (run on the owner's Mac; outputs committed).
- English only.

## 4. Android-specific code

- `NsdDiscovery` implements `:core`'s `ServerDiscovery` with `NsdManager.discoverServices("_trd._tcp",
  NsdManager.PROTOCOL_DNS_SD, …)`; resolves with `registerServiceInfoCallback` on API 34+ and
  `resolveService` below (one resolve at a time on old APIs); reads TXT from
  `NsdServiceInfo.attributes` with `:core`'s TXT parser; holds a `WifiManager.MulticastLock` only
  while discovering (the Servers screen is visible).
- `AndroidServerStore` implements `:core`'s `ServerStore`: records in DataStore Preferences as
  JSON; tokens encrypted with AES-256-GCM using a key in the Android Keystore (alias
  `trm_token_key`), stored as base64 beside the record.
- `AppContainer` (created in `Application.onCreate`) holds the `ServerRegistry`, the active
  `TrdClient`, the presenters and `ThumbnailCache` (cache directory). Activities collect state with
  `repeatOnLifecycle(Lifecycle.State.STARTED)`; the snapshot stream runs only while started.
- Debug builds accept intent extras `trm_server` (`host:port`), `trm_fingerprint_prefix` and
  `trm_pair_code` that add and pair a server at launch (for `make develop`).
- Thumbnails decode with `BitmapFactory.decodeByteArray` into `ImageBitmap`.
- No background work, notifications or services (D15).

## 5. Screens

- Navigation: `NavigationSuiteScaffold` with **Activity**, **Log**, **Config**, **Servers**
  (bottom bar on phones, navigation rail on tablets and in landscape). With no paired server the
  app opens on Servers.
- Top app bar: server name and switcher menu (paired servers, "Add a server…"), the global pause
  action (asks for confirmation the first time, with "Don't ask again"), search (Activity and Log).
- **Activity**: on wide windows a `ListDetailPaneScaffold` (list and Details side by side); on
  phones Details is a separate destination. The list is a `LazyColumn` with the overview §6.1
  sections and rows (posters 96 dp wide, 16:9), stable keys, and the §6.2 rules exactly as the
  desktop does it (record "at top" before an update, `requestScrollToItem(0)` after it when it was,
  otherwise keep the anchor and show the `↑ N new` pill from `ScrollKeeper`). The header (overview
  §6.1 item 1) sits above the list and does not scroll. `PullToRefreshBox` sends `RescanNow`.
  "Show older" pages recently rendered videos.
- **Controls**: on each variant row the icon buttons spec 2 §6.4 lists per status; long-press on a
  card or row opens the same actions in a menu; swiping a failure card dismisses it; Stop asks for
  confirmation with the spec 2 §6.4 text and "Don't ask again" (DataStore). Rendered rows: Copy
  path and Details (no Open). Command failures show a snackbar with the server's message.
- **Details**: overview §6.3 content, with Copy buttons for the command line and the report.
- **Config**: a list of the overview §6.4 groups; each group is a screen of fields; path lists add
  through a folder-browser screen (`DirectoryBrowserModel`: roots, parent, folders, **Choose this
  folder**); variants edit in a bottom sheet (codec picker, frame rate, resolution mode, width,
  height); codecs in their own screen and sheet; issues under fields; **Save** in the top bar,
  disabled while errors exist; leaving with unsaved changes asks to save or discard; a conflict
  offers **Reload**.
- **Log**: overview §6.5, newest first, level filter chips, search, the same scroll rules.
- **Servers**: discovered servers with **Pair**; paired servers with their state, swipe to Forget;
  **Add a server…** (host, port, the 16-hex-digit fingerprint prefix shown in the desktop pairing
  dialog, then the code); the pairing dialog takes 6 digits on a number keyboard and pairs on the
  sixth; **About** (client version, server version, licence notice). Device name =
  `Settings.Global.DEVICE_NAME`, else `Build.MODEL`; platform `DEVICE_PLATFORM_ANDROID`.

## 6. Makefile (`v3/clients/kotlin/android/Makefile`, component `android`)

| Target | Command |
|---|---|
| `run` | `../gradlew -p .. :android:installDebug`, then `adb shell am start -n com.timelapserender.monitor.debug/com.timelapserender.monitor.android.MainActivity` (needs a connected device or running emulator) |
| `develop` | start `make -C ../../../trd develop-daemon` in the background, wait for `TRD_LISTENING`, read `code` and `fingerprint_prefix` from `../../../trd/target/debug/trd pair --json --address 127.0.0.1:50081`, install as `run`, launch with `--es trm_server $(DEVICE_HOST):50082 --es trm_fingerprint_prefix <fingerprint_prefix> --es trm_pair_code <code>` where `DEVICE_HOST` defaults to `10.0.2.2` (the emulator's view of the host; set it to the Mac's LAN address for a phone); stop the daemon on exit |
| `release` | `../gradlew -p .. :android:bundleRelease :android:assembleRelease` (signed when the `KEYSTORE_*` variables are set) |
| `clean` | `../gradlew -p .. :android:clean` |
| `test` | `../gradlew -p .. :android:testDebugUnitTest` |
| `check` | `../gradlew -p .. :android:lintDebug` |
| `ci` | `bash ../scripts/ci-android.sh` |

`run` and `develop` act on a device or emulator the owner provides; no ticket's tests use them
(overview §10 rule 1). Tests run on the JVM with Robolectric.

- `v3/clients/kotlin/scripts/ci-android.sh`: `./gradlew --no-daemon --console=plain
  :android:verifyVersionCodes :android:assembleDebug :android:testDebugUnitTest :android:lintDebug`.
- `v3/clients/kotlin/scripts/mason-android.sh`: `:android:verifyVersionCodes` and
  `:android:testDebugUnitTest` with `JAVA_HOME` and `ANDROID_HOME` from the environment, then merges
  `android/build/test-results/testDebugUnitTest/TEST-*.xml` into `build/mason-android-results.xml`,
  keeping Gradle's exit code.

## 7. GitHub Actions

- `v3-gate.yml` job `android` (defined by TRD-00) runs `scripts/ci-android.sh` once
  `android/build.gradle.kts` exists; nothing here edits it.
- `v3-android-build.yml` (AND-06): `workflow_call` with inputs `version` and `is_release`, secrets
  inherited; one job on `ubuntu-24.04`:
  1. checkout; `actions/setup-java@v5` (temurin 21); `gradle/actions/setup-gradle@v5`;
     `yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install "platforms;android-36" "build-tools;36.0.0"`;
  2. when `is_release`: fail with "::error::KEYSTORE_BASE64 not set" if the secret is empty; decode
     it to `$RUNNER_TEMP/trm-release.jks` and export `KEYSTORE_FILE`;
  3. `./gradlew -p v3/clients/kotlin --no-daemon -Ptrm.version=<v> :android:bundleRelease
     :android:assembleRelease` (when not `is_release`: `:android:assembleDebug` instead, uploaded as
     the workflow artifact `android-debug-apk`);
  4. when `is_release`: upload the AAB to Play with `r0adkll/upload-google-play@v1`
     (`serviceAccountJsonPlainText: PLAY_SERVICE_ACCOUNT_JSON`, `packageName:
     com.timelapserender.monitor`, `releaseFiles` the AAB, `mappingFile` the R8 mapping, `track:
     internal`, `status: completed`); copy the release APK to
     `TimelapseRenderMonitor-<v>-android.apk` and upload it as artifact `release-asset-android`;
  5. always delete the keystore file.
- `v3-release.yml` gains job `android` (`needs: [version]`, `uses: ./.github/workflows/v3-android-build.yml`,
  `with: version, is_release`, `secrets: inherit`), and `android` is appended to `publish.needs`.

## 8. Tickets

Each ticket's last line names the IDs it depends on; when filing, replace it with
`Depends on #<n>, #<m>` (overview §12). File spec 4 after spec 2.

### AND-01 — Android module skeleton

## Goal
`:android` builds a debug and release app that shows an empty Activity screen, with icons, theme,
signing, version rules, Makefile and CI scripts in place.

## Context
§3, §6; spec 2 §3 (the build it joins). The `android` gate job and the `android-test` mason suite
start running once `android/build.gradle.kts` and `scripts/mason-android.sh` exist. The owner has
installed SDK Platform 36 and Build-Tools 36.0.0 (overview §8).

## Files
- `v3/4-trm-android.md` (this spec; §3, §6)
- `v3/assets/icon/monitor-1024.png`
- `v3/clients/kotlin/android/build.gradle.kts` (new)
- `v3/clients/kotlin/android/proguard-rules.pro` (new)
- `v3/clients/kotlin/android/Makefile` (new)
- `v3/clients/kotlin/android/scripts/make-icons.sh` (new)
- `v3/clients/kotlin/android/src/main/AndroidManifest.xml` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/MainActivity.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/Theme.kt` (new)
- `v3/clients/kotlin/scripts/ci-android.sh` (new)
- `v3/clients/kotlin/scripts/mason-android.sh` (new)

## Approach
1. `build.gradle.kts` per §3. The version-code rule is a function in the build script; a task
   `:android:verifyVersionCodes` runs it over the vectors in A2 and fails on any mismatch;
   `scripts/ci-android.sh` and `scripts/mason-android.sh` run that task before the tests.
2. Manifest, `MainActivity` with the Material 3 theme and a placeholder, icons from
   `make-icons.sh`.
3. Makefile and scripts per §6.

## Acceptance criteria
- A1: `make -C v3/clients/kotlin/android test` passes and `assembleDebug` produces an APK — check: the command.
- A2: the version code vectors (`3.0.0-alpha.1` → 3000001, `3.0.0-beta.2` → 3000034, `3.0.0-rc.1` → 3000066, `3.0.0` → 3000099, `3.1.2` → 3010299) hold — check: `../gradlew -p .. :android:verifyVersionCodes`.
- A3: the `android` gate job is green — check: `v3-gate.yml`.

## Tests
`:android:verifyVersionCodes`; a Robolectric test that launches `MainActivity`.

## CI
`v3-gate.yml` `android`.

## Out of scope
Real screens.

Depends on: TRD-GATE, DSK-01

---

### AND-02 — Discovery, storage, app wiring, Servers and pairing

## Goal
The Android app finds `trd`s with NSD, pairs with a code (or a hand-entered address), stores the
token in the Keystore, and switches between paired servers.

## Context
§4, §5 (Servers); spec 2 §5.2–§5.3 (`ServerStore`, `ServerDiscovery`, `PairingClient`); ReelVault
`AndroidServerDiscovery.kt`, `AndroidTokenStorage.kt`.

## Files
- `v3/4-trm-android.md` (this spec; §4, §5)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/TrmApplication.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/AppContainer.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/platform/NsdDiscovery.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/platform/AndroidServerStore.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/AppScaffold.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/servers/ServersScreen.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/servers/PairingDialog.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/servers/AddServerScreen.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/MainActivity.kt`

## Approach
1. `NsdDiscovery`, `AndroidServerStore` and `AppContainer` per §4; the debug intent extras.
2. `AppScaffold` with the `NavigationSuiteScaffold` destinations and the start-on-Servers rule.
3. The Servers screen, Add server and the pairing dialog.

## Acceptance criteria
- A1: a token round-trips through `AndroidServerStore` encrypted (the stored value is not the token) — check: `AndroidServerStoreTest` (Robolectric with a fake keystore provider).
- A2: pairing against `FakeTrdService` from the dialog stores the server and switches to it — check: `ServersScreenTest` (Compose UI test under Robolectric).
- A3: NSD callbacks are translated into `DiscoveredServer`s through the shared TXT parser — check: `NsdDiscoveryTest` (fake `NsdManager` wrapper; no real NSD).

## Tests
The three test classes.

## CI
`v3-gate.yml` `android`.

## Out of scope
Activity (AND-03).

Depends on: TRD-GATE, AND-01, DSK-02, DSK-04

---

### AND-03 — Activity screen

## Goal
The Activity screen shows problems, failures, in-progress sequences and recently rendered videos
on phones and tablets, and never scrolls away from new entries.

## Context
§5 (Activity); overview §6.1, §6.2; spec 2 §5.4 (`ActivityPresenter`, `ScrollKeeper`,
`ThumbnailCache`).

## Files
- `v3/4-trm-android.md` (this spec; §5)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/activity/ActivityScreen.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/activity/ActivityHeader.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/activity/Rows.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/activity/NewItemsPill.kt` (new)

## Approach
The composables, `ListDetailPaneScaffold` on wide windows, the scroll handling, pull to refresh,
search and paging; Details is a placeholder until AND-04.

## Acceptance criteria
- A1: a scripted snapshot shows every section in order with the expected texts — check: `ActivityScreenTest` (Robolectric Compose test).
- A2: at the top a new card is visible after an update; scrolled down the anchor stays and the pill counts — check: `ActivityScrollTest`.
- A3: phone and tablet sizes render (Robolectric `@Config(qualifiers = …)`) — check: `ActivityScreenTest`.

## Tests
Robolectric Compose tests.

## CI
`v3-gate.yml` `android`.

## Out of scope
Controls (AND-04).

Depends on: TRD-GATE, AND-02, DSK-03

---

### AND-04 — Controls and Details

## Goal
Every overview §6.9 control is reachable on Android and Details shows everything overview §6.3
lists.

## Context
§5 (Controls, Details); spec 2 §6.4; spec 2 §5.4 (`Commands`, `DetailsModel`).

## Files
- `v3/4-trm-android.md` (this spec; §5)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/activity/RowActions.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/details/DetailsScreen.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/common/Confirmations.kt` (new)

## Approach
Row buttons and long-press menus per status, swipe-to-dismiss failures, confirmations, the global
pause action, snackbars, Details with Copy.

## Acceptance criteria
- A1: the action set per status equals spec 2 §6.4 and each action sends the right RPC — check: `RowActionsTest` (fake records calls).
- A2: Details shows the command line and log tail — check: `DetailsScreenTest`.

## Tests
Robolectric Compose tests.

## CI
`v3-gate.yml` `android`.

## Out of scope
Config and Log (AND-05).

Depends on: TRD-GATE, AND-03

---

### AND-05 — Config editor, folder browser and Log

## Goal
The whole config and codec table can be edited on Android with a server folder browser, and the log
can be read live.

## Context
§5 (Config, Log); overview §6.4, §6.5; spec 1 §6.1–§6.2; spec 2 §5.4 (`ConfigEditorModel`,
`DirectoryBrowserModel`, `LogModel`).

## Files
- `v3/4-trm-android.md` (this spec; §5)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/config/ConfigScreens.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/config/VariantSheet.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/config/CodecScreens.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/config/FolderBrowserScreen.kt` (new)
- `v3/clients/kotlin/android/src/main/kotlin/com/timelapserender/monitor/android/ui/log/LogScreen.kt` (new)

## Approach
The group screens, sheets and browser of §5; the Log screen with the scroll rules.

## Acceptance criteria
- A1: an invalid regex shows the server's issue and disables Save; a valid edit saves with the loaded revision — check: `ConfigScreensTest`.
- A2: choosing a folder adds it to the path list — check: `FolderBrowserScreenTest`.
- A3: the level chips filter lines and new lines appear at the top — check: `LogScreenTest`.

## Tests
Robolectric Compose tests.

## CI
`v3-gate.yml` `android`.

## Out of scope
Release (AND-06).

Depends on: TRD-GATE, AND-03

---

### AND-06 — Play release workflow and acceptance

## Goal
A `v3.*` tag uploads a signed bundle to the Play internal track and attaches a signed APK to the
draft release; the Android suite floor is raised; the owner accepts the app on a device.

## Context
§7; spec 1 §9.5; ReelVault `.github/workflows/android-release.yml`; overview §8 (Play Console
app, first manual upload, secrets).

## Files
- `v3/4-trm-android.md` (this spec; §7)
- `.github/workflows/v3-android-build.yml` (new)
- `.github/workflows/v3-release.yml`
- `.mason/mason.toml`

## Approach
1. `v3-android-build.yml` per §7; add the `android` job and the `publish.needs` entry to
   `v3-release.yml`.
2. A `workflow_dispatch` of `v3-release.yml` must pass the Android job (debug path).
3. Raise `android-test`'s `min_tests` to 90 % of the measured count.
4. Owner acceptance after the first tag: install from the Play internal track, pair with the Mac's
   `trd` through the desktop app's code, and use Activity, controls and Config; record the result.

## Acceptance criteria
- A1: the dispatch run's Android job is green and uploads `android-debug-apk` — check: the run.
- A2: on a tag with the secrets, the bundle reaches the internal track and `release-asset-android` holds the signed APK — check: the run and the Play Console (owner).
- A3: the floor is raised — check: `.mason/mason.toml`.

## Tests
The workflow run.

## CI
`v3-android-build.yml` (new), `v3-release.yml`.

## Out of scope
Promotion beyond the internal track (the owner does it in the Play Console).

Depends on: TRD-GATE, AND-04, AND-05

## 9. Dependency summary

| Ticket | Depends on |
|---|---|
| AND-01 | TRD-GATE, DSK-01 |
| AND-02 | TRD-GATE, AND-01, DSK-02, DSK-04 |
| AND-03 | TRD-GATE, AND-02, DSK-03 |
| AND-04 | TRD-GATE, AND-03 |
| AND-05 | TRD-GATE, AND-03 |
| AND-06 | TRD-GATE, AND-04, AND-05 |

## 10. Definition of done

- Every ticket's acceptance criteria hold; the `android` gate job is green; a `v3-release.yml`
  dispatch passes the Android job; a tagged build reaches the Play internal track and the owner has
  used it against the Mac's `trd`.
