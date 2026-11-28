#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1304: the SPDX headers are present, DSK-10's named tests exist, and the
# desktop client's tests pass.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
python3 -I v3/scripts/check-test-names.py --ticket DSK-10
bash v3/scripts/suite.sh kotlin-test
