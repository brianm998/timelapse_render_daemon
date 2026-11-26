#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Checks the Windows installer (spec 1 §10.3, acceptance A1): runs <setup.exe> silently into a
# temporary LOCALAPPDATA, runs the installed trd.exe version, runs the silent uninstaller, and
# checks that the install directory and the TimelapseRenderDaemon task are gone. It runs in Git
# Bash on the Windows runner of v3-trd-build.yml (spec 1 §9.4). It refuses to run unless CI=true,
# so it never touches a developer's own install (overview §10, rule 3).
#
# Usage: CI=true bash test-installer.sh <setup.exe>
# Prints PASS or FAIL for each step; exits 0 when all pass, 1 when any fails.
set -euo pipefail

if [ "${CI:-}" != true ]; then
  echo "test-installer.sh refuses to run without CI=true" >&2
  exit 1
fi

if [ $# -ne 1 ]; then
  echo "usage: CI=true bash test-installer.sh <setup.exe>" >&2
  exit 2
fi

if ! command -v cygpath >/dev/null 2>&1; then
  echo "test-installer.sh needs Git Bash on Windows (cygpath is missing)" >&2
  exit 1
fi

TASK="TimelapseRenderDaemon"

setup_dir="$(cd "$(dirname "$1")" && pwd)"
setup="$setup_dir/$(basename "$1")"
[ -f "$setup" ] || { echo "FAIL: installer not found: $1" >&2; exit 1; }

work="$(mktemp -d)"
trap 'chmod -R u+w "$work" 2>/dev/null || true; rm -rf "$work"' EXIT
localappdata="$work/localappdata"
mkdir -p "$localappdata"
install="$localappdata/Programs/TimelapseRenderDaemon"
failures=0

pass() {
  echo "PASS: $1"
}

fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

# Waits up to 60 s for the install directory to be gone and the task to be deleted. The NSIS
# uninstaller copies itself out and returns at once, so the removal happens after it returns.
# schtasks /Query fails when the task does not exist.
wait_for_removal() {
  SECONDS=0
  while :; do
    dir_gone=no
    task_gone=no
    [ -e "$install" ] || dir_gone=yes
    if ! MSYS_NO_PATHCONV=1 schtasks /Query /TN "$TASK" >/dev/null 2>&1; then
      task_gone=yes
    fi
    if [ "$dir_gone" = yes ] && [ "$task_gone" = yes ]; then
      return 0
    fi
    if [ "$SECONDS" -ge 60 ]; then
      return 1
    fi
    sleep 1
  done
}

# Step 1: the silent install. MSYS_NO_PATHCONV stops Git Bash from turning /S into a path.
setup_win="$(cygpath -w "$setup")"
localappdata_win="$(cygpath -w "$localappdata")"
rc=0
MSYS_NO_PATHCONV=1 LOCALAPPDATA="$localappdata_win" "$setup_win" /S || rc=$?
if [ "$rc" -ne 0 ]; then
  fail "silent install exited $rc"
elif [ ! -f "$install/trd.exe" ]; then
  fail "silent install did not create $install/trd.exe"
else
  pass "silent install into $localappdata_win"
fi

# Step 2: the installed trd.exe runs and prints its version line.
if [ -f "$install/trd.exe" ]; then
  rc=0
  version_out="$("$install/trd.exe" version 2>&1)" || rc=$?
  first_line="${version_out%%$'\n'*}"
  if [ "$rc" -ne 0 ]; then
    fail "trd.exe version exited $rc: $first_line"
  else
    case "$first_line" in
      "trd "*) pass "trd.exe version: $first_line" ;;
      *) fail "trd.exe version printed '$first_line', not a line starting 'trd '" ;;
    esac
  fi
fi

# Step 3: the silent uninstall, then the directory and the task are gone.
if [ -f "$install/uninstall.exe" ]; then
  rc=0
  MSYS_NO_PATHCONV=1 "$(cygpath -w "$install/uninstall.exe")" /S || rc=$?
  if [ "$rc" -ne 0 ]; then
    fail "silent uninstall exited $rc"
  else
    pass "silent uninstall started"
  fi
  if wait_for_removal; then
    pass "install directory and task $TASK are gone"
  else
    if [ -e "$install" ]; then
      fail "install directory $install still exists 60 s after the uninstall"
    else
      pass "install directory is gone"
    fi
    if MSYS_NO_PATHCONV=1 schtasks /Query /TN "$TASK" >/dev/null 2>&1; then
      fail "scheduled task $TASK still exists 60 s after the uninstall"
    else
      pass "scheduled task $TASK is gone"
    fi
  fi
else
  fail "no uninstall.exe in $install, so the uninstall was not run"
fi

if [ "$failures" -ne 0 ]; then
  echo "test-installer.sh: $failures step(s) failed" >&2
  exit 1
fi
echo "test-installer.sh: all steps passed"
