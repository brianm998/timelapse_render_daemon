#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1141: the .mason/** impact entry no longer names t0-mason, and the
# workflow, SPDX and script suites pass locally. t0-mason is not run here (owner's Mac only).
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/suite.sh t0-workflows
bash v3/scripts/suite.sh t0-spdx
bash v3/scripts/suite.sh t0-scripts
if grep -F '".mason/**"' .mason/mason.toml | grep -q 't0-mason'; then
  echo "s1141: .mason/mason.toml still names t0-mason in the .mason/** impact entry" >&2
  exit 1
fi
if grep -F '".mason/**"' v3/1-trd-core-daemon.md | grep -q 't0-mason'; then
  echo "s1141: v3/1-trd-core-daemon.md still names t0-mason in the .mason/** impact entry" >&2
  exit 1
fi
echo "s1141: .mason/** impact entry no longer names t0-mason"
