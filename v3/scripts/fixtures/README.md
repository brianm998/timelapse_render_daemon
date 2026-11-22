<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Fixtures for `v3/scripts/test-scripts.sh`

Inputs for the tests of the bootstrap scripts. `test-scripts.sh` copies what it needs into a
temporary tree under the system temporary directory and points each script at it with `V3_ROOT`;
nothing here is ever modified.

Several fixtures are deliberately wrong (a source file without an SPDX header, a workflow with a
syntax error, a Makefile without a `develop` target, a spec that names tests that do not exist),
because the tests prove that the scripts report them. `check-spdx.sh` skips this directory for
that reason.

| File | Used for |
|---|---|
| `spdx-missing.sh` | `check-spdx.sh` must flag it |
| `spdx-one-line.sh` | has only one of the two SPDX lines; must be flagged |
| `spdx-ok.sh` | both lines; must pass |
| `spdx-generated.pb.swift` | no header, but generated; must be skipped |
| `spdx-data.json` | no header, but JSON; must be skipped |
| `workflow-valid.yml` | `check-workflows.sh` must accept it |
| `workflow-broken.yml` | syntax error; `check-workflows.sh` must reject it |
| `Makefile.complete` | every target of overview §9; `check-makefiles.sh` must accept it |
| `Makefile.no-develop` | lacks `develop`; `check-makefiles.sh` must reject it |
| `1-fixture.md` | fixture spec for `check-test-names.py`: one ticket `TRD-01` and a test plan |
