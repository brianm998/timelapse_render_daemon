#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1207: the release-version tests pass, the v3 workflows pass actionlint and
# the SPDX check, and the bootstrap script tests pass (they include test-release-version.sh). The
# dispatch run of v3-release.yml (TRD-27 A1, A2) needs GitHub and is not run here.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/check-workflows.sh
bash v3/scripts/test-release-version.sh
bash v3/scripts/test-scripts.sh
