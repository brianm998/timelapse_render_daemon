#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1210: the new and changed files carry the SPDX header, the v3 workflows pass
# actionlint, and the wait-for-trd.sh stub test passes. The system-test dispatches (TRD-29 A1, A2)
# need GitHub and are not run here.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/check-workflows.sh
bash v3/trd/scripts/test-wait-for-trd.sh
