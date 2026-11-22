#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests check-system-tests.sh against the fixture trees under fixtures/system-tests/: good must
# pass, and each of r1, r2 and r3 must fail naming its rule. Each tree is copied to a temporary
# directory first, so nothing under the workspace is touched. Prints `PASS <name>` or
# `FAIL <name>` per case and exits 1 when any case fails.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fixtures="$here/fixtures/system-tests"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-system-tests-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

failures=0

# run_tree <name>: copies fixtures/system-tests/<name> to a fresh temporary directory and runs the
# checker on it. Sets OUT (its output) and RC (its exit status).
run_tree() {
  local tree="$work/$1"
  mkdir -p "$tree"
  cp -R "$fixtures/$1/." "$tree/"
  OUT="$(TRD_ROOT="$tree" bash "$here/check-system-tests.sh" 2>&1)"
  RC=$?
}

case_good() {
  run_tree good
  if [ "$RC" -eq 0 ] && [ "$OUT" = "SYSTEM-TESTS-OK" ]; then
    echo "PASS good"
  else
    echo "FAIL good (exit $RC): $OUT"
    failures=$((failures + 1))
  fi
}

# case_rule <name> <rule>: the tree must fail with exit 1 and name the rule.
case_rule() {
  run_tree "$1"
  case "$OUT" in
    *": $2 "*) rule_named=yes ;;
    *) rule_named=no ;;
  esac
  if [ "$RC" -eq 1 ] && [ "$rule_named" = yes ]; then
    echo "PASS $1"
  else
    echo "FAIL $1 (exit $RC, expected $2): $OUT"
    failures=$((failures + 1))
  fi
}

case_good
case_rule r1 R1
case_rule r2 R2
case_rule r3 R3

if [ "$failures" -ne 0 ]; then
  exit 1
fi
