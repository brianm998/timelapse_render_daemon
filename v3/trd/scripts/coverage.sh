#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Coverage gate of the trd workspace (overview §15.3): runs the coverage checker's own tests, then
# measures line coverage of trd-core and trd with cargo-llvm-cov and fails when a crate is below
# its floor in coverage-floors.toml. Run from anywhere; `make coverage` runs it.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 -I scripts/test_coverage.py
rustup component add llvm-tools-preview
cargo llvm-cov --workspace --locked --json --output-path target/llvm-cov.json
python3 -I scripts/coverage.py target/llvm-cov.json coverage-floors.toml
