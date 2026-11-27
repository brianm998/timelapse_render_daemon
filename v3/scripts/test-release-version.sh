#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests release-version.sh (spec 1 §9.5, TRD-27 A3): a v3.* tag must match v3/VERSION, a matching
# tag is a release, and a -suffix marks a pre-release. Fixtures are version files in a temporary
# directory under the system temporary directory (overview §10 rule 4). Prints `PASS <name>` or
# `FAIL <name>` per case and exits 1 when any case fails.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/release-version.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-release-version.XXXXXX")"
trap 'rm -rf "$work"' EXIT

failures=0
passes=0
counter=0
pass() { echo "PASS $1"; passes=$((passes + 1)); }
fail() {
  echo "FAIL $1"
  [ -z "${2:-}" ] || printf '     %s\n' "$2" >&2
  failures=$((failures + 1))
}

# check <name> <ref> <version file content (printf %b)> <want exit> [<want stdout>]
check() {
  local name="$1" ref="$2" content="$3" want_rc="$4" want_out="${5:-}" file out rc
  counter=$((counter + 1))
  file="$work/VERSION.$counter"
  printf '%b' "$content" > "$file"
  out="$(bash "$script" "$ref" "$file" 2>"$work/stderr")"; rc=$?
  if [ "$rc" -ne "$want_rc" ]; then
    fail "$name" "exit $rc, want $want_rc; stderr: $(cat "$work/stderr")"
  elif [ "$want_rc" -eq 0 ] && [ "$out" != "$want_out" ]; then
    fail "$name" "stdout [$out], want [$want_out]"
  else
    pass "$name"
  fi
}

# Tag pushes.
check "v3.0.0 with 3.0.0 is a release" refs/tags/v3.0.0 '3.0.0\n' 0 \
  "$(printf 'version=3.0.0\nis_release=true\nprerelease=false')"
check "v3.0.0-alpha.1 with 3.0.0-alpha.1 is a release marked pre-release" refs/tags/v3.0.0-alpha.1 '3.0.0-alpha.1\n' 0 \
  "$(printf 'version=3.0.0-alpha.1\nis_release=true\nprerelease=true')"
check "v3.0.1 with 3.0.0 is rejected" refs/tags/v3.0.1 '3.0.0\n' 1
check "v3.0.0 with 3.0.0-alpha.1 is rejected" refs/tags/v3.0.0 '3.0.0-alpha.1\n' 1
check "v3.0 with 3.0.0 is rejected" refs/tags/v3.0 '3.0.0\n' 1

# Other refs, including a manual dispatch.
check "refs/heads/develop is not a release" refs/heads/develop '3.0.0\n' 0 \
  "$(printf 'version=3.0.0\nis_release=false\nprerelease=false')"
check "a dispatch with 3.0.0-beta.1 is a pre-release, not a release" refs/heads/main '3.0.0-beta.1\n' 0 \
  "$(printf 'version=3.0.0-beta.1\nis_release=false\nprerelease=true')"

# The VERSION file itself.
check "a CRLF VERSION file is trimmed" refs/heads/develop '3.0.0\r\n' 0 \
  "$(printf 'version=3.0.0\nis_release=false\nprerelease=false')"
check "a VERSION file without a final newline is read" refs/tags/v3.0.0 '3.0.0' 0 \
  "$(printf 'version=3.0.0\nis_release=true\nprerelease=false')"
check "a malformed VERSION file exits 1" refs/heads/develop 'three\n' 1
check "a two-part VERSION file exits 1" refs/heads/develop '3.0\n' 1
check "an empty VERSION file exits 1" refs/heads/develop '' 1

# Arguments.
bash "$script" >/dev/null 2>&1; rc=$?
if [ "$rc" -eq 2 ]; then pass "no arguments exits 2"; else fail "no arguments exits 2" "exit $rc"; fi
bash "$script" refs/heads/develop >/dev/null 2>&1; rc=$?
if [ "$rc" -eq 2 ]; then pass "a missing VERSION file argument exits 2"; else fail "a missing VERSION file argument exits 2" "exit $rc"; fi
bash "$script" refs/heads/develop "$work/no-such-file" >/dev/null 2>&1; rc=$?
if [ "$rc" -eq 1 ]; then pass "a VERSION file that does not exist exits 1"; else fail "a VERSION file that does not exist exits 1" "exit $rc"; fi

echo "# $passes passed, $failures failed" >&2
[ "$failures" -eq 0 ]
