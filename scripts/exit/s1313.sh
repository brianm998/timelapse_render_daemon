#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1313: the SPDX headers are present and the Kotlin tests pass.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/suite.sh kotlin-test
