#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests recover-build-script.sh against a stub cargo that logs its calls and prints canned output.
# Prints `PASS <name>` or `FAIL <name>` per case and exits 1 when any case fails.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="$here/recover-build-script.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-recover-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
failures=0

mkdir -p "$work/bin"
printf 'not an executable\n' > "$work/fake-build-script"
# Stub cargo: logs args. `clean` succeeds. Other calls fail while the call count (excluding clean)
# is at most $STUB_FAILS, printing $STUB_KIND output.
cat > "$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "$STUB_DIR/calls"
if [ "$1" = clean ]; then exit 0; fi
echo run >> "$STUB_DIR/runs"
n="$(wc -l < "$STUB_DIR/runs" | tr -d ' ')"
if [ "$n" -le "${STUB_FAILS:-0}" ]; then
  case "$STUB_KIND" in
    signature)
      echo "error: failed to run custom build command for \`trd-proto v3.0.0 (/x/trd-proto)\`"
      echo "  process didn't exit successfully: \`$STUB_FILE\` (exit status: 126)"
      echo "  $STUB_FILE: cannot execute binary file"
      exit 101 ;;
    buildfail)
      echo "error: failed to run custom build command for \`trd-proto v3.0.0 (/x/trd-proto)\`"
      echo "  build script panicked"
      exit 101 ;;
    *)
      echo "error: could not compile \`trd\`"
      exit 101 ;;
  esac
fi
exit 0
STUB
chmod +x "$work/bin/cargo"

pass() { echo "PASS $1"; }
fail() { echo "FAIL $1: $2"; failures=$((failures + 1)); }

# run_case <name> <fails> <kind>: sets rc, out; STUB_DIR holds calls and runs.
run_case() {
  STUB_DIR="$work/$1"
  mkdir -p "$STUB_DIR"
  : > "$STUB_DIR/calls"
  : > "$STUB_DIR/runs"
  export STUB_DIR STUB_FAILS="$2" STUB_KIND="$3" STUB_FILE="$work/fake-build-script"
  out="$(PATH="$work/bin:$PATH" bash "$wrapper" cargo test --no-run 2>&1)"
  rc=$?
  nruns="$(wc -l < "$STUB_DIR/runs" | tr -d ' ')"
  ncleans="$(grep -c '^clean' "$STUB_DIR/calls")"
}

run_case success 0 signature
if [ "$rc" -eq 0 ] && [ "$nruns" -eq 1 ] && [ "$ncleans" -eq 0 ]; then pass success; else fail success "rc=$rc runs=$nruns cleans=$ncleans"; fi

run_case recovers 1 signature
if [ "$rc" -ne 0 ] || [ "$nruns" -ne 2 ]; then fail recovers "rc=$rc runs=$nruns"
elif ! grep -qx 'clean -p trd-proto@3.0.0' "$STUB_DIR/calls"; then fail recovers "no clean -p trd-proto@3.0.0"
elif ! echo "$out" | grep -q 'diagnostics for'; then fail recovers "no diagnostics"
else pass recovers; fi

run_case persistent 5 signature
if [ "$rc" -ne 0 ] && [ "$nruns" -eq 2 ] && [ "$ncleans" -eq 1 ]; then pass persistent; else fail persistent "rc=$rc runs=$nruns cleans=$ncleans"; fi

run_case compile_failure 5 compile
if [ "$rc" -eq 101 ] && [ "$nruns" -eq 1 ] && [ "$ncleans" -eq 0 ]; then pass compile_failure; else fail compile_failure "rc=$rc runs=$nruns cleans=$ncleans"; fi

run_case buildscript_failure 5 buildfail
if [ "$rc" -eq 101 ] && [ "$nruns" -eq 1 ] && [ "$ncleans" -eq 0 ]; then pass buildscript_failure; else fail buildscript_failure "rc=$rc runs=$nruns cleans=$ncleans"; fi

bash "$wrapper" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq 2 ]; then pass no_arguments; else fail no_arguments "exit $rc, expected 2"; fi

[ "$failures" -eq 0 ] || exit 1
