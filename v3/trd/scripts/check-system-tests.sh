#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Enforces the system-test rules of overview §10 (rules 1 and 2) and §15.2 on the daemon's tests.
# It reads the files under crates/*/tests/ and crates/trd-testtools/src/ (TRD_ROOT, default the
# trd workspace above this script):
#   R1  every crates/*/tests/*_live.rs is a system test: it declares its tests only with
#       system_test!( and has no #[test] or #[tokio::test] line;
#   R2  no other file there uses the real process table, mDNS registration or the service-manager
#       runner (SysinfoProcessTable, MdnsSdRegistrar, ServiceDaemon, SystemRunner);
#   R3  every other file there that starts the daemon (contains "daemon") passes --no-mdns and
#       one of --sandbox and --no-adopt.
# Prints `check-system-tests: <file>: R<n> <explanation>` per violation and exits 1, or prints
# SYSTEM-TESTS-OK and exits 0. Bash 3.2 compatible.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRD_ROOT="${TRD_ROOT:-$(cd "$here/.." && pwd)}"

bad=0

violation() {
  echo "check-system-tests: $1: $2 $3"
  bad=1
}

# Test sources: every .rs file under a tests/ directory of a crate, and trd-testtools' sources.
list_files() {
  if [ -d "$TRD_ROOT/crates" ]; then
    find "$TRD_ROOT/crates" \( -path '*/tests/*' -o -path '*/trd-testtools/src/*' \) -name '*.rs' -type f \
      | sort
  fi
}

while IFS= read -r file; do
  [ -n "$file" ] || continue
  rel="${file#"$TRD_ROOT"/}"
  case "$file" in
    */tests/*_live.rs)
      # R1: a system test declares its tests only with system_test!.
      if ! grep -Fq 'system_test!(' "$file"; then
        violation "$rel" "R1" "system test file declares no system_test!( test"
      fi
      if grep -Eq '^[[:space:]]*#\[(tokio::)?test' "$file"; then
        violation "$rel" "R1" "system test file declares a #[test] outside system_test!"
      fi
      continue ;;
  esac
  # R2: ordinary test code does not reach the real process table, mDNS or service manager.
  if grep -Eq 'SysinfoProcessTable|MdnsSdRegistrar|ServiceDaemon|SystemRunner' "$file"; then
    violation "$rel" "R2" "ordinary test uses the real process table, mDNS or service-manager runner"
  fi
  # R3: ordinary test code that starts the daemon runs it without mDNS and in a sandbox.
  if grep -Fq '"daemon"' "$file"; then
    if ! grep -Fq '"--no-mdns"' "$file"; then
      violation "$rel" "R3" "starts the daemon without \"--no-mdns\""
    fi
    if ! grep -Fq '"--sandbox"' "$file" && ! grep -Fq '"--no-adopt"' "$file"; then
      violation "$rel" "R3" "starts the daemon without \"--sandbox\" or \"--no-adopt\""
    fi
  fi
done <<EOT
$(list_files)
EOT

if [ "$bad" -ne 0 ]; then
  exit 1
fi
echo "SYSTEM-TESTS-OK"
