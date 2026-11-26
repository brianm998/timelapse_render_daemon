#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Usage: suite.sh <suite>
# Runs one mason suite of .mason/mason.toml (spec 1 §11.3). When the suite's component does not
# exist yet it prints one passing placeholder case in the format of the suite's parser and exits 0.
# The t0-* suites always run. V3_ROOT is the directory that plays the role of v3/ (default: the
# real one); every command runs from its parent, the repository root.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
V3_ROOT="${V3_ROOT:-$(cd "$here/.." && pwd)}"
repo="$(cd "$V3_ROOT/.." && pwd)"

if [ "$#" -ne 1 ]; then
  echo "usage: suite.sh <suite>" >&2
  exit 2
fi
suite="$1"
cd "$repo"

# placeholder <kind>: one passing case for a component that does not exist yet.
placeholder() {
  case "$1" in
    regex)
      echo "test ${suite}_not_created_yet ... ok" ;;
    smoke)
      echo "PASS ${suite}_not_created_yet" ;;
    xctest)
      echo "Test Case '-[Placeholder.Placeholder testNotCreatedYet]' passed (0.001 seconds)." ;;
    junit:*)
      results="${1#junit:}"
      mkdir -p "$(dirname "$results")"
      cat > "$results" <<XML
<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="${suite}" tests="1" failures="0" errors="0" skipped="0">
  <testcase classname="placeholder" name="${suite}_not_created_yet"/>
</testsuite>
XML
      echo "wrote $results with one placeholder case" ;;
  esac
}

# run_case <name> <command...>: runs the command, prints `test <name> ... ok|FAILED`.
failed=0
run_case() {
  local name="$1"
  shift
  local out
  if out="$("$@" 2>&1)"; then
    echo "test $name ... ok"
  else
    printf '%s\n' "$out"
    echo "test $name ... FAILED"
    failed=1
  fi
}

# need <file>: true when the suite's component exists (file relative to V3_ROOT).
need() { [ -e "$V3_ROOT/$1" ]; }

native_ffmpeg_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os:$arch" in
    Darwin:x86_64) echo macos-x64 ;;
    Darwin:arm64) echo macos-arm64 ;;
    Linux:x86_64|Linux:amd64) echo linux-x64 ;;
    Linux:aarch64|Linux:arm64) echo linux-arm64 ;;
    *) echo "suite.sh: no native ffmpeg target for $os $arch" >&2; return 1 ;;
  esac
}

case "$suite" in
  t0-diff-check)
    # A node's checkout may lack origin/develop (or be shallow): try the usual
    # base refs, then the parent commit, then the empty tree.
    base=""
    for ref in ${MASON_BASE:-} origin/develop develop; do
      if git rev-parse --verify --quiet "$ref^{commit}" >/dev/null; then
        base="$(git merge-base HEAD "$ref" 2>/dev/null || true)"
        [ -n "$base" ] && break
      fi
    done
    if [ -z "$base" ]; then
      if git rev-parse --verify --quiet "HEAD^{commit}" >/dev/null &&
         git rev-parse --verify --quiet "HEAD^1" >/dev/null; then
        echo "suite.sh: no develop base ref; checking the last commit only" >&2
        base="$(git rev-parse HEAD^1)"
      else
        echo "suite.sh: no develop base ref and no parent; checking the whole tree" >&2
        base="$(git hash-object -t tree /dev/null)"
      fi
    fi
    git diff --check "$base"
    echo T0-DIFF-OK ;;
  t0-spdx)
    bash "$V3_ROOT/scripts/check-spdx.sh"
    echo T0-SPDX-OK ;;
  t0-workflows)
    bash "$V3_ROOT/scripts/check-workflows.sh"
    echo T0-WORKFLOWS-OK ;;
  t0-scripts)
    exec bash "$V3_ROOT/scripts/test-scripts.sh" ;;
  t0-makefiles)
    bash "$V3_ROOT/scripts/check-makefiles.sh"
    echo T0-MAKEFILES-OK ;;
  t0-mason)
    mason_bin="$(command -v mason || true)"
    [ -n "$mason_bin" ] || { [ -x "$HOME/.config/mason/bin/mason" ] && mason_bin="$HOME/.config/mason/bin/mason"; }
    # A MASON node has the runtime but no wrapper on PATH: look in its data dir.
    mason_data="${MASON_DATA_DIR:-$HOME/Library/Application Support/mason}"
    if [ -z "$mason_bin" ] && [ -f "$mason_data/runtimes/current" ]; then
      sha="$(cat "$mason_data/runtimes/current")"
      [ -x "$mason_data/runtimes/$sha/mason" ] && mason_bin="$mason_data/runtimes/$sha/mason"
    fi
    if [ -z "$mason_bin" ] && [ -x "$mason_data/bootstrap/mason" ]; then
      mason_bin="$mason_data/bootstrap/mason"
    fi
    if [ -n "$mason_bin" ]; then
      case "$mason_bin" in
        "$mason_data"/*) "$mason_bin" --data-dir "$mason_data" definition check ;;
        *) "$mason_bin" definition check ;;
      esac
    else
      # No mason here (a node without the CLI): at least require valid TOML
      # with the step and suite tables; the full check runs where mason is.
      echo "suite.sh: mason is not installed; checking .mason/mason.toml syntax only" >&2
      python3 -I - "$V3_ROOT/../.mason/mason.toml" <<'PY'
import sys
try:
    import tomllib
except ImportError:
    print("suite.sh: python3 has no tomllib; skipping the syntax check", file=sys.stderr)
    sys.exit(0)
with open(sys.argv[1], "rb") as f:
    d = tomllib.load(f)
if not isinstance(d.get("suite"), dict) or not d["suite"]:
    sys.exit("suite.sh: mason.toml has no [suite.*] tables")
if not isinstance(d.get("step"), dict) or not d["step"]:
    sys.exit("suite.sh: mason.toml has no [step.*] tables")
for name, s in d["suite"].items():
    if "command" not in s:
        sys.exit("suite.sh: suite %s has no command" % name)
PY
    fi
    echo T0-MASON-OK ;;

  trd-check)
    if ! need trd/Cargo.toml; then placeholder regex; exit 0; fi
    cd "$V3_ROOT/trd"
    run_case trd_fmt cargo fmt --all --check
    run_case trd_clippy cargo clippy --workspace --all-targets --locked -- -D warnings
    run_case trd_system_test_rules_selftest bash scripts/test-check-system-tests.sh
    run_case trd_system_test_rules bash scripts/check-system-tests.sh
    exit "$failed" ;;
  trd-test)
    if ! need trd/Cargo.toml; then placeholder regex; exit 0; fi
    cd "$V3_ROOT/trd"
    exec cargo test --workspace --locked ;;
  trd-e2e)
    if ! need trd/scripts/ci-media.sh; then placeholder regex; exit 0; fi
    TRD_E2E=1 exec bash "$V3_ROOT/trd/scripts/ci-media.sh" ;;
  ffmpeg-smoke)
    if ! need ffmpeg/build.sh; then placeholder smoke; exit 0; fi
    target="$(native_ffmpeg_target)"
    bash "$V3_ROOT/ffmpeg/build.sh" "$target"
    exec bash "$V3_ROOT/ffmpeg/smoke_test.sh" --target "$target" "$V3_ROOT/ffmpeg/dist/$target" ;;
  exiftool-smoke)
    if ! need exiftool/fetch.sh; then placeholder smoke; exit 0; fi
    bash "$V3_ROOT/exiftool/fetch.sh" unix "$V3_ROOT/exiftool/dist/unix"
    exec bash "$V3_ROOT/exiftool/smoke_test.sh" "$V3_ROOT/exiftool/dist/unix" ;;
  kotlin-test)
    if ! need clients/kotlin/scripts/mason-desktop.sh; then
      placeholder "junit:$V3_ROOT/clients/kotlin/build/mason-desktop-results.xml"; exit 0
    fi
    exec bash "$V3_ROOT/clients/kotlin/scripts/mason-desktop.sh" ;;
  android-test)
    if ! need clients/kotlin/scripts/mason-android.sh; then
      placeholder "junit:$V3_ROOT/clients/kotlin/build/mason-android-results.xml"; exit 0
    fi
    exec bash "$V3_ROOT/clients/kotlin/scripts/mason-android.sh" ;;
  ios-kit-test)
    if ! need ios/TRMKit/Package.swift; then placeholder xctest; exit 0; fi
    exec swift test --package-path "$V3_ROOT/ios/TRMKit" ;;
  ios-build)
    if ! need ios/scripts/mason-build.sh; then placeholder regex; exit 0; fi
    exec bash "$V3_ROOT/ios/scripts/mason-build.sh" ;;
  *)
    echo "suite.sh: unknown suite '$suite'" >&2
    exit 2 ;;
esac
