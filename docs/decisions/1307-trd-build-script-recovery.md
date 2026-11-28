# 1307: the trd job recovers from an unexecutable build script

## Decision
`v3/trd/scripts/ci.sh` runs clippy and a new `cargo test --workspace --locked --no-run` step
through `scripts/recover-build-script.sh`. When such a command fails with both "failed to run
custom build command" and "cannot execute binary file", the wrapper prints diagnostics of the bad
file, runs `cargo clean -p <name>@<version>` and retries once. Other failures keep their status.
The test run itself stays unwrapped, so test failures are never retried.

## Evidence
Batch #926 (macos-15 trd job): "failed to run custom build command for `trd-proto v3.0.0` ...
target/debug/build/trd-proto-ed68b4b0d46075b8/build-script-build: cannot execute binary file (exit
status: 126)". Clippy had built trd-proto earlier in the same job.

## Confirmed
- The failure text above. #1344 and #1346 touch no build input.
- Nothing between clippy and `cargo test` in ci.sh writes to `target/`; the script tests work in
  temp dirs and `check-system-tests.sh` only reads.

## Inferred, not reproduced
- The build-script file was not a valid executable (corrupt or partial) when cargo ran it; macOS
  hands such a file to /bin/sh, which gives the shell wording and exit 126. A bad rust-cache
  restore is possible but unproven. The cause is unconfirmed.

## Left alone
The rust-cache setup: no evidence implicates it. Scope: only the trd job via ci.sh, not trd-media,
coverage or contract.

## MASON
MASON's code was not inspected. This repo's `.mason/landing.toml` has only the on_red ladder. With
in-job recovery the transient case needs no classification; a failure that survives a clean
rebuild should stay red. Revisit if the `::warning::` line appears repeatedly.
