#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Runs postinst and prerm with a stub systemctl and a fake systemd runtime directory
# (TRD_SYSTEMD_RUN_DIR), covering configure, remove, upgrade and the no-systemd cases. The
# scripts run under `env -i` with a PATH that holds only the stub (or nothing), so no real
# systemctl runs. Prints `PASS <name>` / `FAIL <name>` lines and exits 1 on any failure.
# Everything it creates lives under the system temporary directory (overview §10 rule 4).
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-maint-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

failures=0
passes=0
pass() { echo "PASS $1"; passes=$((passes + 1)); }
fail() { echo "FAIL $1"; failures=$((failures + 1)); [ -z "${2:-}" ] || printf '%s\n' "$2" | sed 's/^/     /' >&2; }

# Stub systemctl: appends its arguments to $STUB_CALLS and exits with $STUB_SYSTEMCTL_RC.
bin="$work/bin"
mkdir -p "$bin"
cat > "$bin/systemctl" <<'STUB'
#!/bin/sh
echo "$*" >> "$STUB_CALLS"
exit "${STUB_SYSTEMCTL_RC:-0}"
STUB
chmod 0755 "$bin/systemctl"

empty_bin="$work/empty-bin"
mkdir -p "$empty_bin"
systemd_dir="$work/run/systemd/system"
mkdir -p "$systemd_dir"
no_systemd_dir="$work/no-such-run-dir"

calls="$work/calls.log"
out="$work/out.log"

# run_script <path-dir> <runtime-dir> <stub-rc> <script> <action>
# Sets $status, $calls_made (the recorded systemctl arguments, empty if none) and $output.
run_script() {
  : > "$calls"
  env -i PATH="$1" STUB_CALLS="$calls" STUB_SYSTEMCTL_RC="$3" TRD_SYSTEMD_RUN_DIR="$2" \
    /bin/sh "$4" "$5" > "$out" 2>&1
  status=$?
  calls_made="$(cat "$calls")"
  output="$(cat "$out")"
}

postinst="$here/postinst"
prerm="$here/prerm"

# Headers: both maintainer scripts carry the two SPDX lines after the shebang.
for s in "$postinst" "$prerm"; do
  name="$(basename "$s")"
  head -n 5 "$s" | grep -q 'SPDX-License-Identifier: GPL-3.0-or-later' \
    && head -n 5 "$s" | grep -q 'Copyright (C) 2026 Timelapse Render Daemon contributors' \
    && pass "$name carries the SPDX header" || fail "$name carries the SPDX header"
  sh -n "$s" && pass "$name parses" || fail "$name parses"
done

# postinst configure with systemd: enables the unit globally and says how to start it.
run_script "$bin" "$systemd_dir" 0 "$postinst" configure
if [ "$status" -eq 0 ] && [ "$calls_made" = "--global enable trd.service" ]; then
  pass "postinst configure enables the unit globally"
else
  fail "postinst configure enables the unit globally" "status $status, calls: $calls_made"
fi
case "$output" in
  *"systemctl --user start trd"*) pass "postinst configure says how to start the unit" ;;
  *) fail "postinst configure says how to start the unit" "$output" ;;
esac

# postinst configure without systemd (no runtime directory): no systemctl call, exit 0.
run_script "$bin" "$no_systemd_dir" 0 "$postinst" configure
if [ "$status" -eq 0 ] && [ -z "$calls_made" ]; then
  pass "postinst configure without systemd calls nothing and exits 0"
else
  fail "postinst configure without systemd calls nothing and exits 0" "status $status, calls: $calls_made"
fi

# postinst configure with the runtime directory but no systemctl on PATH: no call, exit 0.
run_script "$empty_bin" "$systemd_dir" 0 "$postinst" configure
if [ "$status" -eq 0 ] && [ -z "$calls_made" ]; then
  pass "postinst configure without systemctl calls nothing and exits 0"
else
  fail "postinst configure without systemctl calls nothing and exits 0" "status $status, calls: $calls_made"
fi

# postinst configure when systemctl fails: the install still succeeds.
run_script "$bin" "$systemd_dir" 1 "$postinst" configure
if [ "$status" -eq 0 ] && [ "$calls_made" = "--global enable trd.service" ]; then
  pass "postinst configure survives a failing systemctl"
else
  fail "postinst configure survives a failing systemctl" "status $status, calls: $calls_made"
fi

# postinst for other actions (abort-upgrade, abort-install, ...): nothing runs.
run_script "$bin" "$systemd_dir" 0 "$postinst" abort-upgrade
if [ "$status" -eq 0 ] && [ -z "$calls_made" ]; then
  pass "postinst abort-upgrade calls nothing"
else
  fail "postinst abort-upgrade calls nothing" "status $status, calls: $calls_made"
fi

# prerm remove with systemd: disables the unit globally.
run_script "$bin" "$systemd_dir" 0 "$prerm" remove
if [ "$status" -eq 0 ] && [ "$calls_made" = "--global disable trd.service" ]; then
  pass "prerm remove disables the unit globally"
else
  fail "prerm remove disables the unit globally" "status $status, calls: $calls_made"
fi

# prerm remove without systemd: nothing to disable, exit 0.
run_script "$bin" "$no_systemd_dir" 0 "$prerm" remove
if [ "$status" -eq 0 ] && [ -z "$calls_made" ]; then
  pass "prerm remove without systemd calls nothing and exits 0"
else
  fail "prerm remove without systemd calls nothing and exits 0" "status $status, calls: $calls_made"
fi

# prerm remove when systemctl fails: the removal still succeeds.
run_script "$bin" "$systemd_dir" 1 "$prerm" remove
if [ "$status" -eq 0 ] && [ "$calls_made" = "--global disable trd.service" ]; then
  pass "prerm remove survives a failing systemctl"
else
  fail "prerm remove survives a failing systemctl" "status $status, calls: $calls_made"
fi

# prerm upgrade: the unit stays enabled, nothing runs.
run_script "$bin" "$systemd_dir" 0 "$prerm" upgrade
if [ "$status" -eq 0 ] && [ -z "$calls_made" ]; then
  pass "prerm upgrade calls nothing"
else
  fail "prerm upgrade calls nothing" "status $status, calls: $calls_made"
fi

echo "maintainer scripts: $passes passed, $failures failed"
[ "$failures" -eq 0 ]
