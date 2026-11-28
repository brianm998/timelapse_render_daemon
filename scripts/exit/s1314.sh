#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1314 (task 1346): SPDX headers, the daemon's system-test rules, the test of
# the e2e render gate, and the trd-e2e suite (which skips the real-tool path, with a notice, on a
# machine without ffmpeg or exiftool).
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/trd/scripts/check-system-tests.sh
(cd v3/trd && cargo test -p trd --test e2e --locked e2e_gate)
bash v3/scripts/suite.sh trd-e2e
