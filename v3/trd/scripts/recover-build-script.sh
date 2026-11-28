#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Runs a cargo build command. If it fails with "failed to run custom build command" and "cannot
# execute binary file" (a bad build-script binary, see docs/decisions/1307), prints diagnostics on
# the bad file, runs `cargo clean -p <name>@<version>` for the named packages and retries once.
# Any other failure keeps its exit status with no clean and no retry.
# Usage: recover-build-script.sh <command...>
set -uo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: recover-build-script.sh <command...>" >&2
  exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/trd-recover-build-script.XXXXXX")"
trap 'rm -rf "$work"' EXIT
log="$work/log"

CARGO_TERM_COLOR=never "$@" 2>&1 | tee "$log"
rc="${PIPESTATUS[0]}"
if [ "$rc" -eq 0 ]; then
  exit 0
fi

if ! grep -q 'failed to run custom build command for' "$log" ||
  ! grep -q 'cannot execute binary file' "$log"; then
  exit "$rc"
fi

pkgs="$(sed -n 's/.*failed to run custom build command for `\([^ `]*\) v\([^ `]*\)[ `].*/\1@\2/p' "$log" | sort -u)"
if [ -z "$pkgs" ]; then
  exit "$rc"
fi

grep -o "process didn't exit successfully: \`[^\`]*\`" "$log" | sed 's/^[^`]*`//; s/`$//' | sort -u |
  while IFS= read -r f; do
    if [ -f "$f" ]; then
      echo "--- diagnostics for $f"
      ls -l "$f" || true
      file "$f" || true
      shasum -a 256 "$f" || true
      od -A x -t x1 -N 64 "$f" || true
    fi
  done

echo "::warning::build script could not be executed; cleaning and retrying once: $(echo $pkgs)"
for p in $pkgs; do
  if ! cargo clean -p "$p"; then
    exit "$rc"
  fi
done

CARGO_TERM_COLOR=never "$@"
exit $?
