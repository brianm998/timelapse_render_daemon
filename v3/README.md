# Timelapse Render Daemon v3

The third implementation of the timelapse render daemon (after the Perl script and the Swift
rewrite in `rewrite/`): a Rust daemon, `trd`, that starts at login and renders LRTimelapse exports
with a bundled ffmpeg and exiftool, plus monitor clients for desktop (Kotlin Compose), Android
(Kotlin Compose) and iOS/iPadOS (SwiftUI) that watch and control it over gRPC.

Licence: GPL-3.0-or-later (`LICENSE`). Every source file carries an SPDX header.

## Reading order

1. `0-overview.md`: decisions, architecture, shared rules, the Makefile contract, ticket format.
2. `1-trd-core-daemon.md`: `trd`, ffmpeg and exiftool, GitHub Actions, the mason bootstrap.
3. `2-trm-desktop.md`: the desktop monitor (and the Kotlin code Android shares).
4. `3-trm-ios.md`: the iOS monitor.
5. `4-trm-android.md`: the Android monitor.

`instructions` is the owner's original brief.

## Layout

`scripts/` (suite runner, checks, version bump), `proto/`, `trd/`, `ffmpeg/`, `exiftool/`,
`assets/`, `clients/kotlin/`, `ios/`. Components appear as their tickets land.

## make

Run from `v3/`; each target delegates to every component directory that exists, in the order
ffmpeg, exiftool, trd, desktop, android, ios.

| Command | Meaning |
|---|---|
| `make run` / `make develop` | `trd` against the real setup / in an isolated sandbox (ports 50081/50082) |
| `make release` | the packages CI builds for this host |
| `make clean` | delete every build output |
| `make test` | every automated test (what mason runs) |
| `make check` | formatting and lint |
| `make ci` | what the component's job in `v3-gate.yml` runs |
| `make <component>-<target>` | one target of one component, e.g. `make desktop-develop` |

`make -n <target>` is a dry run and works on every OS. Bootstrap checks:
`bash scripts/test-scripts.sh`, `bash scripts/check-spdx.sh`, `bash scripts/check-workflows.sh`,
`bash scripts/check-makefiles.sh`, `python3 -I scripts/check-test-names.py --completed`.
Version bump: `bash scripts/bump-version.sh X.Y.Z[-pre]`.
