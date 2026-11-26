#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1146: the bootstrap script tests pass, including the t0-diff-check cases.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/test-scripts.sh
