#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1300: the SPDX headers are present, the Kotlin tests pass and the DSK-11
# test names exist.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/suite.sh kotlin-test
python3 -I v3/scripts/check-test-names.py --ticket DSK-11
