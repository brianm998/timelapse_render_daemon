#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Tests the bootstrap scripts of v3/scripts, each against a temporary copy of the files it
# touches (every script honours V3_ROOT, the directory that plays the role of v3/). Prints
# `PASS <name>` / `FAIL <name>` lines and exits 1 on any failure. Fixtures: v3/scripts/fixtures/.
# Everything it creates lives under the system temporary directory (overview §10 rule 4).
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
real_v3="$(cd "$here/.." && pwd)"
fixtures="$here/fixtures"
work="$(mktemp -d "${TMPDIR:-/tmp}/trd-scripts-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

failures=0
passes=0
pass() { echo "PASS $1"; passes=$((passes + 1)); }
fail() { echo "FAIL $1"; failures=$((failures + 1)); [ -z "${2:-}" ] || printf '%s\n' "$2" | sed 's/^/     /' >&2; }

# expect <name> <condition-result>: condition-result is 0 for success.
expect() { if [ "$2" -eq 0 ]; then pass "$1"; else fail "$1" "${3:-}"; fi; }

# contains <text> <needle>
contains() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

counter=0
# new_tree: sets T to a fresh <tmp>/<n>/v3 directory (its parent plays the repository root).
# It is not called in a subshell, so $counter and $work/$counter stay valid for the caller.
T=""
new_tree() {
  counter=$((counter + 1))
  mkdir -p "$work/$counter/v3" "$work/$counter/.mason" "$work/$counter/.github/workflows"
  T="$work/$counter/v3"
}

# ---------------------------------------------------------------- check-spdx.sh

new_tree; t="$T"
mkdir -p "$t/src"
cp "$fixtures/spdx-missing.sh" "$t/src/missing.sh"
out="$(V3_ROOT="$t" bash "$here/check-spdx.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "src/missing.sh"; then
  pass "check-spdx flags a fixture missing a header"
else
  fail "check-spdx flags a fixture missing a header" "rc=$rc $out"
fi

new_tree; t="$T"
mkdir -p "$t/src"
cp "$fixtures/spdx-one-line.sh" "$t/src/one-line.sh"
out="$(V3_ROOT="$t" bash "$here/check-spdx.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "src/one-line.sh"; then
  pass "check-spdx flags a fixture with only one SPDX line"
else
  fail "check-spdx flags a fixture with only one SPDX line" "rc=$rc $out"
fi

new_tree; t="$T"
mkdir -p "$t/src" "$t/proto/generated" "$t/other/fixtures"
cp "$fixtures/spdx-ok.sh" "$t/src/ok.sh"
cp "$fixtures/spdx-generated.pb.swift" "$t/src/Thing.pb.swift"
cp "$fixtures/spdx-generated.pb.swift" "$t/proto/generated/Anything.swift"
cp "$fixtures/spdx-data.json" "$t/src/data.json"
cp "$fixtures/spdx-missing.sh" "$t/other/fixtures/deliberately-bad.sh"
cp "$fixtures/spdx-ok.sh" "$work/$counter/.mason/ok.toml"
out="$(V3_ROOT="$t" bash "$here/check-spdx.sh" 2>&1)"; rc=$?
expect "check-spdx passes headers, a generated file, a JSON file and a fixtures directory" "$rc" "rc=$rc $out"

new_tree; t="$T"
cp "$fixtures/spdx-missing.sh" "$work/$counter/.mason/bad.toml"
cp "$fixtures/spdx-missing.sh" "$work/$counter/.github/workflows/v3-bad.yml"
out="$(V3_ROOT="$t" bash "$here/check-spdx.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" ".mason/bad.toml" && contains "$out" "v3-bad.yml"; then
  pass "check-spdx checks .mason and v3 workflows"
else
  fail "check-spdx checks .mason and v3 workflows" "rc=$rc $out"
fi

# ---------------------------------------------------------------- suite.sh

# placeholder_case <suite> <expected line fragment>: absent component prints one passing case.
placeholder_case() {
  local suite="$1" want="$2" tree out rc
  new_tree; tree="$T"
  out="$(V3_ROOT="$tree" bash "$here/suite.sh" "$suite" 2>&1)"; rc=$?
  if [ "$rc" -eq 0 ] && contains "$out" "$want"; then
    pass "suite.sh $suite prints its placeholder when the component is absent"
  else
    fail "suite.sh $suite prints its placeholder when the component is absent" "rc=$rc $out"
  fi
}
for s in trd-check trd-test trd-e2e ios-build; do
  placeholder_case "$s" "test ${s}_not_created_yet ... ok"
done
for s in ffmpeg-smoke exiftool-smoke; do
  placeholder_case "$s" "PASS ${s}_not_created_yet"
done
placeholder_case ios-kit-test "Test Case '-[Placeholder.Placeholder testNotCreatedYet]' passed (0.001 seconds)."

for pair in "kotlin-test:mason-desktop-results.xml" "android-test:mason-android-results.xml"; do
  s="${pair%%:*}"; f="${pair#*:}"
  new_tree; t="$T"
  out="$(V3_ROOT="$t" bash "$here/suite.sh" "$s" 2>&1)"; rc=$?
  results="$t/clients/kotlin/build/$f"
  if [ "$rc" -eq 0 ] && [ -f "$results" ] \
     && grep -q "<testcase classname=\"placeholder\" name=\"${s}_not_created_yet\"/>" "$results" \
     && grep -q 'tests="1" failures="0"' "$results"; then
    pass "suite.sh $s writes a junit placeholder when the component is absent"
  else
    fail "suite.sh $s writes a junit placeholder when the component is absent" "rc=$rc $out"
  fi
done

out="$(V3_ROOT="$work" bash "$here/suite.sh" no-such-suite 2>&1)"; rc=$?
expect "suite.sh rejects an unknown suite" "$([ "$rc" -eq 2 ] && echo 0 || echo 1)" "rc=$rc $out"

# t0-diff-check diffs against the merge-base of HEAD and origin/develop, then develop. Each case
# is a temporary repository whose parent plays the repository root (V3_ROOT is <repo>/v3).
gitc() { git -C "$R" -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false "$@"; }
diff_repo() { # diff_repo <n>: sets R to a fresh repository with one commit on its default branch
  R="$work/diffrepo$1"
  mkdir -p "$R/v3"
  git init -q -b trunk "$R"  # pinned: the default branch must not be develop
  echo base > "$R/v3/base.txt"
  gitc add -A
  gitc commit -q -m base
}
commit_file() { # commit_file <path> <content>: commits one file on the current branch of $R
  printf '%s\n' "$2" > "$R/$1"
  gitc add -A
  gitc commit -q -m "$1"
}

# (a) only a local develop ref: a clean change passes.
diff_repo a
gitc branch develop
commit_file v3/clean.txt clean
out="$(V3_ROOT="$R/v3" bash "$here/suite.sh" t0-diff-check 2>&1)"; rc=$?
expect "suite.sh t0-diff-check passes on a clean change when only a local develop exists" \
  "$([ "$rc" -eq 0 ] && contains "$out" T0-DIFF-OK && echo 0 || echo 1)" "rc=$rc $out"

# (b) neither origin/develop nor develop exists: the suite fails and names the missing ref.
diff_repo b
commit_file v3/clean.txt clean
out="$(V3_ROOT="$R/v3" bash "$here/suite.sh" t0-diff-check 2>&1)"; rc=$?
expect "suite.sh t0-diff-check fails without a base and names origin/develop" \
  "$([ "$rc" -eq 1 ] && ! contains "$out" T0-DIFF-OK && contains "$out" origin/develop && echo 0 || echo 1)" "rc=$rc $out"

# (c) an earlier commit after develop adds trailing whitespace while HEAD is clean: the whole
# change is checked, so the suite fails.
diff_repo c
gitc branch develop
commit_file v3/space.txt 'trailing '
commit_file v3/clean.txt clean
out="$(V3_ROOT="$R/v3" bash "$here/suite.sh" t0-diff-check 2>&1)"; rc=$?
expect "suite.sh t0-diff-check checks the whole change, not only HEAD" \
  "$([ "$rc" -ne 0 ] && ! contains "$out" T0-DIFF-OK && contains "$out" trailing && echo 0 || echo 1)" "rc=$rc $out"

# Stub components and stub tools: a suite whose component exists runs the real command.
new_tree; t="$T"
stubs="$work/stub-bin"
mkdir -p "$stubs"
calls="$work/calls.log"
cat > "$stubs/cargo" <<'EOF'
#!/usr/bin/env bash
echo "cargo $*" >> "$STUB_CALLS"
if [ -n "${STUB_FAIL:-}" ]; then
  case "$*" in *"$STUB_FAIL"*) echo "cargo: forced failure"; exit 1 ;; esac
fi
echo "test stub_cargo_case ... ok"
EOF
cat > "$stubs/swift" <<'EOF'
#!/usr/bin/env bash
echo "swift $*" >> "$STUB_CALLS"
echo "Test Case '-[Stub.Stub testStub]' passed (0.001 seconds)."
EOF
chmod +x "$stubs/cargo" "$stubs/swift"

stub_script() { # <path relative to tree> <extra line>
  mkdir -p "$(dirname "$t/$1")"
  {
    echo '#!/usr/bin/env bash'
    echo 'echo "$(basename "$0") $*" >> "$STUB_CALLS"'
    echo "$2"
  } > "$t/$1"
}

run_suite() { # <suite> [extra env...]
  local suite="$1"
  shift
  : > "$calls"
  env PATH="$stubs:$PATH" STUB_CALLS="$calls" "$@" V3_ROOT="$t" bash "$here/suite.sh" "$suite" 2>&1
}

mkdir -p "$t/trd/scripts"
touch "$t/trd/Cargo.toml"
stub_script trd/scripts/test-check-system-tests.sh 'echo selftest'
stub_script trd/scripts/check-system-tests.sh 'echo rules'
out="$(run_suite trd-test)"; rc=$?
if [ "$rc" -eq 0 ] && contains "$out" "test stub_cargo_case ... ok" \
   && grep -q '^cargo test --workspace --locked$' "$calls" \
   && ! contains "$out" "not_created_yet"; then
  pass "suite.sh trd-test runs cargo test when the component exists"
else
  fail "suite.sh trd-test runs cargo test when the component exists" "rc=$rc $out"
fi

out="$(run_suite trd-check)"; rc=$?
if [ "$rc" -eq 0 ] && contains "$out" "test trd_fmt ... ok" && contains "$out" "test trd_clippy ... ok" \
   && contains "$out" "test trd_system_test_rules_selftest ... ok" \
   && contains "$out" "test trd_system_test_rules ... ok" \
   && grep -q '^cargo fmt --all --check$' "$calls" \
   && grep -q '^cargo clippy --workspace --all-targets --locked -- -D warnings$' "$calls"; then
  pass "suite.sh trd-check runs the four checks when the component exists"
else
  fail "suite.sh trd-check runs the four checks when the component exists" "rc=$rc $out"
fi

out="$(run_suite trd-check STUB_FAIL=clippy)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "test trd_clippy ... FAILED" && contains "$out" "test trd_fmt ... ok"; then
  pass "suite.sh trd-check reports a failing check and exits 1"
else
  fail "suite.sh trd-check reports a failing check and exits 1" "rc=$rc $out"
fi

stub_script trd/scripts/ci-media.sh 'echo "e2e TRD_E2E=${TRD_E2E:-unset}"'
out="$(run_suite trd-e2e)"; rc=$?
if [ "$rc" -eq 0 ] && contains "$out" "e2e TRD_E2E=1"; then
  pass "suite.sh trd-e2e runs ci-media.sh with TRD_E2E=1 when the component exists"
else
  fail "suite.sh trd-e2e runs ci-media.sh with TRD_E2E=1 when the component exists" "rc=$rc $out"
fi

case "$(uname -s):$(uname -m)" in
  Darwin:x86_64) native=macos-x64 ;;
  Darwin:arm64) native=macos-arm64 ;;
  Linux:x86_64|Linux:amd64) native=linux-x64 ;;
  Linux:aarch64|Linux:arm64) native=linux-arm64 ;;
  *) native=unknown ;;
esac
stub_script ffmpeg/build.sh 'echo "PASS stub_build"'
stub_script ffmpeg/smoke_test.sh 'echo "PASS stub_smoke"'
out="$(run_suite ffmpeg-smoke)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q "^build.sh $native\$" "$calls" \
   && grep -q "^smoke_test.sh --target $native $t/ffmpeg/dist/$native\$" "$calls" \
   && contains "$out" "PASS stub_smoke"; then
  pass "suite.sh ffmpeg-smoke builds and tests the native target ($native)"
else
  fail "suite.sh ffmpeg-smoke builds and tests the native target ($native)" "rc=$rc $out $(cat "$calls")"
fi

stub_script exiftool/fetch.sh 'echo "PASS stub_fetch"'
stub_script exiftool/smoke_test.sh 'echo "PASS stub_exiftool_smoke"'
out="$(run_suite exiftool-smoke)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q "^fetch.sh unix $t/exiftool/dist/unix\$" "$calls" \
   && grep -q "^smoke_test.sh $t/exiftool/dist/unix\$" "$calls"; then
  pass "suite.sh exiftool-smoke fetches and tests when the component exists"
else
  fail "suite.sh exiftool-smoke fetches and tests when the component exists" "rc=$rc $out $(cat "$calls")"
fi

stub_script clients/kotlin/scripts/mason-desktop.sh 'echo "desktop stub ran"'
stub_script clients/kotlin/scripts/mason-android.sh 'echo "android stub ran"'
out="$(run_suite kotlin-test)"; rc=$?
if [ "$rc" -eq 0 ] && contains "$out" "desktop stub ran" && [ ! -f "$t/clients/kotlin/build/mason-desktop-results.xml" ]; then
  pass "suite.sh kotlin-test runs mason-desktop.sh when the component exists"
else
  fail "suite.sh kotlin-test runs mason-desktop.sh when the component exists" "rc=$rc $out"
fi
out="$(run_suite android-test)"; rc=$?
expect "suite.sh android-test runs mason-android.sh when the component exists" \
  "$([ "$rc" -eq 0 ] && contains "$out" "android stub ran" && echo 0 || echo 1)" "rc=$rc $out"

mkdir -p "$t/ios/TRMKit"
touch "$t/ios/TRMKit/Package.swift"
out="$(run_suite ios-kit-test)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q "^swift test --package-path $t/ios/TRMKit\$" "$calls"; then
  pass "suite.sh ios-kit-test runs swift test when the component exists"
else
  fail "suite.sh ios-kit-test runs swift test when the component exists" "rc=$rc $out $(cat "$calls")"
fi

stub_script ios/scripts/mason-build.sh 'echo "test ios_build ... ok"'
out="$(run_suite ios-build)"; rc=$?
expect "suite.sh ios-build runs mason-build.sh when the component exists" \
  "$([ "$rc" -eq 0 ] && contains "$out" "test ios_build ... ok" && echo 0 || echo 1)" "rc=$rc $out"

# ---------------------------------------------------------------- bump-version.sh

new_tree; t="$T"
mkdir -p "$t/trd" "$t/ios"
echo "3.0.0" > "$t/VERSION"
cat > "$t/trd/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/trd"]

[workspace.package]
version = "3.0.0"
edition = "2021"

[package]
name = "decoy"
version = "0.0.1"
EOF
cat > "$t/ios/project.yml" <<'EOF'
name: TRM
settings:
  base:
    MARKETING_VERSION: "3.0.0"
    CURRENT_PROJECT_VERSION: 1
EOF
out="$(V3_ROOT="$t" bash "$here/bump-version.sh" 3.1.0-beta.2 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] \
   && [ "$(cat "$t/VERSION")" = "3.1.0-beta.2" ] \
   && grep -q '^version = "3.1.0-beta.2"$' "$t/trd/Cargo.toml" \
   && grep -q '^version = "0.0.1"$' "$t/trd/Cargo.toml" \
   && grep -q '^    MARKETING_VERSION: "3.1.0"$' "$t/ios/project.yml" \
   && grep -q 'CURRENT_PROJECT_VERSION: 1' "$t/ios/project.yml" \
   && contains "$out" "git tag v3.1.0-beta.2"; then
  pass "bump-version.sh 3.1.0-beta.2 writes VERSION, the Cargo version and MARKETING_VERSION 3.1.0"
else
  fail "bump-version.sh 3.1.0-beta.2 writes VERSION, the Cargo version and MARKETING_VERSION 3.1.0" "rc=$rc $out"
fi

new_tree; t="$T"
echo "3.0.0" > "$t/VERSION"
out="$(V3_ROOT="$t" bash "$here/bump-version.sh" 4.0.0 2>&1)"; rc=$?
expect "bump-version.sh writes only VERSION when the other files do not exist" \
  "$([ "$rc" -eq 0 ] && [ "$(cat "$t/VERSION")" = "4.0.0" ] && [ ! -e "$t/trd" ] && [ ! -e "$t/ios" ] && echo 0 || echo 1)" "rc=$rc $out"

out="$(V3_ROOT="$t" bash "$here/bump-version.sh" not-a-version 2>&1)"; rc=$?
expect "bump-version.sh rejects a malformed version" \
  "$([ "$rc" -eq 2 ] && [ "$(cat "$t/VERSION")" = "4.0.0" ] && echo 0 || echo 1)" "rc=$rc $out"

# ---------------------------------------------------------------- check-makefiles.sh

new_tree; t="$T"
cp "$fixtures/Makefile.complete" "$t/Makefile"
mkdir -p "$t/trd"
cp "$fixtures/Makefile.no-develop" "$t/trd/Makefile"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "v3/trd: missing or failing target 'develop'" \
   && ! contains "$out" "v3:"; then
  pass "check-makefiles.sh fails for a stub Makefile missing develop"
else
  fail "check-makefiles.sh fails for a stub Makefile missing develop" "rc=$rc $out"
fi

cp "$fixtures/Makefile.complete" "$t/trd/Makefile"
mkdir -p "$t/clients/kotlin/android"
cp "$fixtures/Makefile.complete" "$t/clients/kotlin/Makefile"
cp "$fixtures/Makefile.complete" "$t/clients/kotlin/android/Makefile"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
expect "check-makefiles.sh passes for complete Makefiles" "$rc" "rc=$rc $out"

mkdir -p "$t/ios"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
expect "check-makefiles.sh fails for a component directory without a Makefile" \
  "$([ "$rc" -eq 1 ] && contains "$out" "v3/ios: no Makefile" && echo 0 || echo 1)" "rc=$rc $out"

new_tree; t="$T"
cp "$fixtures/Makefile.complete" "$t/Makefile"
mkdir -p "$t/trd" "$t/clients/kotlin/build"
cp "$fixtures/Makefile.complete" "$t/trd/Makefile"
: > "$t/clients/kotlin/build/mason-desktop-results.xml"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
expect "check-makefiles.sh passes when a component holds only build/ output" "$rc" "rc=$rc $out"

new_tree; t="$T"
cp "$fixtures/Makefile.complete" "$t/Makefile"
mkdir -p "$t/trd" "$t/clients/kotlin/build"
cp "$fixtures/Makefile.complete" "$t/trd/Makefile"
: > "$t/clients/kotlin/build/mason-desktop-results.xml"
: > "$t/clients/kotlin/notes.txt"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "v3/clients/kotlin: no Makefile"; then
  pass "check-makefiles.sh fails for build/ plus a stray file and no Makefile"
else
  fail "check-makefiles.sh fails for build/ plus a stray file and no Makefile" "rc=$rc $out"
fi

new_tree; t="$T"
cp "$fixtures/Makefile.complete" "$t/Makefile"
mkdir -p "$t/trd" "$t/clients/kotlin/.gradle"
cp "$fixtures/Makefile.complete" "$t/trd/Makefile"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && ! contains "$out" "clients/kotlin: no Makefile"; then
  pass "check-makefiles.sh skips a component holding only .gradle/ output"
else
  fail "check-makefiles.sh skips a component holding only .gradle/ output" "rc=$rc $out"
fi

new_tree; t="$T"
cp "$fixtures/Makefile.complete" "$t/Makefile"
mkdir -p "$t/trd" "$t/clients/kotlin/build" "$t/clients/kotlin/src"
cp "$fixtures/Makefile.complete" "$t/trd/Makefile"
: > "$t/clients/kotlin/build/mason-desktop-results.xml"
echo "class Foo" > "$t/clients/kotlin/src/Foo.kt"
out="$(V3_ROOT="$t" bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && contains "$out" "v3/clients/kotlin: no Makefile"; then
  pass "check-makefiles.sh fails for build/ output plus a source file and no Makefile"
else
  fail "check-makefiles.sh fails for build/ output plus a source file and no Makefile" "rc=$rc $out"
fi

out="$(bash "$here/check-makefiles.sh" 2>&1)"; rc=$?
expect "check-makefiles.sh passes for the real v3/Makefile" "$rc" "rc=$rc $out"

# ---------------------------------------------------------------- check-workflows.sh

tools="${V3_TOOLS:-$real_v3/.tools}"
new_tree; t="$T"
cp "$real_v3/../.github/actionlint.yaml" "$work/$counter/.github/actionlint.yaml"
cp "$fixtures/workflow-valid.yml" "$work/$counter/.github/workflows/v3-valid.yml"
out="$(V3_ROOT="$t" V3_TOOLS="$tools" bash "$here/check-workflows.sh" 2>&1)"; rc=$?
expect "check-workflows.sh passes a valid workflow" "$rc" "rc=$rc $out"

cp "$fixtures/workflow-broken.yml" "$work/$counter/.github/workflows/v3-broken.yml"
out="$(V3_ROOT="$t" V3_TOOLS="$tools" bash "$here/check-workflows.sh" 2>&1)"; rc=$?
if [ "$rc" -ne 0 ] && contains "$out" "v3-broken.yml"; then
  pass "check-workflows.sh fails for a fixture workflow with a syntax error"
else
  fail "check-workflows.sh fails for a fixture workflow with a syntax error" "rc=$rc $out"
fi

# ---------------------------------------------------------------- check-test-names.py

names="python3 -I $here/check-test-names.py"
new_tree; t="$T"
cp "$fixtures/1-fixture.md" "$t/1-fixture.md"
mkdir -p "$t/trd/crates/x/tests" "$t/clients/kotlin/core/src/test/kotlin" "$t/clients/kotlin/core/src/main/kotlin"
cat > "$t/trd/crates/x/tests/present.rs" <<'EOF'
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors
#[test]
fn table_rust_present() {}

// no #[test] attribute: not a test
fn criteria_rust_absent() {}
EOF
cat > "$t/trd/crates/x/tests/criteria_file.rs" <<'EOF'
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors
EOF
cat > "$t/clients/kotlin/core/src/test/kotlin/CriteriaClassTests.kt" <<'EOF'
class CriteriaClassTests {}
EOF
# the table's class exists only outside a test directory: still missing
cat > "$t/clients/kotlin/core/src/main/kotlin/TableClassTests.kt" <<'EOF'
class TableClassTests {}
EOF

out="$(V3_ROOT="$t" $names --spec 1 2>&1)"; rc=$?
want="MISSING TRD-01: criteria_rust_absent (rust test)
MISSING test plan 1: TableClassTests (test class)
MISSING test plan 1: tests/table_file.rs (file)"
if [ "$rc" -eq 1 ] && [ "$out" = "$want" ]; then
  pass "check-test-names prints exactly the three missing names of the fixture spec"
else
  fail "check-test-names prints exactly the three missing names of the fixture spec" "rc=$rc $out"
fi

out="$(V3_ROOT="$t" $names --ticket TRD-01 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && [ "$out" = "MISSING TRD-01: criteria_rust_absent (rust test)" ]; then
  pass "check-test-names --ticket TRD-01 prints only the Rust test's line"
else
  fail "check-test-names --ticket TRD-01 prints only the Rust test's line" "rc=$rc $out"
fi

out="$(V3_ROOT="$t" $names --ticket NOPE-99 2>&1)"; rc=$?
expect "check-test-names rejects an unknown ticket" "$([ "$rc" -eq 2 ] && echo 0 || echo 1)" "rc=$rc $out"

out="$(V3_ROOT="$t" $names 2>&1)"; rc=$?
expect "check-test-names with nothing selected prints TEST-NAMES-OK 0" \
  "$([ "$rc" -eq 0 ] && [ "$out" = "TEST-NAMES-OK 0" ] && echo 0 || echo 1)" "rc=$rc $out"

out="$(V3_ROOT="$t" $names --completed 2>&1)"; rc=$?
expect "check-test-names --completed prints TEST-NAMES-OK 0 with no marker files" \
  "$([ "$rc" -eq 0 ] && [ "$out" = "TEST-NAMES-OK 0" ] && echo 0 || echo 1)" "rc=$rc $out"

# Add the three missing names.
cat >> "$t/trd/crates/x/tests/present.rs" <<'EOF'

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn criteria_rust_absent() {}
EOF
mkdir -p "$t/ios/TRMKit/Tests/TRMCoreTests"
cat > "$t/ios/TRMKit/Tests/TRMCoreTests/TableClassTests.swift" <<'EOF'
final class TableClassTests {}
EOF
cp "$t/trd/crates/x/tests/criteria_file.rs" "$t/trd/crates/x/tests/table_file.rs"
out="$(V3_ROOT="$t" $names --spec 1 2>&1)"; rc=$?
expect "check-test-names prints TEST-NAMES-OK 6 once the three are added" \
  "$([ "$rc" -eq 0 ] && [ "$out" = "TEST-NAMES-OK 6" ] && echo 0 || echo 1)" "rc=$rc $out"

# A completed spec is recognised by the file its final ticket creates.
mkdir -p "$work/$counter/.github/workflows"
mkdir -p "$t/trd/scripts"
touch "$t/trd/scripts/ci-media.sh"
out="$(V3_ROOT="$t" $names --completed 2>&1)"; rc=$?
expect "check-test-names --completed checks a spec once its marker file exists" \
  "$([ "$rc" -eq 0 ] && [ "$out" = "TEST-NAMES-OK 6" ] && echo 0 || echo 1)" "rc=$rc $out"

# A build directory listed in v3/.gitignore is not searched.
rm -f "$t/trd/crates/x/tests/table_file.rs"
mkdir -p "$t/trd/target/debug"
cp "$t/trd/crates/x/tests/criteria_file.rs" "$t/trd/target/debug/table_file.rs"
printf 'trd/target/\n' > "$t/.gitignore"
out="$(V3_ROOT="$t" $names --spec 1 2>&1)"; rc=$?
expect "check-test-names ignores build directories listed in v3/.gitignore" \
  "$([ "$rc" -eq 1 ] && [ "$out" = "MISSING test plan 1: tests/table_file.rs (file)" ] && echo 0 || echo 1)" "rc=$rc $out"

# The real tree: --completed passes on the bootstrap tree.
out="$($names --completed 2>&1)"; rc=$?
expect "check-test-names --completed passes on the real tree" "$rc" "rc=$rc $out"

echo "# $passes passed, $failures failed" >&2
[ "$failures" -eq 0 ]
