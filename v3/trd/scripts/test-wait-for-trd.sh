#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests wait-for-trd.sh (spec 1 §9.7, acceptance A3) against a stub trd. The stub binds no port and
# records each call in a log, so no real trd, daemon or port is involved. The stub is put first on
# PATH for the default-trd case. Prints `PASS <name>` or `FAIL <name>` per case and exits 1 when any
# case fails.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
waiter="$here/wait-for-trd.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-wait-for-trd-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

stub_addr="stub-address:0"
failures=0

# The stub: appends its arguments to $STUB_DIR/calls, then exits 1 while the number of calls so far
# is at most $STUB_FAILS, and 0 after that. Nothing here listens on a port.
mkdir -p "$work/bin"
cat > "$work/bin/trd" <<'STUB'
#!/usr/bin/env bash
# Stub trd for test-wait-for-trd.sh.
echo "$*" >> "$STUB_DIR/calls"
calls="$(wc -l < "$STUB_DIR/calls" | tr -d ' ')"
if [ "$calls" -le "${STUB_FAILS:-0}" ]; then
  exit 1
fi
exit 0
STUB
chmod +x "$work/bin/trd"

# new_case <name>: a fresh directory for a case's call log.
new_case() {
  STUB_DIR="$work/$1"
  mkdir -p "$STUB_DIR"
  export STUB_DIR
}

calls_in() {
  if [ -f "$STUB_DIR/calls" ]; then
    wc -l < "$STUB_DIR/calls" | tr -d ' '
  else
    echo 0
  fi
}

pass() { echo "PASS $1"; }

fail() {
  echo "FAIL $1: $2"
  failures=$((failures + 1))
}

# case_answers_after_failures: the stub fails twice, then answers; the waiter must exit 0 on the
# third call, and each call must be `status --plain --address <a>`.
case_answers_after_failures() {
  new_case answers
  STUB_FAILS=2 bash "$waiter" --trd "$work/bin/trd" --address "$stub_addr" --timeout 30 >/dev/null 2>&1
  local rc=$?
  local n
  n="$(calls_in)"
  if [ "$rc" -ne 0 ]; then
    fail answers_after_failures "exit $rc, expected 0"
  elif [ "$n" -ne 3 ]; then
    fail answers_after_failures "trd was called $n times, expected 3"
  elif [ "$(sort -u "$STUB_DIR/calls")" != "status --plain --address $stub_addr" ]; then
    fail answers_after_failures "trd was called with: $(sort -u "$STUB_DIR/calls")"
  else
    pass answers_after_failures
  fi
}

# case_never_answers: the stub always fails; with a 1 s timeout the waiter must exit 1 and say why.
case_never_answers() {
  new_case never
  STUB_FAILS=1000000 bash "$waiter" --trd "$work/bin/trd" --address "$stub_addr" --timeout 1 \
    >/dev/null 2>"$work/never.err"
  local rc=$?
  if [ "$rc" -ne 1 ]; then
    fail never_answers "exit $rc, expected 1"
  elif ! grep -q "did not answer" "$work/never.err"; then
    fail never_answers "no timeout message on stderr: $(cat "$work/never.err")"
  else
    pass never_answers
  fi
}

# case_default_trd_on_path: with no --trd, the stub first on PATH is the one that runs.
case_default_trd_on_path() {
  new_case default_path
  PATH="$work/bin:$PATH" STUB_FAILS=0 bash "$waiter" --address "$stub_addr" --timeout 5 \
    >/dev/null 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    fail default_trd_on_path "exit $rc, expected 0"
  elif [ "$(calls_in)" -ne 1 ]; then
    fail default_trd_on_path "the stub on PATH was not called"
  else
    pass default_trd_on_path
  fi
}

# case_usage: a bad --timeout and an unknown argument both exit 2.
case_usage() {
  new_case usage
  local rc
  bash "$waiter" --trd "$work/bin/trd" --timeout soon >/dev/null 2>&1
  rc=$?
  if [ "$rc" -ne 2 ]; then
    fail usage "--timeout soon exited $rc, expected 2"
    return
  fi
  bash "$waiter" --bogus >/dev/null 2>&1
  rc=$?
  if [ "$rc" -ne 2 ]; then
    fail usage "--bogus exited $rc, expected 2"
    return
  fi
  if [ "$(calls_in)" -ne 0 ]; then
    fail usage "trd was called on a usage error"
    return
  fi
  pass usage
}

case_answers_after_failures
case_never_answers
case_default_trd_on_path
case_usage

if [ "$failures" -ne 0 ]; then
  exit 1
fi
