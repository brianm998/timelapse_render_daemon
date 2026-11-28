#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1260: the SPDX headers are present, the tests DSK-03 names exist, and the
# Kotlin suite (`:proto`, `:core`, `:desktop`) passes.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
python3 -I v3/scripts/check-test-names.py --ticket DSK-03
bash v3/scripts/suite.sh kotlin-test
