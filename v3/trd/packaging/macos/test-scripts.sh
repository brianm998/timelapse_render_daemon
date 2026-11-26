#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests the installer scripts preinstall and postinstall (spec 1 §10.1, TRD-24 A4). Stub stat, id,
# launchctl, chown and chmod go first on PATH; each stub appends its arguments to a log. The
# plist path comes from TRD_PKG_PLIST, a file in a temporary directory, so /Library is never touched.
# Prints `PASS <name>` or `FAIL <name>` per check and exits 1 when any check fails.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-pkg-scripts.XXXXXX")"
trap 'rm -rf "$work"' EXIT

stubs="$work/bin"
mkdir -p "$stubs"
plist="$work/com.timelapserender.trd.plist"
echo "<plist/>" > "$plist"

# Every stub logs its call. The stat, id and launchctl stubs read what to report from the environment.
for tool in chown chmod; do
  cat > "$stubs/$tool" <<'EOF'
#!/bin/bash
echo "$(basename "$0") $*" >> "$STUB_LOG"
exit 0
EOF
done
cat > "$stubs/stat" <<'EOF'
#!/bin/bash
echo "stat $*" >> "$STUB_LOG"
printf '%s\n' "${STUB_CONSOLE_USER-}"
exit 0
EOF
cat > "$stubs/id" <<'EOF'
#!/bin/bash
echo "id $*" >> "$STUB_LOG"
if [ -n "${STUB_UID-}" ]; then
  printf '%s\n' "$STUB_UID"
  exit 0
fi
exit 1
EOF
cat > "$stubs/launchctl" <<'EOF'
#!/bin/bash
echo "launchctl $*" >> "$STUB_LOG"
exit "${STUB_LAUNCHCTL_STATUS-0}"
EOF
chmod 755 "$stubs"/*

pass=0
fail=0

# report <name> <status>: prints PASS or FAIL for a check whose status is 0 when it holds.
report() {
  if [ "$2" -eq 0 ]; then
    echo "PASS $1"
    pass=$((pass + 1))
  else
    echo "FAIL $1"
    fail=$((fail + 1))
  fi
}

# run_script <script> <case> <console user> <uid> <launchctl status>: runs one install script with
# the stubs, logging to $work/<case>.<script>.log. Sets RC (its exit status) and LOG (the file).
run_script() {
  LOG="$work/$2.$1.log"
  : > "$LOG"
  STUB_LOG="$LOG" STUB_CONSOLE_USER="$3" STUB_UID="$4" STUB_LAUNCHCTL_STATUS="$5" \
    TRD_PKG_PLIST="$plist" PATH="$stubs:$PATH" bash "$here/$1" >/dev/null 2>&1
  RC=$?
}

# launchctl_lines <log>: the launchctl calls of a log, one per line.
launchctl_lines() {
  grep '^launchctl ' "$1"
}

# Cases with no console user: nothing is called except the lookup of the console owner.
for user in "" root loginwindow; do
  for script in preinstall postinstall; do
    run_script "$script" "nouser-${user:-empty}" "$user" "" 0
    report "$script exits 0 with console user '${user:-empty}'" "$RC"
    # Only the console-owner lookup may be logged: any other call makes this check fail.
    if grep -qv '^stat ' "$LOG"; then calls_made=1; else calls_made=0; fi
    report "$script calls nothing with console user '${user:-empty}'" "$calls_made"
  done
done

# Console user alice, uid 501: preinstall boots the agent out, postinstall bootstraps it in.
run_script preinstall alice alice 501 0
report "preinstall exits 0 for alice" "$RC"
expected="launchctl bootout gui/501/com.timelapserender.trd"
[ "$(launchctl_lines "$LOG")" = "$expected" ]
report "preinstall issues exactly the bootout for alice" "$?"

run_script postinstall alice alice 501 0
report "postinstall exits 0 for alice" "$RC"
expected="launchctl bootstrap gui/501 $plist"
[ "$(launchctl_lines "$LOG")" = "$expected" ]
report "postinstall issues exactly the bootstrap for alice" "$?"
grep -qx "chown root:wheel $plist" "$LOG"
report "postinstall makes the plist root:wheel" "$?"
grep -qx "chmod 644 $plist" "$LOG"
report "postinstall makes the plist mode 644" "$?"

# A failing launchctl still lets both scripts exit 0.
run_script preinstall fail-pre alice 501 1
report "preinstall exits 0 when launchctl fails" "$RC"
run_script postinstall fail-post alice 501 1
report "postinstall exits 0 when launchctl fails" "$RC"

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
