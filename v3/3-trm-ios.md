<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Spec 3 — Timelapse Render Monitor for iOS and iPadOS (SwiftUI)

Read `v3/0-overview.md` first (especially §6) and `v3/1-trd-core-daemon.md` §7–§8. Every ticket
here depends on `TRD-GATE`. This spec does not depend on spec 2, but copies its behaviour rules
(spec 2 §5.4 presenters and §5.5 formatting) so all clients look and behave alike.

## 1. Outcome

- An iPhone and iPad app, **Timelapse Render Monitor** (`com.timelapserender.monitor`, iOS and
  iPadOS 18 or newer), that discovers `trd` over Bonjour, pairs with a code issued on `trd`'s
  machine, and implements overview §6 with full parity (D7): Activity, Details, every control,
  the Config editor with a server folder browser, the Log, and Servers. Devices (pairing-code
  issue and revocation) are not on iOS (overview §6.7).
- A local Swift package, `TRMKit`, with the networking, state and SwiftUI views, tested with
  `swift test` on macOS (no simulator).
- Version tags upload a signed build to TestFlight through GitHub Actions.

## 2. What to read

- Overview §2, §4, §6, §9, §10; spec 1 §7 (API), §8 (TLS identity, pairing, mDNS TXT records),
  §12 (the sandbox); spec 2 §5.4 and §5.5 (presenter behaviour and formatting, which iOS
  reproduces exactly).
- ReelVault at `228e3c8b7937291b1d53314eb3df92132f6757d6` (URL form in spec 1 §2):
  `kit/Package.swift`, `kit/regen-proto.sh`, `kit/Sources/ReelVaultKit/Networking/PinnedTLS.swift`
  (pinning by trusting the server's own certificate after a TLS probe), `ServerDiscovery.swift`
  (`NWBrowser`), `PairingClient.swift`, `ios/project.yml`, `ios/ReelVaultiOS/Info.plist`
  (Bonjour and local-network keys), `ios/Makefile`, `.github/workflows/ios-testflight.yml`.

## 3. Layout

```
v3/ios/
  project.yml                    XcodeGen; the .xcodeproj is generated and gitignored
  Makefile                       component `ios` (overview §9)
  scripts/ci.sh                  what v3-gate.yml's `ios` job runs
  scripts/mason-build.sh         the `ios-build` mason suite
  scripts/gen-proto.sh           regenerates TRMKit/Sources/TRMCore/Generated
  scripts/coverage.sh            per-target line coverage of TRMKit against coverage-floors.json
  scripts/contract-test.sh       TRMKit contract tests against a real trd (§4.6)
  scripts/system-test.sh         app-hosted tests on a simulator (GitHub runners only, §7)
  coverage-floors.json           {"TRMCore": n, "TRMUI": n}
  TRMKit/
    Package.swift                swift-tools 6.0; platforms iOS 18, macOS 15
    Sources/TRMCore/             models, networking, state, formatting (no SwiftUI)
    Sources/TRMCore/Generated/   trd.pb.swift, trd.grpc.swift, PROTO_SHA256 (committed)
    Sources/TRMUI/               SwiftUI views; platform-neutral so they compile for macOS too
    Tests/TRMCoreTests/          XCTest, including Contract/ (skipped unless TRM_E2E=1)
    Tests/TRMUITests/            XCTest (offscreen ImageRenderer renders)
  TimelapseRenderMonitor/        the app target: App.swift, Info.plist, Assets.xcassets, PrivacyInfo.xcprivacy
  TimelapseRenderMonitorTests/   app-hosted unit tests (real Keychain, real Bonjour, launch arguments)
```

`TRMKit` dependencies: `swift-protobuf` (from 1.28.0), `grpc-swift` (from 2.2.1; products
`GRPCCore` and, for tests, `GRPCInProcessTransport`), `grpc-swift-nio-transport` (from 1.0.0;
`GRPCNIOTransportHTTP2`), `grpc-swift-protobuf` (exactly 1.3.1; its generator produced the
committed code). `Package.resolved` is committed. Tests use XCTest (the mason `ios-kit-test`
suite parses XCTest output). The package uses the Swift 6 language mode; the app target uses
Swift 5 mode (`SWIFT_VERSION: "5.0"`), as ReelVault does. English only.

`project.yml`: target `TimelapseRenderMonitor` (application, iOS), `PRODUCT_BUNDLE_IDENTIFIER:
com.timelapserender.monitor`, deployment target 18.0, `TARGETED_DEVICE_FAMILY: "1,2"`,
`MARKETING_VERSION` from `v3/VERSION` without a pre-release suffix (updated by
`v3/scripts/bump-version.sh`), `CURRENT_PROJECT_VERSION: "1"` (CI overrides it),
`DEVELOPMENT_TEAM: G3L75S65V9`, automatic signing in Debug, manual signing in Release with
`CODE_SIGN_IDENTITY: Apple Distribution` and `PROVISIONING_PROFILE_SPECIFIER:
$(IOS_PROVISION_PROFILE_SPECIFIER)`; package dependency `TRMKit` (path `TRMKit`, products `TRMCore`
and `TRMUI`). A second target `TimelapseRenderMonitorTests` (`bundle.unit-test`, host application
`TimelapseRenderMonitor`, sources `TimelapseRenderMonitorTests`) is part of the scheme's test
action; it is compiled by `build-for-testing` everywhere and run only on GitHub runners (§7).
`Info.plist`: `CFBundleDisplayName` "Timelapse Render Monitor",
`NSLocalNetworkUsageDescription` "Timelapse Render Monitor finds and connects to the Timelapse
Render Daemon on your network.", `NSBonjourServices` `[_trd._tcp]`,
`NSAppTransportSecurity`/`NSAllowsLocalNetworking` true, `ITSAppUsesNonExemptEncryption` false
(the app uses only standard TLS), `UILaunchScreen` empty dictionary, all four orientations on iPad
and portrait plus landscape on iPhone. `PrivacyInfo.xcprivacy` declares `UserDefaults` access
reason `CA92.1` and no tracking. The app icon is `v3/assets/icon/monitor-1024.png` as a single-size
`AppIcon` in `Assets.xcassets`.

## 4. TRMCore

### 4.1 Networking

- `PinnedCertificate`: `probe(host:port:expectedFingerprint:)` performs a TLS handshake with
  `URLSession` and returns the leaf certificate's DER when its SHA-256 (lowercase hex) equals the
  expected fingerprint (or, for a hand-entered server, starts with the entered 16-hex-digit
  prefix); the DER is stored with the server record so later connections need no probe.
- Transport: `HTTP2ClientTransport.Posix` with `.plaintext` for a loopback address (the iOS
  simulator reaching `127.0.0.1`) and, for LAN, TLS whose only trust root is the stored DER and
  whose `serverCertificateVerification` is `.noHostnameVerification` (ReelVault `PinnedTLS.swift`).
- `TrdClient(endpoint, token)`: wraps the generated `Trd_V1_TrdService.Client`; adds
  `authorization: Bearer <token>` metadata; `serverInfo()`, `watchSnapshots(maxPerSecond:)` as an
  `AsyncThrowingStream`, every unary RPC as `async throws`, `watchLog(afterSeq:)`.
- `ConnectionState` with the same cases, messages and back-off (1, 2, 5, 10 s) as spec 2 §5.1;
  `NeedsPairing` on `unauthenticated`; `Incompatible` when `api_version != 1`; a certificate whose
  fingerprint no longer matches → `IdentityChanged` ("This server's identity changed. Forget it
  and pair again.").
- Lifecycle: streaming stops when the scene moves to the background and resumes (reconnecting) when
  it becomes active.

### 4.2 Servers and storage

- `ServerRecord(serverId, name, host, port, fingerprint, certificateDER, platform, lastSeenMs)`
  stored in `UserDefaults` (suite `com.timelapserender.monitor`) as JSON; the token is stored in the
  Keychain (`kSecClassGenericPassword`, service `com.timelapserender.monitor.token`, account =
  server id, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
- `ServerRegistry`: stored records plus discovered services; the active server id.

### 4.3 Discovery and pairing

- `ServerDiscovery`: `NWBrowser` for `.bonjourWithTXTRecord(type: "_trd._tcp", domain: nil)`;
  parses TXT `id`, `name`, `fp`, `port`, `ver`, `api` (records missing `id`, `fp` or `port`, or
  with `api` other than `1`, are ignored); resolves the endpoint's host with an `NWConnection`
  when the TXT has no address.
- `PairingClient.pair(server, code, deviceName)`: probe with the TXT fingerprint (or the entered
  prefix), call `Pair` without a token, confirm with `GetServerInfo` that `caller_is_paired`, store
  the record and the token; the error texts of spec 2 §5.3. The device name is
  `UIDevice.current.name`, platform `DEVICE_PLATFORM_IOS`.

### 4.4 State

Swift ports of spec 2 §5.4 with identical behaviour and output strings: `SnapshotRepository`,
`ActivityModel` (the presenter), `ScrollKeeper`, `Commands`, `ThumbnailCache` (memory LRU of 200
plus a disk cache of 200 MB per server in the caches directory), `DetailsModel`, `LogModel`,
`ConfigEditorModel` (400 ms validation debounce, revision-checked saves, conflict flag),
`DirectoryBrowserModel`. `Format` reproduces spec 2 §5.5 exactly (a shared table of test vectors in
`TRMCoreTests/FormatVectors.json`, copied from the Kotlin `FormatTest`). All models are
`@Observable` and `@MainActor`.

### 4.5 Test support

`FakeTrdService` implements every RPC over `GRPCInProcessTransport`, scripted like the Kotlin
fake (spec 2 §5.6). Unit, integration and UI tests use it. Only the contract tests (§4.6) talk to
a real `trd`; only the app-hosted tests (§7) open a Bonjour browser or touch the real Keychain,
and they run only on a simulator on GitHub runners.

### 4.6 Contract tests

`TRMKit/Tests/TRMCoreTests/Contract/` (skipped unless `TRM_E2E=1`) runs `TRMCore` against a real
`trd` from the same commit, exactly as the Kotlin contract tests do (spec 2 §5.7): a `TrdProcess`
helper builds a sandbox tree with `python3 -I v3/trd/sandbox/sandbox.py init --dir <tmp>`, starts
`$TRM_TRD_BIN daemon --sandbox … --port 0 --lan-bind 127.0.0.1:0 --no-mdns` with the fake tools
(`$TRM_FAKE_TOOLS_DIR`) and a per-test `TRD_FAKE_FFMPEG` script, reads `TRD_LISTENING`, and stops
it afterwards. Scenarios: connection and `api_version` 1; a sequence's stages into Recently rendered
through `ActivityModel`; every control of overview §6.9 with its effect on the snapshot and on
`config.json`; `ConfigEditorModel` load, validation, save and a revision conflict; `UpdateCodecs`;
pairing through `PinnedCertificate.probe` and `PairingClient` against the real certificate,
streaming with the token, a wrong code, the attempt limit and revocation (`NeedsPairing`);
log events; a real thumbnail through `ThumbnailCache`; `DirectoryBrowserModel` on the sandbox.
`scripts/contract-test.sh` builds `trd` and the fake tools when `TRM_TRD_BIN` is unset (spec 2
§5.7 describes the same steps) and runs `swift test --package-path TRMKit --filter Contract` with
`TRM_E2E=1`; the gate's `contract` job runs it on macOS, and it is safe on the owner's Mac.

## 5. TRMUI and the app

### 5.1 Navigation

- iPhone (compact width): a `TabView` with **Activity**, **Log**, **Config**, **Servers**.
  Selecting a row in Activity pushes its Details screen.
- iPad (regular width): a `NavigationSplitView`; sidebar **Activity**, **Log**, **Config**,
  **Servers**; Activity shows the list in the content column and Details of the selection in the
  detail column.
- The navigation title is the server name; the toolbar has the server switcher (menu: paired
  servers, "Add a server…") and the global pause button (pause/play icon; turning it on asks for
  confirmation the first time, with "Don't ask again").
- With no paired server, the app opens on **Servers** with the discovery list.

### 5.2 Activity

- Header (overview §6.1 item 1) pinned above the list with `.safeAreaInset(edge: .top)`:
  connection text, phase chip, `SANDBOX` badge, counts line.
- The list is a `ScrollView` with a `LazyVStack(pinnedViews: [.sectionHeaders])` using
  `.scrollTargetLayout()` and `.scrollPosition(id: $anchorID, anchor: .top)`, so the anchored row
  stays put when rows are inserted above it. Before applying a new snapshot, record whether the
  first row was the anchor (at the top); after it, set `anchorID` to the new first row when it was
  (overview §6.2 S2), otherwise keep it and show the `↑ N new` pill from `ScrollKeeper` (S3).
  Tapping the pill scrolls to the top with animation.
- Rows follow overview §6.1: problem banners, failure cards, sequence cards with variant rows,
  recently-rendered rows with 96-pt-wide 16:9 posters (decoded from `ThumbnailCache`), "Show older".
- `.searchable` filters by sequence and file name.
- Pull to refresh sends `RescanNow`.

### 5.3 Controls and Details

- Variant rows: swipe actions and a context menu with exactly the buttons spec 2 §6.4 lists per
  status. Sequence cards: context menu Pause / Resume sequence. Failure cards: Retry, Dismiss,
  Details. Rendered rows: Copy path, Details (no Open or Reveal on iOS).
- Stop asks for confirmation (the spec 2 §6.4 text) with "Don't ask again".
- A failed command shows an alert with the server's message.
- Details screens: overview §6.3 content, with Copy buttons for the command line and the report.

### 5.4 Config, Log and Servers

- **Config**: a `Form` with one section per overview §6.4 group; path lists edit through a pushed
  folder browser (`DirectoryBrowserModel`: roots, parent, folders; **Choose this folder**); the
  rendering table as a list of variants, each editing in a sheet (codec picker, frame rate,
  resolution mode, width, height); codecs in a pushed list with an editor sheet; issues shown under
  fields; **Save** in the toolbar, disabled while errors exist; leaving with unsaved changes asks
  to save or discard; a conflict offers **Reload**.
- **Log**: overview §6.5, newest first, level picker, search, the same scroll rules.
- **Servers**: discovered servers (name, version) with **Pair**; paired servers with their state
  and **Forget** (swipe); **Add a server…** (host, port, the 16-hex-digit fingerprint prefix shown
  in the desktop pairing dialog, then the code); the pairing sheet takes 6 digits with a number pad
  and pairs on the sixth digit; **About** (client version, server version, licence notice).

### 5.5 Development launch arguments (Debug builds only)

`-TRMServer host:port`, `-TRMFingerprintPrefix <16 hex>` and `-TRMPairCode <6 digits>` add and pair
a server at launch without typing; `make develop` uses them with the sandbox (§6).

## 6. Makefile (`v3/ios/Makefile`, component `ios`)

| Target | Command |
|---|---|
| `project` | `xcodegen generate` |
| `run` | `project`, build Debug for the iOS Simulator (`-destination 'platform=iOS Simulator,name=iPhone 17'`), boot that simulator if needed, `xcrun simctl install` and `launch` the app (it then discovers the installed `trd`) |
| `develop` | start `make -C ../trd develop-daemon` in the background, wait for `TRD_LISTENING`, read `code` and `fingerprint_prefix` from `../trd/target/debug/trd pair --json --address 127.0.0.1:50081`, then as `run` but launch with `-TRMServer 127.0.0.1:50082 -TRMFingerprintPrefix <fingerprint_prefix> -TRMPairCode <code>`; stop the daemon on exit |
| `release` | `project`, `xcodebuild archive` (Release) and `-exportArchive` to `build/` with `method app-store-connect`, `destination export`; signed only when an Apple Distribution identity and the provisioning profile are installed locally |
| `clean` | remove `TimelapseRenderMonitor.xcodeproj`, `build/`, `DerivedData/`, `TRMKit/.build` |
| `test` | `swift test --package-path TRMKit` |
| `check` | `swift build --package-path TRMKit -Xswiftc -warnings-as-errors` |
| `ci` | `bash scripts/ci.sh` |
| `coverage` | `swift test --package-path TRMKit --enable-code-coverage` then `bash scripts/coverage.sh` |
| `contract` | `bash scripts/contract-test.sh` (§4.6) |

`run` and `develop` open the Simulator on the owner's screen; they are for the owner only, and no
ticket's tests use them (overview §10 rule 1). The app-hosted tests also need a simulator, so they
have no local target; `scripts/system-test.sh` runs them on GitHub runners.

- `scripts/ci.sh`: `xcodegen generate`; `swift test --package-path TRMKit --enable-code-coverage`;
  `bash scripts/coverage.sh`; `xcodebuild build-for-testing -project TimelapseRenderMonitor.xcodeproj
  -scheme TimelapseRenderMonitor -sdk iphonesimulator -destination 'generic/platform=iOS Simulator'
  -derivedDataPath DerivedData CODE_SIGNING_ALLOWED=NO` (compiles the app and its app-hosted tests);
  last, only when `CI=true` (on a GitHub runner, where a simulator boot would be harmless if the
  guard were broken), `env -u CI bash scripts/system-test.sh`, failing unless that exits 1 and
  prints `system-test.sh refuses to run without CI=true`.
- `scripts/coverage.sh [FLOORS_FILE]`: reads the JSON at `swift test --package-path TRMKit
  --show-codecov-path`, sums line coverage over the files under `Sources/TRMCore` (excluding
  `Generated/`) and `Sources/TRMUI`, prints both, and exits 1 when either is below its floor in
  `FLOORS_FILE` (default `coverage-floors.json`; both 0 until IOS-09, then the overview §15.3
  floors).
- `scripts/mason-build.sh`: `xcodegen generate` and the same `xcodebuild build-for-testing`, printing
  `test ios_build ... ok` or `test ios_build ... FAILED`.
- `scripts/system-test.sh` (GitHub runners only): its first command after `set -euo pipefail`
  exits 1 with `system-test.sh refuses to run without CI=true` unless `CI` is `true`; then it picks the newest
  available iPhone simulator from `xcrun simctl list devices available -j` and runs `xcodebuild test
  -project TimelapseRenderMonitor.xcodeproj -scheme TimelapseRenderMonitor -destination
  'platform=iOS Simulator,id=<udid>' -only-testing:TimelapseRenderMonitorTests CODE_SIGNING_ALLOWED=NO`.
- `scripts/gen-proto.sh`: builds `protoc-gen-swift` (from the resolved `swift-protobuf` checkout)
  and `protoc-gen-grpc-swift-2` (from `grpc-swift-protobuf` 1.3.1) with `swift build -c release`,
  runs `protoc` (Homebrew `protobuf`) with `Visibility=Public` on `v3/proto/trd/v1/trd.proto`,
  writes `Generated/`, and writes the SHA-256 of the proto into `Generated/PROTO_SHA256`.

## 7. GitHub Actions

- `v3-gate.yml` job `ios` (defined by TRD-00) runs `scripts/ci.sh` once `v3/ios/project.yml`
  exists, and job `contract` runs `scripts/contract-test.sh` on macOS once IOS-08 creates it;
  nothing here edits the gate.
- `v3-ios-build.yml` (IOS-09): `workflow_call` with inputs `version` and `is_release`, secrets
  inherited; one job on `macos-15`:
  1. checkout; `maxim-lobanov/setup-xcode@v1` (`latest-stable`); `brew install xcodegen`;
     `xcodegen generate` in `v3/ios`;
  2. `swift test --package-path v3/ios/TRMKit`;
  3. when `is_release`: fail with "::error::IOS_DIST_CERT_P12_BASE64 not set" if the secret is
     empty; import the Apple Distribution certificate into a temporary keychain; install the
     provisioning profile and read its name with `security cms -D -i … | plutil -extract Name raw -o - -`;
     stage the App Store Connect API key;
  4. `xcodebuild archive -project TimelapseRenderMonitor.xcodeproj -scheme TimelapseRenderMonitor
     -configuration Release -sdk iphoneos -archivePath $RUNNER_TEMP/TRM.xcarchive
     DEVELOPMENT_TEAM=$APPLE_TEAM_ID IOS_PROVISION_PROFILE_SPECIFIER=<profile name>
     MARKETING_VERSION=<version without pre-release suffix> CURRENT_PROJECT_VERSION=$GITHUB_RUN_NUMBER`
     (when not `is_release`: an unsigned simulator build replaces steps 3–5);
  5. write `ExportOptions.plist` (`method` `app-store-connect`, `teamID`, `signingStyle` `manual`,
     `provisioningProfiles` `{com.timelapserender.monitor: <profile name>}`, `uploadSymbols` true,
     `destination` `upload`) and run `xcodebuild -exportArchive -archivePath … -exportOptionsPlist …
     -exportPath $RUNNER_TEMP/export -authenticationKeyPath <p8> -authenticationKeyID $APPLE_API_KEY_ID
     -authenticationKeyIssuerID $APPLE_API_ISSUER_ID`, which uploads the build to TestFlight;
  6. always delete the keychain, the key and the profile.
  No GitHub Release asset is produced for iOS (an `.ipa` cannot be installed without provisioning).
- `v3-release.yml` gains job `ios` (`needs: [version]`, `uses: ./.github/workflows/v3-ios-build.yml`,
  `with: version, is_release`, `secrets: inherit`), and `ios` is appended to both
  `system-tests.needs` and `publish.needs`.
- `v3-system-tests.yml` (spec 1 §9.7) gains, from IOS-09, job `ios-app-tests` on `macos-15`
  (`needs: [version]`; it builds from source, so it needs no package): checkout; setup-xcode
  `latest-stable`; `brew install xcodegen`; `xcodegen generate` in `v3/ios`; `CI=true bash
  v3/ios/scripts/system-test.sh`; on failure, upload the `.xcresult` bundle as an artifact.

## 8. Tickets

Each ticket's last line names the IDs it depends on; when filing, replace it with
`Depends on #<n>, #<m>` (overview §12).

### IOS-01 — iOS project, TRMKit package and generated API code

## Goal
`v3/ios` generates an Xcode project for an iPhone and iPad app linked to the `TRMKit` package,
which contains the generated `trd.v1` Swift code and builds and tests on macOS.

## Context
§3, §6; ReelVault `kit/Package.swift`, `kit/regen-proto.sh`, `ios/project.yml`, `ios/Makefile`.
The `ios` gate job and the `ios-kit-test` and `ios-build` mason suites start running once
`project.yml`, `TRMKit/Package.swift` and `scripts/mason-build.sh` exist.

## Files
- `v3/3-trm-ios.md` (this spec; §3, §6)
- `v3/assets/icon/monitor-1024.png`
- `v3/ios/project.yml` (new)
- `v3/ios/Makefile` (new)
- `v3/ios/scripts/ci.sh` (new)
- `v3/ios/scripts/mason-build.sh` (new)
- `v3/ios/scripts/gen-proto.sh` (new)
- `v3/ios/scripts/coverage.sh` (new)
- `v3/ios/scripts/system-test.sh` (new)
- `v3/ios/coverage-floors.json` (new)
- `v3/ios/TimelapseRenderMonitorTests/LaunchArgumentsTests.swift` (new)
- `v3/ios/TRMKit/Package.swift` (new)
- `v3/ios/TRMKit/Package.resolved` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Generated/trd.pb.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Generated/trd.grpc.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Generated/PROTO_SHA256` (new)
- `v3/ios/TRMKit/Sources/TRMUI/TRMUI.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/GeneratedCodeTests.swift` (new)
- `v3/ios/TimelapseRenderMonitor/App.swift` (new)
- `v3/ios/TimelapseRenderMonitor/Info.plist` (new)
- `v3/ios/TimelapseRenderMonitor/PrivacyInfo.xcprivacy` (new)
- `v3/ios/TimelapseRenderMonitor/Assets.xcassets/AppIcon.appiconset/Contents.json` (new)

## Approach
1. `Package.swift` and `project.yml` per §3 (including the `TimelapseRenderMonitorTests` target);
   the app's `App.swift` shows "Not connected".
2. Run `gen-proto.sh` once and commit `Generated/`.
3. Makefile and scripts per §6; `coverage-floors.json` with both floors at 0 (IOS-09 raises
   them); `.gitignore` entries already exist (TRD-00).
4. `LaunchArgumentsTests`: the parser of the §5.5 Debug launch arguments (the parser itself lives
   in `TRMCore`, so its logic is also unit-tested there; this app-hosted test proves the app
   target reads `ProcessInfo.arguments` through it).

## Acceptance criteria
- A1: `swift test --package-path v3/ios/TRMKit` passes, including a test that `PROTO_SHA256` equals the SHA-256 of `v3/proto/trd/v1/trd.proto` — check: `make -C v3/ios test`.
- A2: the app and its app-hosted tests compile for the simulator unsigned — check: `bash v3/ios/scripts/mason-build.sh` prints `test ios_build ... ok`.
- A3: the `ios` gate job is green, including `coverage.sh` — check: `v3-gate.yml`.
- A4: `coverage.sh` reports TRMCore and TRMUI line coverage and fails when a floor is above it — check: `bash v3/ios/scripts/coverage.sh` with a temporary floors file set to 101.
- A5: `system-test.sh` refuses to run without `CI=true` before touching any simulator — check: the refusal check at the end of `scripts/ci.sh`, in the gate's `ios` job.
- A6: `check-makefiles.sh` passes for `v3/ios` — check: `bash v3/scripts/check-makefiles.sh`.

## Tests
`GeneratedCodeTests` (the hash check and a `Trd_V1_Snapshot` round trip); `LaunchArgumentsTests`
(app-hosted, run by the system tests).

## CI
`v3-gate.yml` `ios`.

## Out of scope
Networking and UI.

Depends on: TRD-GATE

---

### IOS-02 — TRMCore networking: pinned TLS, tokens, servers, client

## Goal
`TRMCore` connects to `trd` over pinned TLS (or plaintext on loopback) with a Keychain-stored
token, and reports connection state like the Kotlin clients.

## Context
§4.1, §4.2; spec 2 §5.1 (states, messages, back-off); ReelVault `PinnedTLS.swift`.

## Files
- `v3/3-trm-ios.md` (this spec; §4.1, §4.2)
- `v3/ios/TRMKit/Sources/TRMCore/Net/PinnedCertificate.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Net/Transport.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Net/TrdClient.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Net/ConnectionState.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Servers/ServerStore.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Servers/TokenKeychain.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Servers/ServerRegistry.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Fakes/FakeTrdService.swift` (new)
- `v3/ios/TimelapseRenderMonitorTests/KeychainTests.swift` (new)

## Approach
1. The §4.1 and §4.2 types; the Keychain access behind a protocol with an in-memory test
   implementation.
2. `FakeTrdService` over `GRPCInProcessTransport` (§4.5).
3. `KeychainTests` (app-hosted, system test): the real `TokenKeychain` stores, reads, replaces and
   deletes a token with the §4.2 attributes, and a token stored for one server id is not returned
   for another.

## Acceptance criteria
- A1: the fingerprint check accepts the matching DER and rejects another — check: `PinnedCertificateTests` (DER fixtures generated in the test).
- A2: `unauthenticated` gives `NeedsPairing`; API version 2 gives `Incompatible`; the back-off is 1, 2, 5, 10 s (injected clock) — check: `TrdClientTests`.
- A3: records round-trip through `UserDefaults` and tokens through the Keychain protocol — check: `ServerStoreTests`.
- A4: a certificate whose fingerprint changed yields `IdentityChanged` — check: `TrdClientTests`.
- A5: the real Keychain behaves as A3 requires — check: `KeychainTests` in the `ios-app-tests` system job (after IOS-09); compiled locally by `mason-build.sh`.

## Tests
The three test classes, against `FakeTrdService`; `KeychainTests` (app-hosted).

## CI
`v3-gate.yml` `ios`; `v3-system-tests.yml` `ios-app-tests`.

## Out of scope
State models (IOS-03).

Depends on: TRD-GATE, IOS-01

---

### IOS-03 — TRMCore state: presenters, formatting and models

## Goal
`TRMCore` produces the same UI state and strings as the Kotlin `:core` from snapshots, logs and
configs.

## Context
§4.4; spec 2 §5.4, §5.5; overview §6.1–§6.5, §6.9.

## Files
- `v3/3-trm-ios.md` (this spec; §4.4)
- `v3/ios/TRMKit/Sources/TRMCore/State/SnapshotRepository.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/ActivityModel.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/ScrollKeeper.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/Commands.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/ThumbnailCache.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/DetailsModel.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/LogModel.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/ConfigEditorModel.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/State/DirectoryBrowserModel.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Format/Format.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/FormatVectors.json` (new)

## Approach
Port each spec 2 §5.4 class and §5.5 rule; `FormatVectors.json` holds input/output pairs for every
§5.5 row, identical to the Kotlin `FormatTest` cases.

## Acceptance criteria
- A1: `ScrollKeeper` implements overview §6.2 S1–S7 — check: `ScrollKeeperTests`.
- A2: `ActivityModel`'s sections, order, keys and strings for a scripted snapshot equal a golden list — check: `ActivityModelTests`.
- A3: every vector in `FormatVectors.json` passes — check: `FormatTests`.
- A4: the config editor debounces validation, saves with the revision and flags conflicts — check: `ConfigEditorModelTests` (injected clock).

## Tests
One test class per model.

## CI
`v3-gate.yml` `ios`.

## Out of scope
Views.

Depends on: TRD-GATE, IOS-02

---

### IOS-04 — Discovery, pairing and the Servers screen

## Goal
The app finds `trd`s on the network, pairs with one using a 6-digit code (or a hand-entered
address), and switches between paired servers.

## Context
§4.3, §5.1, §5.4 (Servers), §5.5; spec 1 §8.3–§8.4; ReelVault `ServerDiscovery.swift`,
`PairingClient.swift`.

## Files
- `v3/3-trm-ios.md` (this spec; §4.3, §5.4)
- `v3/ios/TRMKit/Sources/TRMCore/Discovery/ServerDiscovery.swift` (new)
- `v3/ios/TRMKit/Sources/TRMCore/Pairing/PairingClient.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Servers/ServersView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Servers/PairingSheet.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Servers/AddServerView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/AppShell.swift` (new)
- `v3/ios/TimelapseRenderMonitor/App.swift`
- `v3/ios/TimelapseRenderMonitorTests/BonjourDiscoveryTests.swift` (new)

## Approach
1. TXT parsing as a pure function; `NWBrowser` wrapper (not started by TRMKit tests).
2. `PairingClient` with the spec 2 §5.3 messages; Debug launch arguments of §5.5.
3. `AppShell`: the §5.1 navigation for compact and regular widths, the server switcher, and the
   start-on-Servers rule.
4. `BonjourDiscoveryTests` (app-hosted, system test): publishes a `_trd._tcp` service with trd's
   TXT records from an `NWListener` inside the test and checks that `ServerDiscovery` reports it with
   every field, then that cancelling the listener removes it.

## Acceptance criteria
- A1: TXT parsing accepts a valid record and rejects each missing field and `api=2` — check: `DiscoveryTests`.
- A2: pairing against the fake stores the record and token; the wrong-code and exhausted messages match spec 2 §5.3 — check: `PairingClientTests`.
- A3: `ServersView` renders offscreen for no servers, discovered servers and paired servers — check: `ServersViewTests` (`ImageRenderer` on macOS; PNGs written to `.build/screenshots/` for review).
- A4: the app shell starts on Servers with no paired server and on Activity with one, in both size classes — check: `AppShellTests` (model-level).
- A5: real Bonjour discovery works inside the simulator — check: `BonjourDiscoveryTests` in the `ios-app-tests` system job (after IOS-09).

## Tests
The test classes above; `AppShellTests`; `BonjourDiscoveryTests` (app-hosted).

## CI
`v3-gate.yml` `ios`; `v3-system-tests.yml` `ios-app-tests`.

## Out of scope
Activity (IOS-05).

Depends on: TRD-GATE, IOS-03

---

### IOS-05 — Activity screen

## Goal
The Activity screen shows problems, failures, in-progress sequences and recently rendered videos
on iPhone and iPad, and never scrolls away from new entries.

## Context
§5.2; overview §6.1, §6.2; `ActivityModel`, `ScrollKeeper`, `ThumbnailCache` (IOS-03).

## Files
- `v3/3-trm-ios.md` (this spec; §5.2)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/ActivityView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/HeaderView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/ProblemBanner.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/FailureCardView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/SequenceCardView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/RenderedRowView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/NewItemsPill.swift` (new)

## Approach
The views and the §5.2 scroll handling, search, pull to refresh and "Show older"; Details is a
placeholder until IOS-06.

## Acceptance criteria
- A1: the anchor logic keeps the top when at the top and keeps the anchored row with a correct pill count when scrolled — check: `ActivityScrollTests` (drives the view's scroll-state helper with scripted snapshots).
- A2: every row type renders offscreen in light and dark mode at compact and regular widths — check: `ActivityViewTests` (`ImageRenderer`; PNGs for review).

## Tests
The two test classes.

## CI
`v3-gate.yml` `ios`.

## Out of scope
Controls and Details (IOS-06).

Depends on: TRD-GATE, IOS-04

---

### IOS-06 — Controls and Details

## Goal
Every overview §6.9 control is reachable on iOS and Details screens show everything overview §6.3
lists.

## Context
§5.3; overview §6.3, §6.9; spec 2 §6.4 (the buttons per status and the Stop text); `Commands`,
`DetailsModel` (IOS-03).

## Files
- `v3/3-trm-ios.md` (this spec; §5.3)
- `v3/ios/TRMKit/Sources/TRMUI/Details/DetailsView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Activity/RowActions.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Common/Confirmations.swift` (new)

## Approach
Swipe actions, context menus, confirmations with "Don't ask again" (stored in `UserDefaults`),
alerts for failures, the Details screens and Copy buttons; the global pause button in the toolbar.

## Acceptance criteria
- A1: the action set per status equals spec 2 §6.4 and each action calls the right RPC — check: `RowActionsTests` (fake records calls).
- A2: Details shows the command line and log tail from `GetRenderDetails` — check: `DetailsViewTests` (offscreen render plus model assertions).

## Tests
The two test classes.

## CI
`v3-gate.yml` `ios`.

## Out of scope
Config and Log (IOS-07).

Depends on: TRD-GATE, IOS-05

---

### IOS-07 — Config editor, folder browser and Log

## Goal
The whole config and codec table can be edited on iOS, with a server folder browser, and the log
can be read live.

## Context
§5.4; overview §6.4, §6.5; spec 1 §6.1–§6.2 (validation rules); `ConfigEditorModel`,
`DirectoryBrowserModel`, `LogModel` (IOS-03).

## Files
- `v3/3-trm-ios.md` (this spec; §5.4)
- `v3/ios/TRMKit/Sources/TRMUI/Config/ConfigView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Config/PathListSection.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Config/VariantEditor.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Config/CodecEditor.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Config/FolderBrowserView.swift` (new)
- `v3/ios/TRMKit/Sources/TRMUI/Log/LogView.swift` (new)

## Approach
The `Form` sections, sheets and pushed browsers of §5.4; the Log view with the scroll rules.

## Acceptance criteria
- A1: an invalid regex shows the server's issue and disables Save; a valid edit saves with the loaded revision — check: `ConfigViewTests` (model-driven).
- A2: choosing a folder in the browser adds it to the list — check: `FolderBrowserTests`.
- A3: the Log level picker filters lines and new lines appear at the top — check: `LogViewTests`.

## Tests
The three test classes.

## CI
`v3-gate.yml` `ios`.

## Out of scope
Release (IOS-09).

Depends on: TRD-GATE, IOS-05

---

### IOS-08 — TRMKit contract tests against a real trd

## Goal
`TRMCore` is proven against a real `trd` from the same commit: every RPC it uses, every control,
config editing, pinned-TLS pairing, events, thumbnails and the folder browser.

## Context
§4.6; spec 2 §5.7 (the same scenarios in Kotlin); spec 1 §5 (`TRD_LISTENING`), §12 (sandbox),
TRD-05 (fake tools, `rules`, `wait_for_file`); the gate's `contract` job (spec 1 §9.1) runs the
script on macOS once it exists.

## Files
- `v3/3-trm-ios.md` (this spec; §4.6)
- `v3/ios/scripts/contract-test.sh` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/TrdProcess.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/ConnectionContractTests.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/ControlsContractTests.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/ConfigContractTests.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/PairingContractTests.swift` (new)
- `v3/ios/TRMKit/Tests/TRMCoreTests/Contract/EventsContractTests.swift` (new)

## Approach
1. `TrdProcess` and the scenarios of §4.6, each test starting its own daemon on its own temporary
   tree, polling snapshots with deadlines of at most 120 s as hang guards.
2. `contract-test.sh` per §4.6; add the `contract` target to the Makefile.
3. Run it on the owner's Mac and in the gate.

## Acceptance criteria
- A1: every §4.6 scenario passes against a `trd` built from the same commit — check: `bash v3/ios/scripts/contract-test.sh`.
- A2: without `TRM_E2E=1` the contract tests are skipped and `swift test` is otherwise unchanged — check: `make -C v3/ios test`.
- A3: the gate's `contract` job (macOS, iOS entry) is green — check: `v3-gate.yml`.

## Tests
The contract test classes listed in Files.

## CI
`v3-gate.yml` `contract`.

## Out of scope
SwiftUI views (covered by their offscreen tests).

Depends on: TRD-GATE, IOS-03, IOS-04

---

### IOS-09 — TestFlight release, simulator system tests and coverage

## Goal
A `v3.*` tag uploads a signed build to TestFlight; the app-hosted tests run on a simulator in CI;
the coverage and suite floors are raised.

## Context
§7; spec 1 §9.5, §9.7; overview §15.3 (coverage floors); ReelVault
`.github/workflows/ios-testflight.yml`; overview §8 (owner prerequisites: bundle id, App Store
Connect record, secrets); §11 (test plan).

## Files
- `v3/3-trm-ios.md` (this spec; §7, §11)
- `.github/workflows/v3-ios-build.yml` (new)
- `.github/workflows/v3-release.yml`
- `.github/workflows/v3-system-tests.yml`
- `v3/ios/coverage-floors.json`
- `.mason/mason.toml`

## Approach
1. `v3-ios-build.yml` per §7; add the `ios` job to `v3-release.yml` and `ios` to its
   `system-tests.needs` and `publish.needs`; add `ios-app-tests` to `v3-system-tests.yml`.
2. A `workflow_dispatch` of `v3-release.yml` must pass the iOS job (unsigned path), and one of
   `v3-system-tests.yml` must pass `ios-app-tests`.
3. Raise `coverage-floors.json` to 80 (`TRMCore`) and 40 (`TRMUI`), adding tests for whatever is
   uncovered until `ci.sh` passes.
4. Raise `ios-kit-test`'s `min_tests` to 90 % of the measured count.
5. Walk the §11 test plan and add any test it names that does not exist yet, until
   `python3 -I v3/scripts/check-test-names.py --spec 3` passes. From now on the gate's `t0` job
   runs it for spec 3 on every batch (`--completed` recognises `v3-ios-build.yml`).

## Acceptance criteria
- A1: the dispatch run's iOS job is green — check: the run.
- A2: on a tag with the secrets, the signed archive is exported and the upload step (`xcodebuild -exportArchive` with `destination` `upload`) succeeds; that the build then installs from TestFlight is the owner's manual check M-IOS-1, outside this ticket — check: the tag's `v3-release.yml` run.
- A3: `ios-app-tests` is green: `LaunchArgumentsTests`, `KeychainTests` and `BonjourDiscoveryTests` pass on a simulator — check: the `v3-system-tests.yml` dispatch.
- A4: the `ios` gate job passes with coverage floors of 80 % (`TRMCore`) and 40 % (`TRMUI`) — check: the gate run.
- A5: the `ios-kit-test` floor is raised — check: `.mason/mason.toml`.
- A6: every test that spec 3's acceptance criteria and §11 table name exists under that name — check: `python3 -I v3/scripts/check-test-names.py --spec 3`.

## Tests
The workflow runs; the app-hosted tests; whatever step 3 adds.

## CI
`v3-ios-build.yml` (new), `v3-release.yml`, `v3-system-tests.yml`.

## Out of scope
App Store review submission (the owner promotes builds manually). The manual checks of §11.1.

Depends on: TRD-GATE, IOS-06, IOS-07, IOS-08

## 9. Dependency summary

| Ticket | Depends on |
|---|---|
| IOS-01 | TRD-GATE |
| IOS-02 | TRD-GATE, IOS-01 |
| IOS-03 | TRD-GATE, IOS-02 |
| IOS-04 | TRD-GATE, IOS-03 |
| IOS-05 | TRD-GATE, IOS-04 |
| IOS-06 | TRD-GATE, IOS-05 |
| IOS-07 | TRD-GATE, IOS-05 |
| IOS-08 | TRD-GATE, IOS-03, IOS-04 |
| IOS-09 | TRD-GATE, IOS-06, IOS-07, IOS-08 |

## 10. Definition of done

- Every ticket's acceptance criteria hold; the `ios` gate job is green at the overview §15.3
  coverage floors; the gate's `contract` job is green for iOS; `v3-system-tests.yml`
  `ios-app-tests` is green; a `v3-release.yml` dispatch passes the iOS job; a tagged build reaches
  TestFlight.
- Before a release is published, the owner has run the §11.1 manual checks and recorded the
  results on the IOS-09 issue (overview §15.4).

## 11. Test plan

| Area | Automated tests | Kind | Ticket | Runs in |
|---|---|---|---|---|
| project, package, generated API code, coverage and system-test scripts | `GeneratedCodeTests`, `check-makefiles.sh`, `coverage.sh` floor check, the `system-test.sh` refusal check in `ci.sh` | unit, static | IOS-01 | mason `ios-kit-test`, `ios-build`; gate `ios`, `t0` |
| launch arguments in the app | `LaunchArgumentsTests` | app-hosted | IOS-01 | system `ios-app-tests` |
| pinned TLS, client, servers, Keychain protocol | `PinnedCertificateTests`, `TrdClientTests`, `ServerStoreTests` | unit, integration | IOS-02 | mason `ios-kit-test`; gate `ios` |
| the real Keychain | `KeychainTests` | app-hosted | IOS-02 | system `ios-app-tests` |
| presenters, scroll rules, formatting, models | `ScrollKeeperTests`, `ActivityModelTests`, `FormatTests` (shared vectors with Kotlin), `ConfigEditorModelTests`, … | unit | IOS-03 | mason `ios-kit-test`; gate `ios` |
| discovery parsing, pairing, Servers screen, app shell | `DiscoveryTests`, `PairingClientTests`, `ServersViewTests`, `AppShellTests` | unit, UI | IOS-04 | same |
| real Bonjour on iOS | `BonjourDiscoveryTests` | app-hosted | IOS-04 | system `ios-app-tests` |
| Activity screen and its scrolling | `ActivityScrollTests`, `ActivityViewTests` | UI | IOS-05 | mason `ios-kit-test`; gate `ios` |
| controls and Details | `RowActionsTests`, `DetailsViewTests` | UI | IOS-06 | same |
| Config editor, folder browser, Log | `ConfigViewTests`, `FolderBrowserTests`, `LogViewTests` | UI | IOS-07 | same |
| `TRMCore` against a real trd | the contract tests of §4.6 | contract | IOS-08 | gate `contract` (macOS); `make contract` locally |
| release pipeline | dispatch runs; the upload step on a tag | workflow | IOS-09 | `v3-release.yml`; manual M-IOS-1 |
| coverage | floors 80 % (`TRMCore`), 40 % (`TRMUI`) | coverage | IOS-01, IOS-09 | gate `ios` |
| every named test exists | `check-test-names.py --spec 3` | static | IOS-09 | gate `t0` on every batch once IOS-09 has landed |

### 11.1 Manual checks (the owner, on an iPhone and an iPad)

| ID | Steps | Expected result |
|---|---|---|
| M-IOS-1 | Install the build from TestFlight and open it on the home Wi-Fi. | iOS asks once for local network access; after allowing, the Servers screen lists the Mac's `trd`. |
| M-IOS-2 | Pair with a code from the desktop app's **Pair a phone…**. | The sixth digit pairs; the app switches to Activity showing the same sequences and videos as the desktop. |
| M-IOS-3 | Watch a render to completion. | Frame counts and progress advance; when the sequence finishes its videos appear at the top of Recently rendered with posters; the list stays at the top while it is at the top, and shows `↑ N new` when scrolled. |
| M-IOS-4 | Use suspend, resume, stop and retry on a sandbox render (`make -C v3/trd develop`). | Each takes effect within seconds and the desktop app shows the same state. |
| M-IOS-5 | Edit a harmless config value with the folder browser, save, then revert. | `trd` reloads it; the desktop app's Config window shows the change. |
| M-IOS-6 | Send the app to the background for a minute and bring it back. | It reconnects and catches up without user action. |
| M-IOS-7 | On the iPad, rotate and use the split view. | List and Details sit side by side; selection and scroll position survive rotation. |
| M-IOS-8 | Revoke the phone in the desktop app's Devices. | The phone shows that it must be paired again. |
