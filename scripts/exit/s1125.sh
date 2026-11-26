#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1125: the trd service tests pass locally, and the system-test rules hold.
set -euo pipefail
cd "$(dirname "$0")/../.."
cd v3/trd
cargo test -p trd service -- --nocapture
bash scripts/check-system-tests.sh
