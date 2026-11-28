#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Usage: check-system-tests.sh skipped|ran
# Checks the JmdnsLiveTest report: `skipped` (ordinary runs) needs every case skipped and no other
# test reaching jmdns; `ran` (the kotlin-mdns system job) needs every case run and passing.
set -uo pipefail
cd "$(dirname "$0")/.."

mode="${1:-}"
case "$mode" in skipped | ran) ;; *) echo "usage: $0 skipped|ran" >&2; exit 2 ;; esac

report=core/build/test-results/test/TEST-com.timelapserender.monitor.core.discovery.JmdnsLiveTest.xml
problems=""
add() { problems="${problems}${1}"$'\n'; }

attr() { # attr <name>: value from the <testsuite ...> line
  grep -m1 '<testsuite ' "$report" | sed -n "s/.* $1=\"\([0-9]*\)\".*/\1/p"
}

if [ ! -f "$report" ]; then
  add "missing report $report"
else
  tests=$(attr tests); skipped=$(attr skipped); failures=$(attr failures); errors=$(attr errors)
  tests=${tests:-0}; skipped=${skipped:-0}; failures=${failures:-0}; errors=${errors:-0}
  if [ "$tests" -lt 1 ]; then add "JmdnsLiveTest reports no test cases"; fi
  if [ "$mode" = skipped ]; then
    if [ "$skipped" -ne "$tests" ]; then add "JmdnsLiveTest ran cases without the system-test switches: tests=$tests skipped=$skipped"; fi
  else
    if [ "$skipped" -ne 0 ]; then add "JmdnsLiveTest skipped $skipped of $tests cases"; fi
    if [ "$failures" -ne 0 ]; then add "JmdnsLiveTest has $failures failures"; fi
    if [ "$errors" -ne 0 ]; then add "JmdnsLiveTest has $errors errors"; fi
  fi
fi

if [ "$mode" = skipped ]; then
  for d in core/src/test desktop/src/test android/src/test android/src/androidTest; do
    [ -d "$d" ] || continue
    hits=$(grep -rlE 'JmDNS|JmdnsDiscovery' "$d" 2>/dev/null | grep -v '/JmdnsLiveTest\.kt$' || true)
    if [ -n "$hits" ]; then
      while IFS= read -r f; do add "$f reaches jmdns; only JmdnsLiveTest may"; done <<< "$hits"
    fi
  done
fi

if [ -n "$problems" ]; then
  printf '%s' "$problems"
  exit 1
fi
echo SYSTEM-TESTS-OK
