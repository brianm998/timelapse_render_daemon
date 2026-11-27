# 1203: TRD-GATE acceptance, media job and API v1 freeze

Decisions taken while closing spec 1 (TRD-GATE) that the spec leaves open.

- **Sequence C renders; it does not fail.** Real ffmpeg decodes the sandbox's truncated frame 12 and
  still renders all 24 frames, so no `FAILURE_SOURCE_FRAME_UNREADABLE` card is raised. The owner
  accepted this. `tests/e2e.rs` asserts that C has three renders of 24 frames, no failure card, and a
  decode error in the log tail of each render (`GetRenderDetails`). The fake-tool sandbox test
  (`tests/sandbox.rs`) still injects a failure for C, because it uses a fake ffmpeg on purpose.
- **Suite floors are 90 % of counts measured on the Mac, rounded down.** `trd-test` runs the whole
  workspace: 556 tests, floor 500. `trd-check` has four `ok` lines (fmt, clippy and two system-test
  rule checks), floor 3. `trd-e2e` is `ci-media.sh`: 2 e2e tests and 14 XMP tests, floor 14. The
  counts come from the built test binaries listed with `--list`, minus the ignored tests. The tests
  themselves were not run to produce them.
- **Coverage floors are `trd-core = 80` and `trd = 60`, as the spec says.** They were not measured
  here: `cargo llvm-cov` runs the tests, and the tests run on the pipeline. The gate's `trd-coverage`
  job is the check. If it fails, the missing lines need tests before the floors can stay.
- **The e2e test runs only under `TRD_E2E=1`.** Without the variable each test returns at once. With it,
  a missing ffmpeg, ffprobe or exiftool fails the test. `ci-media.sh` sets the variable itself when
  the tools resolve: `TRD_FFMPEG_DIR`/`TRD_EXIFTOOL_DIR`, then PATH, then MacPorts' `/opt/local/bin`,
  then a native ffmpeg built in `v3/ffmpeg` and an exiftool fetched into `v3/exiftool/dist/unix`.
- **Outside CI, a machine without the tools skips the real-tool path (owner's decision).** A test node
  such as marmot has no ffmpeg and no meson to build one. When `GITHUB_ACTIONS` is unset and ffmpeg
  or exiftool neither resolves nor can be built or fetched, `ci-media.sh` prints a `SKIPPED` notice
  and runs the e2e and XMP tests without `TRD_E2E`, so they return early and still report. In CI the
  same situation is an error, so the gate's `trd-media` job always runs the real tools.
- **`trd-media` runs on Linux and macOS with the bundles of this run.** It needs the `ffmpeg` and
  `exiftool` reusable jobs, downloads `v3-ffmpeg-linux-x64` or `v3-ffmpeg-macos-universal` and
  `v3-exiftool-unix`, restores their executable bits, and sets `TRD_FFMPEG_DIR`, `TRD_EXIFTOOL_DIR`
  and `TRD_E2E=1`.
- **Foreign renders are checked with a stand-in process.** `e2e_ignores_foreign_renders` starts its own
  fake ffmpeg with a render's arguments and waits for two scans. The snapshot must not list it, and no
  daemon render may be adopted. The test signals only that child.
- **API version 1 is frozen as of 2026-11-27.** `v3/proto/README.md` says so, and says that changes
  need `api_version` 2 and a new proto package `trd.v2`. Nothing else in the proto or its README
  changes.

Stage note: `docs/stages/s1213.md`.
