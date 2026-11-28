#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# What the trd job of v3-gate.yml runs on macOS, Linux and Windows (spec 1 §9.1). It runs from
# the trd workspace, whatever the current directory is, and needs no make, so Git Bash on
# Windows can run it.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

rustup show
cargo fmt --all --check
# Clippy and the test build go through a wrapper that cleans and retries once when a build script
# binary cannot be executed (docs/decisions/1307-trd-build-script-recovery.md).
bash scripts/test-recover-build-script.sh
bash scripts/recover-build-script.sh cargo clippy --workspace --all-targets --locked -- -D warnings
bash scripts/test-check-system-tests.sh
bash scripts/check-system-tests.sh
bash scripts/test-wait-for-trd.sh
bash scripts/recover-build-script.sh cargo test --workspace --locked --no-run
cargo test --workspace --locked
if [ "$(uname -s)" = Darwin ]; then
  # The macOS installer (TRD-24): the self-check assembles a bundle and a .pkg from stubs, and the
  # install scripts are tested with stub launchctl, stat and id.
  bash packaging/macos/build-pkg.sh --self-check
  bash packaging/macos/test-scripts.sh
fi
