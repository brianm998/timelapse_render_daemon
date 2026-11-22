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
  `compose-ui-test-manifest` (debug), `androidx.test:core`. Instrumented tests
  (`src/androidTest`, run only on an emulator in GitHub Actions, §7): `androidx.test:runner`,
  `androidx.test.ext:junit`, `compose-ui-test-junit4`; `testInstrumentationRunner =
  "androidx.test.runner.AndroidJUnitRunner"`.
- Coverage: Kover applied to `:android` (generated code excluded) with a verification rule for the
  debug variant whose minimum line coverage starts at 0 (AND-06 raises it to 50 %, overview §15.3).
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
| `coverage` | `../gradlew -p .. :android:koverVerifyDebug` (report under `android/build/reports/kover`) |

`run` and `develop` act on a device or emulator the owner provides; no ticket's tests use them
(overview §10 rule 1). Unit and UI tests run on the JVM with Robolectric; instrumented tests are
compiled everywhere and run only on an emulator in GitHub Actions (§7).

- `v3/clients/kotlin/scripts/ci-android.sh`: `./gradlew --no-daemon --console=plain
  :android:verifyVersionCodes :android:assembleDebug :android:assembleDebugAndroidTest
  :android:testDebugUnitTest :android:koverVerifyDebug :android:lintDebug` (Kover's verification
  task for the debug variant; if the Kover version names it differently, use the debug-variant
  verification task it provides); last, only when `CI=true` (on a GitHub runner, where touching
  an emulator would be harmless if the guard were broken), `env -u CI bash
  scripts/system-test-android.sh`, failing unless that exits 1 and prints
  `system-test-android.sh refuses to run without CI=true`.
- `v3/clients/kotlin/scripts/system-test-android.sh` (GitHub runners only): its first command
  after `set -euo pipefail` exits 1 with `system-test-android.sh refuses to run without CI=true`
  unless `CI` is `true`. Run by the emulator job of §7 once the emulator has booted; (1) `./gradlew --no-daemon
  :android:assembleDebug :android:assembleDebugAndroidTest`; (2) only then, so the 5-minute code
  cannot expire during the build, reads `code` and `fingerprint_prefix` from `$TRM_TRD_BIN pair
  --json --address 127.0.0.1:50081`; (3) `./gradlew --no-daemon :android:connectedDebugAndroidTest`
  with the instrumentation arguments `trm_server=$TRM_SERVER`, `trm_fingerprint_prefix` and
  `trm_pair_code`.
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
     internal`, `status: completed`); check the release APK with
     `"$ANDROID_HOME/build-tools/36.0.0/apksigner" verify --print-certs`, failing unless it
     verifies and the printed certificate SHA-256 equals that of the key in `KEYSTORE_FILE`
     (`keytool -list -v`), then copy it to `TimelapseRenderMonitor-<v>-android.apk` and
     upload it as artifact `release-asset-android`;
  5. always delete the keystore file.
- `v3-release.yml` gains job `android` (`needs: [version]`, `uses: ./.github/workflows/v3-android-build.yml`,
  `with: version, is_release`, `secrets: inherit`), and `android` is appended to both
  `system-tests.needs` and `publish.needs`.
- `v3-system-tests.yml` (spec 1 §9.7) gains, from AND-06, job `android-emulator` on
  `ubuntu-24.04` (`needs: [version]`; it builds from source):
  1. checkout; setup-java 21; setup-gradle; the SDK install line of the gate's `android` job;
     enable KVM (`echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' | sudo
     tee /etc/udev/rules.d/99-kvm4all.rules`, `sudo udevadm control --reload-rules`, `sudo udevadm
     trigger --name-match=kvm`);
  2. build `trd` and the fake tools (`cargo build -p trd --locked --manifest-path v3/trd/Cargo.toml`,
     `bash v3/trd/scripts/fake-tools-dir.sh "$RUNNER_TEMP/fake-tools"`); create a sandbox tree
     (`python3 -I v3/trd/sandbox/sandbox.py init --dir "$RUNNER_TEMP/sb"`); start `trd daemon
     --sandbox --config "$RUNNER_TEMP/sb/config.json" --data-dir "$RUNNER_TEMP/sb/data" --log-dir
     "$RUNNER_TEMP/sb/logs" --port 50081 --lan-bind 0.0.0.0:50082 --no-mdns` in the background with
     the fake tools; wait for `TRD_LISTENING`;
  3. `reactivecircus/android-emulator-runner@v2` with `api-level: 34`, `target: google_apis`,
     `arch: x86_64`, `emulator-options: -no-window -no-snapshot -noaudio -no-boot-anim`, and
     `script: CI=true TRM_TRD_BIN=$PWD/v3/trd/target/debug/trd TRM_SERVER=10.0.2.2:50082 bash
     v3/clients/kotlin/scripts/system-test-android.sh` (it issues the pairing code itself, after
     building);
  4. on failure, upload `v3/clients/kotlin/android/build/reports/androidTests` and the sandbox's
     `trd.log` as artifacts.

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
- `v3/clients/kotlin/android/src/androidTest/kotlin/com/timelapserender/monitor/android/MainActivityInstrumentedTest.kt` (new)
- `v3/clients/kotlin/scripts/ci-android.sh` (new)
- `v3/clients/kotlin/scripts/mason-android.sh` (new)
- `v3/clients/kotlin/scripts/system-test-android.sh` (new)

## Approach
1. `build.gradle.kts` per §3 (Kover with the debug-variant rule at 0; the instrumented-test runner
   and dependencies). The version-code rule is a function in the build script; a task
   `:android:verifyVersionCodes` runs it over the vectors in A2 and fails on any mismatch;
   `scripts/ci-android.sh` and `scripts/mason-android.sh` run that task before the tests.
2. Manifest, `MainActivity` with the Material 3 theme and a placeholder, icons from
   `make-icons.sh`.
3. `MainActivityInstrumentedTest`: launches `MainActivity` on the emulator and finds the placeholder
   text (it runs in the system tests once AND-06 adds the emulator job).
4. Makefile and scripts per §6.

## Acceptance criteria
- A1: `make -C v3/clients/kotlin/android test` passes and `assembleDebug` produces an APK — check: the command.
- A2: the version code vectors (`3.0.0-alpha.1` → 3000001, `3.0.0-beta.2` → 3000034, `3.0.0-rc.1` → 3000066, `3.0.0` → 3000099, `3.1.2` → 3010299) hold — check: `../gradlew -p .. :android:verifyVersionCodes`.
- A3: the `android` gate job is green, including `assembleDebugAndroidTest` and the Kover check — check: `v3-gate.yml`.
- A4: `system-test-android.sh` refuses to run without `CI=true` before touching any device or emulator — check: the refusal check at the end of `ci-android.sh`, in the gate's `android` job.
- A5: `check-makefiles.sh` passes for `v3/clients/kotlin/android` — check: `bash v3/scripts/check-makefiles.sh`.

## Tests
`:android:verifyVersionCodes`; a Robolectric test that launches `MainActivity`;
`MainActivityInstrumentedTest` (emulator, system test).

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
- `v3/clients/kotlin/android/src/androidTest/kotlin/com/timelapserender/monitor/android/KeystoreServerStoreInstrumentedTest.kt` (new)
- `v3/clients/kotlin/android/src/androidTest/kotlin/com/timelapserender/monitor/android/NsdDiscoveryInstrumentedTest.kt` (new)
- `v3/clients/kotlin/android/src/androidTest/kotlin/com/timelapserender/monitor/android/PairingInstrumentedTest.kt` (new)

## Approach
1. `NsdDiscovery`, `AndroidServerStore` and `AppContainer` per §4; the debug intent extras.
2. `AppScaffold` with the `NavigationSuiteScaffold` destinations and the start-on-Servers rule.
3. The Servers screen, Add server and the pairing dialog.
4. Instrumented tests (emulator, system test): `KeystoreServerStoreInstrumentedTest` stores,
   reads, replaces and removes a token with the real Android Keystore and checks that the stored
   preference value is not the token; `NsdDiscoveryInstrumentedTest` registers a `_trd._tcp`
   service with trd's TXT records through `NsdManager` on the emulator and checks that
   `NsdDiscovery` reports it with every field, then that unregistering removes it;
   `PairingInstrumentedTest` reads the instrumentation arguments `trm_server`,
   `trm_fingerprint_prefix` and `trm_pair_code` (it is skipped when they are absent), pairs through
   the Add server path with the host's sandbox `trd`, and checks that a snapshot arrives with the
   sandbox's sequences.

## Acceptance criteria
- A1: a token round-trips through `AndroidServerStore` encrypted (the stored value is not the token) — check: `AndroidServerStoreTest` (Robolectric with a fake keystore provider).
- A2: pairing against `FakeTrdService` from the dialog stores the server and switches to it — check: `ServersScreenTest` (Compose UI test under Robolectric).
- A3: NSD callbacks are translated into `DiscoveredServer`s through the shared TXT parser — check: `NsdDiscoveryTest` (fake `NsdManager` wrapper; no real NSD).
- A4: the debug intent extras add and pair a server at launch — check: `DebugExtrasTest` (Robolectric, against `FakeTrdService`).
- A5: the three instrumented tests pass on the emulator — check: the `android-emulator` job of `v3-system-tests.yml` (after AND-06); compiled locally and in the gate by `assembleDebugAndroidTest`.

## Tests
The test classes above, `DebugExtrasTest`, and the three instrumented tests.

## CI
`v3-gate.yml` `android`; `v3-system-tests.yml` `android-emulator`.

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

### AND-06 — Play release, emulator system tests and coverage

## Goal
A `v3.*` tag uploads a signed bundle to the Play internal track and attaches a signed APK to the
draft release; the instrumented tests run on an emulator against a real `trd` in CI; the coverage
and suite floors are raised.

## Context
§7; spec 1 §9.5, §9.7; overview §15.3 (coverage floors); ReelVault
`.github/workflows/android-release.yml`; overview §8 (Play Console app, first manual upload,
secrets); §11 (test plan).

## Files
- `v3/4-trm-android.md` (this spec; §7, §11)
- `.github/workflows/v3-android-build.yml` (new)
- `.github/workflows/v3-release.yml`
- `.github/workflows/v3-system-tests.yml`
- `v3/clients/kotlin/android/build.gradle.kts`
- `.mason/mason.toml`

## Approach
1. `v3-android-build.yml` per §7; add the `android` job to `v3-release.yml` and `android` to its
   `system-tests.needs` and `publish.needs`; add `android-emulator` to `v3-system-tests.yml`.
2. A `workflow_dispatch` of `v3-release.yml` must pass the Android job (debug path), and one of
   `v3-system-tests.yml` must pass `android-emulator`.
3. Raise the Kover debug-variant rule to 50 % line coverage, adding tests for whatever is
   uncovered until `ci-android.sh` passes.
4. Raise `android-test`'s `min_tests` to 90 % of the measured count.
5. Walk the §11 test plan and add any test it names that does not exist yet, until
   `python3 -I v3/scripts/check-test-names.py --spec 4` passes. From now on the gate's `t0` job
   runs it for spec 4 on every batch (`--completed` recognises `v3-android-build.yml`).

## Acceptance criteria
- A1: the dispatch run's Android job is green and uploads `android-debug-apk` — check: the run.
- A2: on a tag with the secrets, the Play upload step succeeds and `release-asset-android` holds an APK that `apksigner verify` accepts with the release certificate; that the bundle then installs from the internal track is the owner's manual check M-AND-1, outside this ticket — check: the tag's `v3-release.yml` run.
- A3: `android-emulator` is green: `MainActivityInstrumentedTest`, `KeystoreServerStoreInstrumentedTest`, `NsdDiscoveryInstrumentedTest` and `PairingInstrumentedTest` (against the sandbox `trd` on the runner) pass — check: the `v3-system-tests.yml` dispatch.
- A4: the gate's `android` job passes with the 50 % Kover floor — check: the gate run.
- A5: the `android-test` floor is raised — check: `.mason/mason.toml`.
- A6: every test that spec 4's acceptance criteria and §11 table name exists under that name — check: `python3 -I v3/scripts/check-test-names.py --spec 4`.

## Tests
The workflow runs; the instrumented tests; whatever step 3 adds.

## CI
`v3-android-build.yml` (new), `v3-release.yml`, `v3-system-tests.yml`.

## Out of scope
Promotion beyond the internal track (the owner does it in the Play Console). The manual checks of
§11.1.

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

- Every ticket's acceptance criteria hold; the `android` gate job is green at the overview §15.3
  coverage floor; `v3-system-tests.yml` `android-emulator` is green; a `v3-release.yml` dispatch
  passes the Android job; a tagged build reaches the Play internal track.
- The shared `:core` code Android uses is covered by spec 2's unit and contract tests (DSK-02 to
  DSK-04, DSK-12).
- Before a release is published, the owner has run the §11.1 manual checks and recorded the
  results on the AND-06 issue (overview §15.4).

## 11. Test plan

| Area | Automated tests | Kind | Ticket | Runs in |
|---|---|---|---|---|
| module, version codes, coverage, system-test script | `:android:verifyVersionCodes`, launch test, `check-makefiles.sh`, the `system-test-android.sh` refusal check in `ci-android.sh` | unit, static | AND-01 | mason `android-test`; gate `android`, `t0` |
| the app on a real Android | `MainActivityInstrumentedTest` | instrumented | AND-01 | system `android-emulator` |
| storage, discovery, Servers, pairing, debug extras | `AndroidServerStoreTest`, `ServersScreenTest`, `NsdDiscoveryTest`, `DebugExtrasTest` | unit, UI | AND-02 | mason `android-test`; gate `android` |
| real Keystore, real NSD, pairing with a real trd | `KeystoreServerStoreInstrumentedTest`, `NsdDiscoveryInstrumentedTest`, `PairingInstrumentedTest` | instrumented, end-to-end | AND-02 | system `android-emulator` |
| Activity screen, scrolling, phone and tablet layouts | `ActivityScreenTest`, `ActivityScrollTest` | UI | AND-03 | mason `android-test`; gate `android` |
| controls and Details | `RowActionsTest`, `DetailsScreenTest` | UI | AND-04 | same |
| Config editor, folder browser, Log | `ConfigScreensTest`, `FolderBrowserScreenTest`, `LogScreenTest` | UI | AND-05 | same |
| shared `:core` (connection, presenters, models, pairing) | spec 2's unit and contract tests | unit, contract | DSK-02–DSK-04, DSK-12 | gate `kotlin`, `contract` |
| release pipeline | dispatch runs; the upload and `apksigner verify` steps on a tag | workflow | AND-06 | `v3-release.yml`; manual M-AND-1 |
| coverage | Kover floor 50 % (`:android`) | coverage | AND-01, AND-06 | gate `android` |
| every named test exists | `check-test-names.py --spec 4` | static | AND-06 | gate `t0` on every batch once AND-06 has landed |

### 11.1 Manual checks (the owner, on an Android phone and tablet)

| ID | Steps | Expected result |
|---|---|---|
| M-AND-1 | Install from the Play internal track and open it on the home Wi-Fi. | The Servers screen lists the Mac's `trd` (after any local-network permission the platform asks for). |
| M-AND-2 | Pair with a code from the desktop app's **Pair a phone…**. | The sixth digit pairs; Activity shows the same sequences and videos as the desktop. |
| M-AND-3 | Watch a render to completion. | Progress advances; the finished sequence's videos appear at the top of Recently rendered with posters; the list stays at the top while it is at the top, and shows `↑ N new` when scrolled. |
| M-AND-4 | Use suspend, resume, stop and retry on a sandbox render (`make -C v3/trd develop`). | Each takes effect within seconds and the desktop app shows the same state. |
| M-AND-5 | Edit a harmless config value with the folder browser, save, then revert. | `trd` reloads it; the desktop app's Config window shows the change. |
| M-AND-6 | Rotate the phone; open the app on the tablet. | The phone keeps its place; the tablet shows list and Details side by side with a navigation rail. |
| M-AND-7 | Send the app to the background for a minute and bring it back. | It reconnects and catches up without user action. |
| M-AND-8 | Revoke the phone in the desktop app's Devices. | The app shows that it must be paired again. |
