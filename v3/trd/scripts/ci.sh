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
cargo clippy --workspace --all-targets --locked -- -D warnings
bash scripts/test-check-system-tests.sh
bash scripts/check-system-tests.sh
cargo test --workspace --locked
