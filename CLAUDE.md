# Timelapse Render Daemon — rules for agents

This repository holds three implementations of the timelapse render daemon:

- the Perl daemon `timelapse_render_daemon` (with `inc/`), the original;
- the Swift rewrite in `rewrite/`, a read-only reference for behaviour;
- `v3/`, the third implementation: the Rust daemon `trd`, Kotlin desktop and Android clients and a
  Swift iOS client, with the bundled ffmpeg and exiftool build recipes.

New work is v3 work. Start every branch from `origin/develop`, never `main`. An interactive
session puts its worktree in `.claude/worktrees/<branch-name>`.

`v3/0-overview.md` and the specs `v3/1-trd-core-daemon.md` … `v3/4-trm-android.md` are the plan.
They win over any other text, including tickets that disagree with them. Nothing in `v3/` changes
the Perl daemon or `rewrite/`.

## Rules for every ticket and every agent

(Verbatim from `v3/0-overview.md` §10.)

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

## Running the tests

- One mason suite: `bash v3/scripts/suite.sh <name>` (the names are in `.mason/mason.toml`; a
  suite whose component does not exist yet prints one passing placeholder case).
- One component: `make test` (and `make check`, `make ci`) in its directory, or `make test` in
  `v3/` for every component that exists. `make -n <target>` is a dry run.
- The bootstrap scripts: `bash v3/scripts/test-scripts.sh`.
- Every test the specs name exists: `python3 -I v3/scripts/check-test-names.py --ticket <ID>`.

Ports 50071 and 50072 belong to the owner's installed `trd`. Tests use port 0; the sandbox
(`make develop`) uses 50081 and 50082.

## Filing work for the pipeline

If you file a task that already contains the plan (the files to change, the
approach and the acceptance criteria), mark it explored so the pipeline skips
its exploring step: pass `explored: true` to `file_proposal`, or `--explored` to
`mason task add`, or put `explored: true` in the task's front matter. Only do
this when the task body really is the plan. Run `mason help filing` for details.
