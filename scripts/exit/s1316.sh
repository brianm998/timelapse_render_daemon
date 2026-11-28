#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1316 (task 1349): SPDX headers, the workflows, spec 2's test names and the
# script tests, which check that the Linux self-test creates the system menu directory before it
# installs the .deb.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/check-workflows.sh
python3 -I v3/scripts/check-test-names.py --spec 2
bash v3/scripts/test-scripts.sh
