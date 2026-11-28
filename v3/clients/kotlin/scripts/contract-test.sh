#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The Kotlin contract tests (spec 2 §5.7): the shared client classes against a real trd from this
# checkout, with the fake media tools. TRM_TRD_BIN names a built trd and TRM_FAKE_TOOLS_DIR a
# directory of fake tools; whichever is unset is built here. Safe on the owner's Mac: temporary
# directories, port 0, the LAN listener on 127.0.0.1 and no mDNS. Fails unless every contract class
# ran every one of its tests and passed. Bash 3.2 compatible.
set -euo pipefail

kotlin_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ws="$(cd "$kotlin_dir/../../trd" && pwd)"
caller="$PWD"

work="$(mktemp -d "${TMPDIR:-/tmp}/trm-contract-tools.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# A path the caller gave is relative to the caller; the tests run elsewhere.
absolute() {
  case "$1" in
    /* | ?:*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$caller/$1" ;;
  esac
}

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) suffix=".exe" ;;
  *) suffix="" ;;
esac

if [ -z "${TRM_TRD_BIN:-}" ]; then
  # cargo runs in $ws, so rust-toolchain.toml applies and a relative CARGO_TARGET_DIR is
  # relative to $ws (as in fake-tools-dir.sh).
  target_dir="${CARGO_TARGET_DIR:-$ws/target}"
  case "$target_dir" in
    /* | ?:*) ;;
    *) target_dir="$ws/$target_dir" ;;
  esac
  (cd "$ws" && cargo build -p trd --locked)
  TRM_TRD_BIN="$target_dir/debug/trd$suffix"
else
  TRM_TRD_BIN="$(absolute "$TRM_TRD_BIN")"
fi

if [ -z "${TRM_FAKE_TOOLS_DIR:-}" ]; then
  bash "$ws/scripts/fake-tools-dir.sh" "$work/tools"
  TRM_FAKE_TOOLS_DIR="$work/tools"
else
  TRM_FAKE_TOOLS_DIR="$(absolute "$TRM_FAKE_TOOLS_DIR")"
fi

export TRM_TRD_BIN TRM_FAKE_TOOLS_DIR

cd "$kotlin_dir"
reports="core/build/test-results/test"
# A report left by an earlier run must not count for this one.
rm -f "$reports"/TEST-com.timelapserender.monitor.core.contract.*.xml

TRM_E2E=1 ./gradlew --no-daemon --console=plain :core:test \
  --tests 'com.timelapserender.monitor.core.contract.*'

python3 -I - "$reports" <<'PY'
import os
import sys
import xml.etree.ElementTree as ET

reports = sys.argv[1]
problems = []
total = 0
for name in ("Connection", "Controls", "Config", "Pairing", "Events"):
    suite = "com.timelapserender.monitor.core.contract.%sContractTest" % name
    path = os.path.join(reports, "TEST-%s.xml" % suite)
    if not os.path.isfile(path):
        problems.append("missing report %s" % path)
        continue
    root = ET.parse(path).getroot()
    counts = {key: int(root.get(key, "0")) for key in ("tests", "skipped", "failures", "errors")}
    total += counts["tests"]
    if counts["tests"] < 1 or counts["skipped"] or counts["failures"] or counts["errors"]:
        problems.append("%s: %s" % (suite, counts))
if problems:
    sys.exit("contract tests did not all run and pass:\n  " + "\n  ".join(problems))
print("CONTRACT-TESTS-OK (%d tests)" % total)
PY
